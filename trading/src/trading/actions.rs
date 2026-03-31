use crate::constants::{ONE_HOUR_SECONDS, SCALAR_7};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{ApplyFunding, CancelLimit, ClosePosition, ModifyCollateral, OpenMarket, PlaceLimit, SetTriggers};
use crate::storage;
use crate::trading::context::Context;
use crate::trading::position::Position;
use crate::dependencies::PriceData;
use crate::types::MarketStatus;
use crate::validation::{require_active, require_can_manage};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, Address, Env};

/// Create a pending limit order. Validates parameters, stores position, transfers collateral.
///
/// The order is not filled immediately -- a keeper calls `execute` with the position ID
/// when the market price reaches `entry_price`.
#[allow(clippy::too_many_arguments)]
pub fn execute_create_limit(
    e: &Env,
    user: &Address,
    feed_id: u32,
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
    let market_config = storage::get_market_config(e, feed_id);
    let (id, position) = Position::create(e, user, feed_id, is_long, entry_price, collateral, notional_size, stop_loss, take_profit);
    position.validate(e, market_config.status, config.min_notional, config.max_notional, market_config.margin);
    storage::set_position(e, id, &position);

    let token_client = TokenClient::new(e, &storage::get_token(e));
    token_client.transfer(user, e.current_contract_address(), &collateral);

    PlaceLimit {
        feed_id,
        user: user.clone(),
        position_id: id,
    }
    .publish(e);

    id
}

/// Cancel a pending (unfilled) limit order. Returns collateral to user.
///
/// No fees are charged on cancellation since the order was never filled.
pub fn execute_cancel_limit(e: &Env, position_id: u32) -> i128 {
    require_can_manage(e);
    let position = storage::get_position(e, position_id);
    position.user.require_auth();

    if position.filled {
        panic_with_error!(e, TradingError::PositionNotPending);
    }

    let token_client = TokenClient::new(e, &storage::get_token(e));
    token_client.transfer(&e.current_contract_address(), &position.user, &position.col);

    storage::remove_user_position(e, &position.user, position_id);
    storage::remove_position(e, position_id);

    CancelLimit {
        feed_id: position.feed,
        user: position.user.clone(),
        position_id,
    }
        .publish(e);

    position.col
}

/// Create and immediately fill a market order at the current oracle price.
///
/// Unlike `execute_create_limit`, this fills the position in the same transaction.
/// Open fees (base + impact) are deducted from collateral. The remaining fee
/// portion goes to the vault and treasury.
#[allow(clippy::too_many_arguments)]
pub fn execute_create_market(
    e: &Env,
    user: &Address,
    collateral: i128,
    notional_size: i128,
    is_long: bool,
    take_profit: i128,
    stop_loss: i128,
    price_data: &PriceData,
) -> u32 {
    require_active(e);
    user.require_auth();

    let mut market = Context::load(e, price_data);

    let (id, mut position) = Position::create(e, user, market.feed_id, is_long, market.price, collateral, notional_size, stop_loss, take_profit);
    let (base_fee, impact_fee) = market.open(e, &mut position, id);
    market.store(e);

    let total_fee = base_fee + impact_fee;
    let treasury_fee = market.treasury_fee(e, total_fee);
    let vault_fee = total_fee - treasury_fee;

    let token_client = TokenClient::new(e, &market.token);
    token_client.transfer(user, e.current_contract_address(), &collateral);
    if vault_fee > 0 {
        token_client.transfer(&e.current_contract_address(), &market.vault, &vault_fee);
    }
    if treasury_fee > 0 {
        token_client.transfer(&e.current_contract_address(), &market.treasury, &treasury_fee);
    }

    OpenMarket {
        feed_id: market.feed_id,
        user: user.clone(),
        position_id: id,
        base_fee,
        impact_fee,
    }
    .publish(e);

    id
}

