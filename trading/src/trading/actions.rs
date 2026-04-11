use crate::constants::{ONE_HOUR_SECONDS, SCALAR_7};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{ApplyFunding, ClosePosition, ModifyCollateral, OpenMarket, PlaceLimit, RefundPosition, SetTriggers};
use crate::storage;
use crate::trading::context::Context;
use crate::trading::position::Position;
use crate::dependencies::PriceData;
use crate::validation::{require_active, require_can_manage};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, Address, Env};

/// Create a pending limit order. Validates parameters, stores position, transfers collateral.
///
/// The order is not filled immediately, a keeper calls `execute` with the position ID
/// when the market price reaches `entry_price`.
#[allow(clippy::too_many_arguments)]
pub fn execute_create_limit(
    e: &Env,
    user: &Address,
    market_id: u32,
    collateral: i128,
    notional_size: i128,
    is_long: bool,
    entry_price: i128,
    take_profit: i128,
    stop_loss: i128,
) -> u32 {
    require_active(e);
    user.require_auth();

    let config = storage::get_config(e);
    let market_config = storage::get_market_config(e, market_id);
    let (id, position) = Position::create(e, user, market_id, is_long, entry_price, collateral, notional_size, stop_loss, take_profit);
    position.validate(e, market_config.enabled, config.min_notional, config.max_notional, market_config.margin);
    storage::set_position(e, user, id, &position);

    let token_client = TokenClient::new(e, &storage::get_token(e));
    token_client.transfer(user, e.current_contract_address(), &collateral);

    PlaceLimit {
        market_id,
        user: user.clone(),
        position_id: id,
    }
    .publish(e);

    id
}

/// Cancel a position and refund collateral. No settlement or fees applied.
///
/// - **Pending** (not filled): requires user auth, cancels the limit order.
/// - **Filled + market deleted**: permissionless (anyone can clean up stranded positions).
/// - **Filled + market exists**: panics (use `close_position` for settlement).
pub fn execute_cancel_position(e: &Env, user: &Address, seq: u32) -> i128 {
    require_can_manage(e);
    let position = storage::get_position(e, user, seq);

    if position.filled {
        // Filled positions can only be cancelled if the market was deleted
        if storage::has_market(e, position.market_id) {
            panic_with_error!(e, TradingError::PositionNotPending);
        }
        // Permissionless: anyone can clean up stranded positions on deleted markets
    } else {
        position.user.require_auth();
    }

    let payout = position.col;
    if payout > 0 {
        let token_client = TokenClient::new(e, &storage::get_token(e));
        token_client.transfer(&e.current_contract_address(), &position.user, &payout);
    }

    storage::remove_position(e, user, seq);

    RefundPosition {
        market_id: position.market_id,
        user: position.user.clone(),
        position_id: seq,
        amount: payout,
    }
    .publish(e);

    payout
}

/// Create and immediately fill a market order at the current oracle price.
///
/// Unlike `execute_create_limit`, this fills the position in the same transaction.
/// Open fees (base + impact) are deducted from collateral. The remaining fee
/// portion goes to the vault and treasury.
///
/// `Context::load` verifies that `price_data.feed_id` matches the market's configured feed.
#[allow(clippy::too_many_arguments)]
pub fn execute_create_market(
    e: &Env,
    user: &Address,
    market_id: u32,
    collateral: i128,
    notional_size: i128,
    is_long: bool,
    take_profit: i128,
    stop_loss: i128,
    price_data: &PriceData,
) -> u32 {
    require_active(e);
    user.require_auth();

    let mut ctx = Context::load(e, market_id, price_data);

    let (id, mut position) = Position::create(e, user, market_id, is_long, ctx.price, collateral, notional_size, stop_loss, take_profit);
    let (base_fee, impact_fee) = ctx.open(e, &mut position, user, id);
    ctx.store(e);

    let total_fee = base_fee + impact_fee;
    let treasury_fee = ctx.treasury_fee(e, total_fee);
    let vault_fee = total_fee - treasury_fee;

    let token_client = TokenClient::new(e, &ctx.token);
    token_client.transfer(user, e.current_contract_address(), &collateral);
    if vault_fee > 0 {
        token_client.transfer(&e.current_contract_address(), &ctx.vault, &vault_fee);
    }
    if treasury_fee > 0 {
        token_client.transfer(&e.current_contract_address(), &ctx.treasury, &treasury_fee);
    }

    OpenMarket {
        market_id: ctx.market_id,
        user: user.clone(),
        position_id: id,
        base_fee,
        impact_fee,
    }
    .publish(e);

    id
}

