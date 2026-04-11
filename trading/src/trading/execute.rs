use crate::constants::SCALAR_7;
use crate::errors::TradingError;
use crate::events::{FillLimit, Liquidation, StopLoss, TakeProfit};
use crate::storage;
use crate::trading::context::Context;
use crate::trading::position::{Position, Settlement};
use crate::dependencies::PriceData;
use crate::validation::require_can_manage;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

/// Accumulate a transfer amount for an address (batches multiple payouts).
fn add_transfer(map: &mut Map<Address, i128>, address: &Address, amount: i128) {
    map.set(
        address.clone(),
        amount + map.get(address.clone()).unwrap_or(0),
    );
}

/// Execute a batch of keeper triggers for a single market.
///
/// Auto-detects the action for each position:
/// - **Not filled** → fill limit order (if price crossed entry)
/// - **Filled** → priority order: liquidate > stop-loss > take-profit
pub fn execute_trigger(
    e: &Env,
    caller: &Address,
    market_id: u32,
    users: Vec<Address>,
    ids: Vec<u32>,
    price_data: &PriceData,
) {
    require_can_manage(e);

    let mut ctx = Context::load(e, market_id, price_data);
    let transfers = process_positions(e, &mut ctx, caller, users, ids);

    let token_client = TokenClient::new(e, &ctx.token);
    let vault_client = crate::dependencies::VaultClient::new(e, &ctx.vault);

    // STEP 1: Vault pays to contract (if needed)
    let vault_transfer = transfers.get(ctx.vault.clone()).unwrap_or(0);
    if vault_transfer < 0 {
        vault_client.strategy_withdraw(&e.current_contract_address(), &vault_transfer.abs());
    }

    // STEP 2: Handle all other transfers
    for (address, amount) in transfers.iter() {
        if address != ctx.vault && amount > 0 {
            token_client.transfer(&e.current_contract_address(), &address, &amount);
        }
    }

    // STEP 3: Contract pays to vault if needed
    if vault_transfer > 0 {
        token_client.transfer(&e.current_contract_address(), &ctx.vault, &vault_transfer);
    }

    ctx.store(e);
}

fn process_positions(
    e: &Env,
    ctx: &mut Context,
    caller: &Address,
    users: Vec<Address>,
    ids: Vec<u32>,
) -> Map<Address, i128> {
    let mut t: Map<Address, i128> = Map::new(e);

    for i in 0..users.len() {
        let user = users.get(i).unwrap();
        let id = ids.get(i).unwrap();
        let mut position = storage::get_position(e, &user, id);

        if position.market_id != ctx.market_id {
            panic_with_error!(e, TradingError::InvalidPrice);
        }

        if !position.filled {
            apply_fill(e, &mut t, ctx, caller, &mut position, &user, id);
        } else {
            apply_close(e, &mut t, ctx, caller, &mut position, &user, id);
        }
    }

    t
}

/// Close a filled position, auto-detecting the action:
/// liquidate (equity < threshold) > stop-loss > take-profit.
///
/// Liquidation bypasses MIN_OPEN_TIME (only requires fresh price).
/// SL/TP require MIN_OPEN_TIME via require_closable.
fn apply_close(
    e: &Env,
    t: &mut Map<Address, i128>,
    ctx: &mut Context,
    caller: &Address,
    position: &mut Position,
    user: &Address,
    id: u32,
) {
    let col = position.col;
    let s = ctx.close(e, position, user, id);
    let liq_threshold = position.notional.fixed_mul_floor(e, &ctx.config.liq_fee, &SCALAR_7);
    let equity = s.equity(col);

    // Priority 1: Liquidation if under collateralized, regardless of open time or SL/TP
    if equity < liq_threshold {
        position.require_liquidatable(e, ctx.publish_time);
        settle_liquidation(e, t, ctx, caller, position, user, id, col, &s, equity);
    }
    // Priority 2: Stop-loss if trigger price hit, requires open time
    else if position.check_stop_loss(ctx.price) {
        position.require_closable(e);
        settle_close(e, t, ctx, caller, user, col, &s);
        StopLoss {
            market_id: position.market_id,
            user: user.clone(),
            position_id: id,
            price: ctx.price,
            pnl: s.net_pnl(col),
            base_fee: s.base_fee,
            impact_fee: s.impact_fee,
            funding: s.funding,
            borrowing_fee: s.borrowing_fee,
        }
        .publish(e);
    }
    // Priority 3: Take-profit if trigger price hit, requires open time
    else if position.check_take_profit(ctx.price) {
        position.require_closable(e);
        settle_close(e, t, ctx, caller, user, col, &s);
        TakeProfit {
            market_id: position.market_id,
            user: user.clone(),
            position_id: id,
            price: ctx.price,
            pnl: s.net_pnl(col),
            base_fee: s.base_fee,
            impact_fee: s.impact_fee,
            funding: s.funding,
            borrowing_fee: s.borrowing_fee,
        }
        .publish(e);
    } else {
        panic_with_error!(e, TradingError::NotActionable);
    }
}