/// Close a filled position at the current oracle price.
///
/// Settles all accrued fees (funding, borrowing) and PnL. User receives
/// `max(equity, 0)`. If the position is underwater, user gets nothing and
/// the vault absorbs the remainder. Treasury receives its fee cut.
///
/// # Returns
/// User payout amount (token_decimals), >= 0.
pub fn execute_close_position(e: &Env, position_id: u32, price_data: &PriceData) -> i128 {
    require_can_manage(e);
    let mut position = storage::get_position(e, position_id);

    // Delisting: anyone can close any position (payout still goes to position.user)
    let market_config = storage::get_market_config(e, position.feed);
    if MarketStatus::from_u32(e, market_config.status) != MarketStatus::Delisting {
        position.user.require_auth();
    }

    position.require_closable(e);
    if price_data.feed_id != position.feed {
        panic_with_error!(e, TradingError::InvalidPrice);
    }

    let mut market = Context::load(e, price_data);
    let col = position.col;
    let s = market.close(e, &mut position, position_id);

    let user_payout = s.equity(col).max(0);
    let treasury_fee = market.treasury_fee(e, s.protocol_fee());
    let vault_transfer = col - user_payout - treasury_fee;

    let token_client = TokenClient::new(e, &market.token);
    if vault_transfer < 0 {
        VaultClient::new(e, &market.vault)
            .strategy_withdraw(&e.current_contract_address(), &(-vault_transfer));
    } else if vault_transfer > 0 {
        token_client.transfer(&e.current_contract_address(), &market.vault, &vault_transfer);
    }
    if treasury_fee > 0 {
        token_client.transfer(&e.current_contract_address(), &market.treasury, &treasury_fee);
    }
    if user_payout > 0 {
        token_client.transfer(&e.current_contract_address(), &position.user, &user_payout);
    }

    market.store(e);

    ClosePosition {
        feed_id: position.feed,
        user: position.user.clone(),
        position_id,
        price: market.price,
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
pub fn execute_modify_collateral(e: &Env, position_id: u32, new_collateral: i128, price_data: &PriceData) {
    require_can_manage(e);
    let mut position = storage::get_position(e, position_id);
    position.user.require_auth();

    // No fees have been charged recreate position instead of modifying collateral
    if !position.filled {
        panic_with_error!(e, TradingError::ActionNotAllowedForStatus);
    }
    if price_data.feed_id != position.feed {
        panic_with_error!(e, TradingError::InvalidPrice);
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
        let market = Context::load(e, price_data);
        let token_client = TokenClient::new(e, &market.token);
        let s = position.settle(e, &market);
        let equity = position.col + s.pnl - s.total_fee();
        if equity < position.notional.fixed_mul_ceil(e, &market.config.margin, &SCALAR_7) {
            panic_with_error!(e, TradingError::WithdrawalBreaksMargin);
        }

        market.store(e);
        token_client.transfer(&e.current_contract_address(), &position.user, &-collateral_diff);
    }

    storage::set_position(e, position_id, &position);
    ModifyCollateral {
        feed_id: position.feed,
        user: position.user.clone(),
        position_id,
        amount: collateral_diff,
    }
    .publish(e);
}

/// Update take-profit and stop-loss trigger prices on a position.
///
/// Set to 0 to clear a trigger. Validates direction constraints (TP must be
/// above entry for longs, below for shorts; SL is the reverse).
pub fn execute_set_triggers(e: &Env, position_id: u32, take_profit: i128, stop_loss: i128) {
    require_can_manage(e);
    let mut position = storage::get_position(e, position_id);
    position.user.require_auth();

    position.tp = take_profit;
    position.sl = stop_loss;
    position.validate_triggers(e);
    storage::set_position(e, position_id, &position);
    
    SetTriggers {
        feed_id: position.feed,
        user: position.user.clone(),
        position_id,
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
/// - `TradingError::FundingTooEarly` (790) if < 1 hour since last call
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

    for feed_id in markets.iter() {
        let market_config = storage::get_market_config(e, feed_id);
        let mut data = storage::get_market_data(e, feed_id);

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

        storage::set_market_data(e, feed_id, &data);
    }

    (ApplyFunding {}).publish(e);

    storage::set_last_funding_update(e, e.ledger().timestamp());
}


#[cfg(test)]
mod tests {
    use crate::constants::SCALAR_7;
    use crate::storage;
    use crate::testutils::{
        setup_contract, setup_env, BTC_FEED_ID, BTC_PRICE,
    };
    use crate::dependencies::PriceData;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;

    /// Helper: create a pending long limit order
    fn place_limit_long(e: &soroban_sdk::Env, contract: &Address, user: &Address, collateral: i128, notional: i128) -> u32 {
        e.as_contract(contract, || {
            super::execute_create_limit(
                e,
                user,
                BTC_FEED_ID,
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
                BTC_FEED_ID,
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
            let pos = storage::get_position(&e, id);
            assert_eq!(pos.col, collateral);
            assert_eq!(pos.notional, notional);
            assert!(pos.long);
            assert!(!pos.filled);
            assert_eq!(pos.entry_price, BTC_PRICE);

            let positions = storage::get_user_positions(&e, &user);
            assert_eq!(positions.len(), 1);
            assert_eq!(positions.get(0).unwrap(), id);
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
            let pos = storage::get_position(&e, id);
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
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, collateral, notional, true, 0, 0, &price_data,
            )
        });

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, id);
            assert!(pos.col < collateral); // collateral reduced by open fees
            assert_eq!(pos.notional, notional);
            assert!(pos.long);
            assert!(pos.filled); // market order is filled immediately
            assert_eq!(pos.entry_price, BTC_PRICE);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #735)")]
    fn test_create_limit_zero_collateral() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        place_limit_long(&e, &contract, &user, 0, 10_000 * SCALAR_7);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #736)")]
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
    #[should_panic(expected = "Error(Contract, #790)")]
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
    fn test_cancel_limit() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let balance_before = token_client.balance(&user);
        let id = place_limit_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);

        e.as_contract(&contract, || {
            super::execute_cancel_limit(&e, id);

            let positions = storage::get_user_positions(&e, &user);
            assert_eq!(positions.len(), 0);
        });

        // User gets full collateral back (no fees charged for limits)
        let balance_after = token_client.balance(&user);
        assert_eq!(balance_after, balance_before);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #733)")]
    fn test_cancel_limit_filled_panics() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        // Create a market order (immediately filled)
        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        e.as_contract(&contract, || {
            super::execute_cancel_limit(&e, id);
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
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        jump(&e, 1000 + 31);

        let balance_before = token_client.balance(&user);
        e.as_contract(&contract, || {
            let payout = super::execute_close_position(&e, id, &pd);
            assert!(payout > 0);

            let positions = storage::get_user_positions(&e, &user);
            assert_eq!(positions.len(), 0);
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
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let collateral = 1_000 * SCALAR_7;
        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, collateral, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        let new_collateral = 2_000 * SCALAR_7;
        e.as_contract(&contract, || {
            super::execute_modify_collateral(&e, id, new_collateral, &pd);
            let pos = storage::get_position(&e, id);
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
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let collateral = 5_000 * SCALAR_7;
        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, collateral, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, id);
            // Withdraw a small amount — must stay above margin
            let new_collateral = pos.col - 100 * SCALAR_7;
            super::execute_modify_collateral(&e, id, new_collateral, &pd);
            let pos = storage::get_position(&e, id);
            assert_eq!(pos.col, new_collateral);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #740)")]
    fn test_modify_collateral_unchanged_panics() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, id);
            super::execute_modify_collateral(&e, id, pos.col, &pd);
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
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        let tp = 110_000 * PRICE_SCALAR;
        let sl = 95_000 * PRICE_SCALAR;
        e.as_contract(&contract, || {
            super::execute_set_triggers(&e, id, tp, sl);
            let pos = storage::get_position(&e, id);
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
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true,
                110_000 * 100_000_000, 95_000 * 100_000_000, &pd,
            )
        });

        // Clear both triggers by setting to 0
        e.as_contract(&contract, || {
            super::execute_set_triggers(&e, id, 0, 0);
            let pos = storage::get_position(&e, id);
            assert_eq!(pos.tp, 0);
            assert_eq!(pos.sl, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #712)")]
    fn test_create_limit_halted() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        // Set market to Halted
        e.as_contract(&contract, || {
            let mut mc = storage::get_market_config(&e, BTC_FEED_ID);
            mc.status = 1; // MarketStatus::Halted
            storage::set_market_config(&e, BTC_FEED_ID, &mc);
        });

        place_limit_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #712)")]
    fn test_create_market_delisting() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        // Set market to Delisting
        e.as_contract(&contract, || {
            let mut mc = storage::get_market_config(&e, BTC_FEED_ID);
            mc.status = 2; // MarketStatus::Delisting
            storage::set_market_config(&e, BTC_FEED_ID, &mc);
        });

        let pd = PriceData {
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            );
        });
    }

    #[test]
    fn test_close_position_delisting_no_auth() {
        use crate::testutils::jump;
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let pd = PriceData {
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        // Open a position
        let id = e.as_contract(&contract, || {
            super::execute_create_market(
                &e, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &pd,
            )
        });

        jump(&e, 1000 + 31);

        // Set market to Delisting
        e.as_contract(&contract, || {
            let mut mc = storage::get_market_config(&e, BTC_FEED_ID);
            mc.status = 2; // MarketStatus::Delisting
            storage::set_market_config(&e, BTC_FEED_ID, &mc);
        });

        // Close without position owner auth -- should succeed in Delisting mode
        let balance_before = token_client.balance(&user);
        e.as_contract(&contract, || {
            let payout = super::execute_close_position(&e, id, &pd);
            assert!(payout > 0);

            let positions = storage::get_user_positions(&e, &user);
            assert_eq!(positions.len(), 0);
        });

        // User still gets paid
        let balance_after = token_client.balance(&user);
        assert!(balance_after > balance_before);
    }

}