/// Close a filled position at the current oracle price with full settlement.
///
/// Requires a valid price feed. For deleted markets or pending positions,
/// use `cancel_position` instead.
///
/// # Returns
/// User payout amount (token_decimals), >= 0.
pub fn execute_close_position(e: &Env, user: &Address, seq: u32, price: soroban_sdk::Bytes) -> i128 {
    require_can_manage(e);
    let pv = crate::dependencies::PriceVerifierClient::new(e, &storage::get_price_verifier(e));
    let price_data = pv.verify_price(&price);

    let mut position = storage::get_position(e, user, seq);
    position.user.require_auth();
    position.require_closable(e);

    let mut ctx = Context::load(e, position.market_id, &price_data);
    let col = position.col;
    let s = ctx.close(e, &mut position, user, seq);

    let user_payout = s.equity(col).max(0);
    let treasury_fee = ctx.treasury_fee(e, s.protocol_fee());
    let vault_transfer = col - user_payout - treasury_fee;

    let token_client = TokenClient::new(e, &ctx.token);
    if vault_transfer < 0 {
        VaultClient::new(e, &ctx.vault)
            .strategy_withdraw(&e.current_contract_address(), &(-vault_transfer));
    } else if vault_transfer > 0 {
        token_client.transfer(&e.current_contract_address(), &ctx.vault, &vault_transfer);
    }
    if treasury_fee > 0 {
        token_client.transfer(&e.current_contract_address(), &ctx.treasury, &treasury_fee);
    }
    if user_payout > 0 {
        token_client.transfer(&e.current_contract_address(), &position.user, &user_payout);
    }

    ctx.store(e);

    ClosePosition {
        market_id: position.market_id,
        user: position.user.clone(),
        position_id: seq,
        price: ctx.price,
        pnl: s.net_pnl(col),
        base_fee: s.base_fee,
        impact_fee: s.impact_fee,
        funding: s.funding,
        borrowing_fee: s.borrowing_fee,
    }
    .publish(e);

    user_payout
}

/// Add or withdraw collateral on an open (filled) position.
///
/// For withdrawals, a margin check is performed: the position's equity after
/// settlement must remain above `notional * margin`. This prevents users from
/// extracting collateral to a point where the position would be immediately liquidatable.
pub fn execute_modify_collateral(e: &Env, user: &Address, seq: u32, new_collateral: i128, price_data: &PriceData) {
    require_can_manage(e);
    let mut position = storage::get_position(e, user, seq);
    position.user.require_auth();

    if !position.filled {
        panic_with_error!(e, TradingError::ActionNotAllowedForStatus);
    }

    let collateral_diff = new_collateral - position.col;
    if collateral_diff == 0 {
        panic_with_error!(e, TradingError::CollateralUnchanged);
    }
    position.col = new_collateral;

    if collateral_diff > 0 {
        let token_client = TokenClient::new(e, &storage::get_token(e));
        token_client.transfer(&position.user, e.current_contract_address(), &collateral_diff);
    } else {
        let ctx = Context::load(e, position.market_id, price_data);
        let token_client = TokenClient::new(e, &ctx.token);
        let s = position.settle(e, &ctx);
        let equity = position.col + s.pnl - s.total_fee();
        if equity < position.notional.fixed_mul_ceil(e, &ctx.config.margin, &SCALAR_7) {
            panic_with_error!(e, TradingError::WithdrawalBreaksMargin);
        }

        ctx.store(e);
        token_client.transfer(&e.current_contract_address(), &position.user, &-collateral_diff);
    }

    storage::set_position(e, user, seq, &position);
    ModifyCollateral {
        market_id: position.market_id,
        user: position.user.clone(),
        position_id: seq,
        amount: collateral_diff,
    }
    .publish(e);
}