/// Distribute transfers for a normal close (SL/TP).
fn settle_close(
    e: &Env,
    t: &mut Map<Address, i128>,
    ctx: &Context,
    caller: &Address,
    user: &Address,
    col: i128,
    s: &Settlement,
) {
    let user_payout = s.equity(col).max(0);
    let treasury_fee = ctx.treasury_fee(e, s.protocol_fee());
    let caller_fee = s.trading_fee()
        .fixed_mul_floor(e, &ctx.trading_config.caller_rate, &SCALAR_7);
    let vault_transfer = col - user_payout - treasury_fee - caller_fee;

    if user_payout > 0 { add_transfer(t, user, user_payout); }
    if vault_transfer != 0 { add_transfer(t, &ctx.vault, vault_transfer); }
    if treasury_fee > 0 { add_transfer(t, &ctx.treasury, treasury_fee); }
    if caller_fee > 0 { add_transfer(t, caller, caller_fee); }
}

/// Distribute transfers for a liquidation.
fn settle_liquidation(
    e: &Env,
    t: &mut Map<Address, i128>,
    ctx: &Context,
    caller: &Address,
    position: &Position,
    user: &Address,
    id: u32,
    col: i128,
    s: &Settlement,
    equity: i128,
) {
    // liq_fee is the residual equity at liquidation (clamped to 0 from below).
    // The configured liq_fee threshold gates the liquidation path above; this
    // gives the keeper whatever equity remains. Underwater positions yield 0.
    let liq_fee = equity.max(0);
    let revenue = (s.protocol_fee() + liq_fee).min(col);
    let treasury_fee = ctx.treasury_fee(e, revenue);
    let caller_fee = (s.trading_fee() + liq_fee).min(col)
        .fixed_mul_floor(e, &ctx.trading_config.caller_rate, &SCALAR_7);

    add_transfer(t, &ctx.vault, col - treasury_fee - caller_fee);
    if treasury_fee > 0 { add_transfer(t, &ctx.treasury, treasury_fee); }
    if caller_fee > 0 { add_transfer(t, caller, caller_fee); }

    Liquidation {
        market_id: position.market_id,
        user: user.clone(),
        position_id: id,
        price: ctx.price,
        base_fee: s.base_fee,
        impact_fee: s.impact_fee,
        funding: s.funding,
        borrowing_fee: s.borrowing_fee,
        liq_fee,
    }
    .publish(e);
}

/// Fill a pending limit order.
fn apply_fill(
    e: &Env,
    t: &mut Map<Address, i128>,
    ctx: &mut Context,
    caller: &Address,
    position: &mut Position,
    user: &Address,
    id: u32,
) {
    if position.filled {
        panic_with_error!(e, TradingError::PositionNotPending);
    }

    // Long limit: fills when market price falls to or below the entry (buy at or better).
    // Short limit: fills when market price rises to or above the entry (sell at or better).
    let can_fill = if position.long {
        ctx.price <= position.entry_price
    } else {
        ctx.price >= position.entry_price
    };
    if !can_fill {
        panic_with_error!(e, TradingError::NotActionable);
    }

    position.entry_price = ctx.price;

    let (base_fee, impact_fee) = ctx.open(e, position, user, id);
    let total_fee = base_fee + impact_fee;
    let treasury_fee = ctx.treasury_fee(e, total_fee);
    let caller_fee = total_fee
        .fixed_mul_floor(e, &ctx.trading_config.caller_rate, &SCALAR_7);
    let vault_fee = total_fee - treasury_fee - caller_fee;

    add_transfer(t, &ctx.vault, vault_fee);
    if treasury_fee > 0 { add_transfer(t, &ctx.treasury, treasury_fee); }
    if caller_fee > 0 { add_transfer(t, caller, caller_fee); }

    FillLimit {
        market_id: position.market_id,
        user: user.clone(),
        position_id: id,
        base_fee,
        impact_fee,
    }
    .publish(e);
}

#[cfg(test)]
mod tests {
    use crate::constants::SCALAR_7;
    use crate::storage;
    use crate::testutils::{
        setup_contract, setup_env, FEED_BTC, BTC_PRICE, PRICE_SCALAR,
    };
    use crate::dependencies::PriceData;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{vec, Address};

    fn btc_price_data(e: &soroban_sdk::Env, price: i128) -> PriceData {
        PriceData {
            feed_id: FEED_BTC,
            price,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        }
    }

    fn create_pending_long(
        e: &soroban_sdk::Env,
        contract: &Address,
        user: &Address,
        collateral: i128,
        notional: i128,
        entry_price: i128,
    ) -> u32 {
        e.as_contract(contract, || {
            crate::trading::execute_create_limit(
                e, user, FEED_BTC, collateral, notional, true, entry_price, 0, 0,
            )
        })
    }