/// Update take-profit and stop-loss trigger prices on a position.
///
/// Set to 0 to clear a trigger. TP/SL are pure price triggers — no
/// entry-price validation. Invalid values simply never fire.
pub fn execute_set_triggers(e: &Env, user: &Address, seq: u32, take_profit: i128, stop_loss: i128) {
    require_can_manage(e);
    let mut position = storage::get_position(e, user, seq);
    position.user.require_auth();

    position.tp = take_profit;
    position.sl = stop_loss;
    storage::set_position(e, user, seq, &position);

    SetTriggers {
        market_id: position.market_id,
        user: position.user.clone(),
        position_id: seq,
        take_profit,
        stop_loss,
    }
    .publish(e);
}

/// Apply funding rate updates across all markets. Permissionless, callable once per hour.
///
/// For each market: accrues borrowing + funding indices, then recalculates the
/// funding rate based on current OI imbalance. The new rate takes effect for the
/// next accrual period.
///
/// # Panics
/// - `TradingError::FundingTooEarly` (752) if < 1 hour since last call
pub fn execute_apply_funding(e: &Env) {
    let last_funding_update = storage::get_last_funding_update(e);
    let elapsed = e.ledger().timestamp() - last_funding_update;
    if elapsed < ONE_HOUR_SECONDS {
        panic_with_error!(e, TradingError::FundingTooEarly);
    }

    let config = storage::get_config(e);
    let markets = storage::get_markets(e);
    let vault_balance = VaultClient::new(e, &storage::get_vault(e)).total_assets();
    let total_notional = storage::get_total_notional(e);

    for market_id in markets.iter() {
        let market_config = storage::get_market_config(e, market_id);
        let mut data = storage::get_market_data(e, market_id);

        data.accrue(
            e,
            config.r_base,
            config.r_var,
            market_config.r_var_market,
            vault_balance,
            total_notional,
            config.max_util,
            market_config.max_util,
        );
        data.update_funding_rate(e, config.r_funding);

        storage::set_market_data(e, market_id, &data);
    }

    (ApplyFunding {}).publish(e);

    storage::set_last_funding_update(e, e.ledger().timestamp());
}


#[cfg(test)]
mod tests {
    use crate::constants::SCALAR_7;
    use crate::storage;
    use crate::testutils::{
        setup_contract, setup_env, FEED_BTC, BTC_PRICE,
    };
    use crate::dependencies::PriceData;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Bytes};

    /// Dummy price bytes — mock price verifier ignores content,
    /// returns stored price instead.
    fn dummy_price_bytes(e: &soroban_sdk::Env) -> Bytes {
        Bytes::new(e)
    }

    /// Helper: create a pending long limit order
    fn place_limit_long(e: &soroban_sdk::Env, contract: &Address, user: &Address, collateral: i128, notional: i128) -> u32 {
        e.as_contract(contract, || {
            super::execute_create_limit(
                e,
                user,
                FEED_BTC,
                collateral,
                notional,
                true,
                BTC_PRICE,
                0, 0,
            )
        })
    }

    fn place_limit_short(e: &soroban_sdk::Env, contract: &Address, user: &Address, collateral: i128, notional: i128) -> u32 {
        e.as_contract(contract, || {
            super::execute_create_limit(
                e,
                user,
                FEED_BTC,
                collateral,
                notional,
                false,
                BTC_PRICE,
                0, 0,
            )
        })
    }

    #[test]
    fn test_create_limit_long() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let collateral = 1_000 * SCALAR_7;
        let notional = 10_000 * SCALAR_7;
        let id = place_limit_long(&e, &contract, &user, collateral, notional);

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, &user, id);
            assert_eq!(pos.col, collateral);
            assert_eq!(pos.notional, notional);
            assert!(pos.long);
            assert!(!pos.filled);
            assert_eq!(pos.entry_price, BTC_PRICE);

            let counter = storage::get_user_counter(&e, &user);
            assert_eq!(counter, 1);
        });
    }

    #[test]
    fn test_create_limit_short() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = place_limit_short(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, &user, id);
            assert!(!pos.long);
            assert!(!pos.filled);
        });
    }

    #[test]
    fn test_create_market_long() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let collateral = 1_000 * SCALAR_7;
        let notional = 10_000 * SCALAR_7;

        let price_data = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, collateral, notional, true, 0, 0, &price_data,
            )
        });

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, &user, id);
            assert!(pos.col < collateral); // collateral reduced by open fees
            assert_eq!(pos.notional, notional);
            assert!(pos.long);
            assert!(pos.filled); // market order is filled immediately
            assert_eq!(pos.entry_price, BTC_PRICE);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #723)")]
    fn test_create_limit_zero_collateral() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        place_limit_long(&e, &contract, &user, 0, 10_000 * SCALAR_7);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #724)")]
    fn test_create_limit_below_min_notional() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        // min_notional = 10 * SCALAR_7, try with 5
        place_limit_long(&e, &contract, &user, SCALAR_7, 5 * SCALAR_7);
    }

    #[test]
    fn test_apply_funding_rate() {
        use crate::testutils::jump;

        let e = setup_env();
        let (contract, _token_client) = setup_contract(&e);

        jump(&e, 1000 + 3601);

        e.as_contract(&contract, || {
            super::execute_apply_funding(&e);
            let last = storage::get_last_funding_update(&e);
            assert_eq!(last, 1000 + 3601);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #752)")]
    fn test_apply_funding_too_early() {
        use crate::testutils::jump;

        let e = setup_env();
        let (contract, _token_client) = setup_contract(&e);

        jump(&e, 1000 + 1800);

        e.as_contract(&contract, || {
            super::execute_apply_funding(&e);
        });
    }

    #[test]
    fn test_cancel_position() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let balance_before = token_client.balance(&user);
        let id = place_limit_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);

        e.as_contract(&contract, || {
            super::execute_cancel_position(&e, &user, id);
        });

        // User gets full collateral back (no fees charged for limits)
        let balance_after = token_client.balance(&user);
        assert_eq!(balance_after, balance_before);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #721)")]
    fn test_cancel_position_filled_panics() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        // Create a market order (immediately filled)
        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        e.as_contract(&contract, || {
            super::execute_cancel_position(&e, &user, id);
        });
    }

    #[test]
    fn test_close_position() {
        use crate::testutils::jump;
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        jump(&e, 1000 + 31);

        let balance_before = token_client.balance(&user);
        e.as_contract(&contract, || {
            let payout = super::execute_close_position(&e, &user, id, dummy_price_bytes(&e));
            assert!(payout > 0);
        });

        let balance_after = token_client.balance(&user);
        assert!(balance_after > balance_before);
    }

    #[test]
    fn test_modify_collateral_add() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let collateral = 1_000 * SCALAR_7;
        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, collateral, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        let new_collateral = 2_000 * SCALAR_7;
        e.as_contract(&contract, || {
            super::execute_modify_collateral(&e, &user, id, new_collateral, &pd);
            let pos = storage::get_position(&e, &user, id);
            assert_eq!(pos.col, new_collateral);
        });
    }

    #[test]
    fn test_modify_collateral_withdraw() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let collateral = 5_000 * SCALAR_7;
        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, collateral, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, &user, id);
            // Withdraw a small amount — must stay above margin
            let new_collateral = pos.col - 100 * SCALAR_7;
            super::execute_modify_collateral(&e, &user, id, new_collateral, &pd);
            let pos = storage::get_position(&e, &user, id);
            assert_eq!(pos.col, new_collateral);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #727)")]
    fn test_modify_collateral_unchanged_panics() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, &user, id);
            super::execute_modify_collateral(&e, &user, id, pos.col, &pd);
        });
    }

    #[test]
    fn test_set_triggers() {
        use crate::testutils::PRICE_SCALAR;
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        let tp = 110_000 * PRICE_SCALAR;
        let sl = 95_000 * PRICE_SCALAR;
        e.as_contract(&contract, || {
            super::execute_set_triggers(&e, &user, id, tp, sl);
            let pos = storage::get_position(&e, &user, id);
            assert_eq!(pos.tp, tp);
            assert_eq!(pos.sl, sl);
        });
    }

    #[test]
    fn test_set_triggers_clear() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true,
                110_000 * 100_000_000, 95_000 * 100_000_000, &pd,
            )
        });

        // Clear both triggers by setting to 0
        e.as_contract(&contract, || {
            super::execute_set_triggers(&e, &user, id, 0, 0);
            let pos = storage::get_position(&e, &user, id);
            assert_eq!(pos.tp, 0);
            assert_eq!(pos.sl, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #702)")]
    fn test_create_limit_disabled() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        e.as_contract(&contract, || {
            let mut mc = storage::get_market_config(&e, FEED_BTC);
            mc.enabled = false;
            storage::set_market_config(&e, FEED_BTC, &mc);
        });

        place_limit_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #702)")]
    fn test_create_market_disabled() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        e.as_contract(&contract, || {
            let mut mc = storage::get_market_config(&e, FEED_BTC);
            mc.enabled = false;
            storage::set_market_config(&e, FEED_BTC, &mc);
        });

        let pd = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            );
        });
    }

    #[test]
    fn test_close_position_disabled_settles_normally() {
        use crate::testutils::jump;
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        // Open a filled market position
        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        // Disable market
        e.as_contract(&contract, || {
            let mut mc = storage::get_market_config(&e, FEED_BTC);
            mc.enabled = false;
            storage::set_market_config(&e, FEED_BTC, &mc);
        });

        jump(&e, 1000 + 31);

        // Close settles normally (price unchanged → payout = col - fees)
        let balance_before = token_client.balance(&user);
        e.as_contract(&contract, || {
            let payout = super::execute_close_position(&e, &user, id, dummy_price_bytes(&e));
            assert!(payout > 0);
        });

        let balance_after = token_client.balance(&user);
        assert!(balance_after > balance_before);
    }

    #[test]
    fn test_cancel_position_deleted_market_refund() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: FEED_BTC,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        // Create filled position, then delete the market
        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, FEED_BTC, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        let col = e.as_contract(&contract, || {
            storage::get_position(&e, &user, id).col
        });

        e.as_contract(&contract, || {
            crate::trading::execute_del_market(&e, FEED_BTC);
        });

        // cancel_position works for filled positions when market is deleted
        let balance_before = token_client.balance(&user);
        e.as_contract(&contract, || {
            let payout = super::execute_cancel_position(&e, &user, id);
            assert_eq!(payout, col);
        });

        let balance_after = token_client.balance(&user);
        assert_eq!(balance_after - balance_before, col);
    }

    #[test]
    fn test_cancel_position_pending_disabled() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let collateral = 1_000 * SCALAR_7;
        let id = place_limit_long(&e, &contract, &user, collateral, 10_000 * SCALAR_7);

        // Disable market — pending position can still be cancelled
        e.as_contract(&contract, || {
            let mut mc = storage::get_market_config(&e, FEED_BTC);
            mc.enabled = false;
            storage::set_market_config(&e, FEED_BTC, &mc);
        });

        let balance_before = token_client.balance(&user);
        e.as_contract(&contract, || {
            let payout = super::execute_cancel_position(&e, &user, id);
            assert_eq!(payout, collateral);
        });

        let balance_after = token_client.balance(&user);
        assert_eq!(balance_after - balance_before, collateral);
    }

}