    fn create_pending_short(
        e: &soroban_sdk::Env,
        contract: &Address,
        user: &Address,
        collateral: i128,
        notional: i128,
        entry_price: i128,
    ) -> u32 {
        e.as_contract(contract, || {
            crate::trading::execute_create_limit(
                e, user, FEED_BTC, collateral, notional, false, entry_price, 0, 0,
            )
        })
    }

    /// Helper: build parallel user/seq vecs for a single position trigger.
    fn trigger_one(e: &soroban_sdk::Env, user: &Address, id: u32) -> (soroban_sdk::Vec<Address>, soroban_sdk::Vec<u32>) {
        (vec![e, user.clone()], vec![e, id])
    }

    #[test]
    fn test_fill_long_limit_order() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);

        let pd = btc_price_data(&e, BTC_PRICE);
        let caller_before = token_client.balance(&caller);
        e.as_contract(&contract, || {
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);

            let pos = storage::get_position(&e, &user, id);
            assert!(pos.filled);
            assert_eq!(pos.col, 9_949_999_988);
        });
        assert_eq!(token_client.balance(&caller) - caller_before, 5_000_001);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #731)")]
    fn test_fill_long_limit_not_fillable() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_long(
            &e, &contract, &user,
            1_000 * SCALAR_7, 10_000 * SCALAR_7,
            90_000 * PRICE_SCALAR,
        );

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);
        });
    }

    #[test]
    fn test_fill_short_limit_order() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_short(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);

            let pos = storage::get_position(&e, &user, id);
            assert!(pos.filled);
            assert_eq!(pos.col, 9_949_999_988);
        });
    }

    #[test]
    fn test_liquidation_underwater_position() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_long(&e, &contract, &user, 1_100 * SCALAR_7, 100_000 * SCALAR_7, BTC_PRICE);

        let balance_after_create = token_client.balance(&user);
        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);

            // Price crashes -2% on 100x leverage → underwater
            let crash_pd = btc_price_data(&e, 9_800_000_000_000_i128);
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &crash_pd);
        });
        // User gets nothing back (underwater liquidation)
        assert_eq!(token_client.balance(&user), balance_after_create);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #731)")]
    fn test_liquidation_healthy_position() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);

            // Price unchanged, no SL/TP set — no action should be possible
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);
        });
    }

    #[test]
    fn test_stop_loss_triggered() {
        use crate::testutils::jump;
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = e.as_contract(&contract, || {
            crate::trading::execute_create_limit(
                &e, &user, FEED_BTC,
                1_000 * SCALAR_7,
                10_000 * SCALAR_7,
                true,
                BTC_PRICE,
                0,
                95_000 * PRICE_SCALAR,
            )
        });

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);
        });

        jump(&e, 1000 + 31);

        let balance_before_sl = token_client.balance(&user);
        e.as_contract(&contract, || {
            let sl_pd = btc_price_data(&e, 9_400_000_000_000_i128);
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &sl_pd);
        });
        let balance_after_sl = token_client.balance(&user);
        assert!(balance_after_sl > balance_before_sl, "user should receive SL payout");
    }

    #[test]
    fn test_take_profit_triggered() {
        use crate::testutils::jump;
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = e.as_contract(&contract, || {
            crate::trading::execute_create_limit(
                &e, &user, FEED_BTC,
                1_000 * SCALAR_7,
                10_000 * SCALAR_7,
                true,
                BTC_PRICE,
                110_000 * PRICE_SCALAR,
                0,
            )
        });

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);
        });

        jump(&e, 1000 + 31);

        let balance_before_tp = token_client.balance(&user);
        e.as_contract(&contract, || {
            let tp_pd = btc_price_data(&e, 11_500_000_000_000_i128);
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &tp_pd);
        });
        let balance_after_tp = token_client.balance(&user);
        assert!(balance_after_tp > balance_before_tp + 1_000 * SCALAR_7,
            "TP payout should exceed original collateral");
    }

    #[test]
    fn test_batch_multiple_requests() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(1_000_000 * SCALAR_7));

        let id1 = create_pending_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);
        let id2 = create_pending_short(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);

        let caller_before = token_client.balance(&caller);
        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let users = vec![&e, user.clone(), user.clone()];
            let ids = vec![&e, id1, id2];
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);

            let pos1 = storage::get_position(&e, &user, id1);
            let pos2 = storage::get_position(&e, &user, id2);
            assert!(pos1.filled);
            assert!(pos2.filled);
        });
        // Caller earned fees from both fills
        assert!(token_client.balance(&caller) > caller_before);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #731)")]
    fn test_fill_already_filled_panics() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);

            // Already filled, no SL/TP, not liquidatable — should panic
            let (users, ids) = trigger_one(&e, &user, id);
            super::execute_trigger(&e, &caller, FEED_BTC, users, ids, &pd);
        });
    }

}
