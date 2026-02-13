use crate::constants::MIN_LEVERAGE;
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{CancelLimit, ClosePosition, ModifyCollateral, OpenMarket, PlaceLimit, SetTriggers};
use crate::storage;
use crate::trading::market::Market;
use crate::trading::oracle::load_price;
use crate::trading::position::Position;
use crate::validation::{require_active, require_min_open_time, require_not_frozen, require_market_enabled};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, Address, Env};

/// Close a position (handles both Open and Pending positions)
/// Pending positions are cancelled, Open positions are closed with PnL settlement
/// Returns (pnl, fee) tuple
pub fn execute_close_position(e: &Env, position_id: u32) -> (i128, i128) {
    require_not_frozen(e);
    let position = Position::load(e, position_id);
    position.user.require_auth();

    let mut market = Market::load(e, position.asset_index);
    market.accrue_interest(e);

    let result = if position.filled {
        require_min_open_time(e, &position);
        close_filled_position(e, &position, &mut market)
    } else {
        cancel_pending_position(e, &position, &market)
    };

    storage::remove_user_position(e, &position.user, position.id);
    storage::remove_position(e, position.id);
    market.store(e);

    result
}

/// Cancel a pending limit order - refunds collateral + fees
fn cancel_pending_position(e: &Env, position: &Position, market: &Market) -> (i128, i128) {
    let token = storage::get_token(e);
    let token_client = TokenClient::new(e, &token);
    let token_scalar = storage::get_token_scalar(e);

    // Calculate fees using the same formula as open (notional_size based)
    let base_fee = position
        .notional_size
        .fixed_mul_ceil(e, &market.config.base_fee, &token_scalar);
    let price_impact = position
        .notional_size
        .fixed_div_ceil(e, &market.config.price_impact_scalar, &token_scalar);

    // Refund collateral + fees to user
    let total_refund = position.collateral + base_fee + price_impact;
    token_client.transfer(&e.current_contract_address(), &position.user, &total_refund);

    CancelLimit {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id: position.id,
        base_fee,
        impact_fee: price_impact,
    }
    .publish(e);

    (0, 0)
}

/// Close a filled position with PnL settlement
fn close_filled_position(e: &Env, position: &Position, market: &mut Market) -> (i128, i128) {
    let config = storage::get_config(e);
    let vault = storage::get_vault(e);
    let token = storage::get_token(e);
    let token_scalar = storage::get_token_scalar(e);

    let price = load_price(e, &config.oracle, &market.config.asset, config.max_price_age);
    let price_scalar = storage::get_price_scalar(e);
    let pnl = position.calculate_pnl(e, price, price_scalar);
    let fees = position.calculate_fee_breakdown(e, market);

    // Calculate payouts
    let equity = position.collateral + pnl - fees.total_fee();
    let max_payout = position
        .collateral
        .fixed_mul_floor(e, &market.config.max_payout, &token_scalar);
    let user_payout = equity.max(0).min(max_payout);
    let vault_transfer = position.collateral - user_payout;

    let token_client = TokenClient::new(e, &token);
    let vault_client = VaultClient::new(e, &vault);

    // Handle vault transfer (negative = vault pays, positive = vault receives)
    if vault_transfer < 0 {
        vault_client.strategy_withdraw(&e.current_contract_address(), &(-vault_transfer));
    } else if vault_transfer > 0 {
        token_client.transfer(&e.current_contract_address(), &vault, &vault_transfer);
    }

    // Pay user their payout
    if user_payout > 0 {
        token_client.transfer(&e.current_contract_address(), &position.user, &user_payout);
    }

    // Update market stats
    market.update_stats(-position.notional_size, position.is_long);

    ClosePosition {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id: position.id,
        price,
        pnl,
        base_fee: fees.base_fee,
        impact_fee: fees.impact_fee,
        interest: fees.interest,
    }
    .publish(e);

    (pnl, fees.total_fee())
}

/// Modify collateral on a position to a new absolute value
/// Works for both filled positions and pending limit orders
pub fn execute_modify_collateral(e: &Env, position_id: u32, new_collateral: i128) {
    require_not_frozen(e);
    let mut position = Position::load(e, position_id);
    position.user.require_auth();

    if new_collateral <= 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    let market_config = storage::get_market_config(e, position.asset_index);
    let token_scalar = storage::get_token_scalar(e);

    // Check collateral bounds
    if new_collateral < market_config.min_collateral {
        panic_with_error!(e, TradingError::CollateralBelowMinimum);
    }
    if new_collateral > market_config.max_collateral {
        panic_with_error!(e, TradingError::CollateralAboveMaximum);
    }

    // Check minimum leverage
    let min_notional = new_collateral.fixed_mul_ceil(e, &(MIN_LEVERAGE * token_scalar), &token_scalar);
    if position.notional_size < min_notional {
        panic_with_error!(e, TradingError::LeverageBelowMinimum);
    }

    let token = storage::get_token(e);
    let token_client = TokenClient::new(e, &token);
    let collateral_diff = new_collateral - position.collateral;

    if collateral_diff == 0 {
        panic_with_error!(e, TradingError::CollateralUnchanged);
    }

    if position.filled {
        // Filled position: need to check margin and update market stats
        let config = storage::get_config(e);
        let mut market = Market::load(e, position.asset_index);
        market.accrue_interest(e);

        if collateral_diff > 0 {
            token_client.transfer(&position.user, &e.current_contract_address(), &collateral_diff);
        } else {
            let current_price =
                load_price(e, &config.oracle, &market.config.asset, config.max_price_age);
            let price_scalar = storage::get_price_scalar(e);
            let pnl = position.calculate_pnl(e, current_price, price_scalar);
            let fees = position.calculate_fee_breakdown(e, &market);
            let equity = new_collateral + pnl - fees.total_fee();
            let required_margin = position
                .notional_size
                .fixed_mul_floor(e, &market.config.init_margin, &token_scalar);

            if equity < required_margin {
                panic_with_error!(e, TradingError::WithdrawalBreaksMargin);
            }

            token_client.transfer(&e.current_contract_address(), &position.user, &-collateral_diff);
        }

        market.store(e);
    } else {
        // Pending limit order: check margin requirement for when it fills
        if collateral_diff > 0 {
            token_client.transfer(&position.user, &e.current_contract_address(), &collateral_diff);
        } else {
            let required_margin = position
                .notional_size
                .fixed_mul_floor(e, &market_config.init_margin, &token_scalar);

            if new_collateral < required_margin {
                panic_with_error!(e, TradingError::WithdrawalBreaksMargin);
            }

            token_client.transfer(&e.current_contract_address(), &position.user, &-collateral_diff);
        }
    }

    position.collateral = new_collateral;
    position.store(e);

    ModifyCollateral {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id: position.id,
        amount: collateral_diff,
    }
    .publish(e);
}

/// Set take profit and stop loss triggers
/// Use 0 to clear/disable a trigger
pub fn execute_set_triggers(e: &Env, position_id: u32, take_profit: i128, stop_loss: i128) {
    require_not_frozen(e);
    let mut position = Position::load(e, position_id);
    position.user.require_auth();

    let config = storage::get_config(e);
    let market_config = storage::get_market_config(e, position.asset_index);
    let current_price = load_price(e, &config.oracle, &market_config.asset, config.max_price_age);

    // Validate and set take profit
    if take_profit > 0
        && ((position.is_long && take_profit <= current_price)
        || (!position.is_long && take_profit >= current_price))
    {
        panic_with_error!(e, TradingError::InvalidTakeProfitPrice);
    }

    if stop_loss > 0
        && ((position.is_long && stop_loss >= current_price)
        || (!position.is_long && stop_loss <= current_price))
    {
        panic_with_error!(e, TradingError::InvalidStopLossPrice);
    }

    position.take_profit = take_profit;
    position.stop_loss = stop_loss;

    SetTriggers {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id: position.id,
        take_profit,
        stop_loss,
    }
    .publish(e);

    position.store(e);
}

#[allow(clippy::too_many_arguments)]
pub fn execute_create_position(
    e: &Env,
    user: &Address,
    asset_index: u32,
    collateral: i128,
    notional_size: i128,
    is_long: bool,
    entry_price: i128,
    take_profit: i128,
    stop_loss: i128,
) -> (u32, i128) {
    require_active(e);
    user.require_auth();

    let config = storage::get_config(e);
    let vault = storage::get_vault(e);
    let token = storage::get_token(e);
    let token_scalar = storage::get_token_scalar(e);

    let mut market = Market::load(e, asset_index);
    market.accrue_interest(e);
    require_market_enabled(e, &market.config);

    if collateral < 0 || notional_size < 0 || entry_price < 0 || take_profit < 0 || stop_loss < 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // Check utilization limit: total_notional must not exceed vault_assets * max_utilization
    let vault_client = VaultClient::new(e, &vault);
    let vault_assets = vault_client.total_assets();
    let current_total_notional = market.data.long_notional_size + market.data.short_notional_size;
    let new_total_notional = current_total_notional + notional_size;
    let max_allowed_notional = vault_assets.fixed_mul_floor(e, &config.max_utilization, &token_scalar);
    if new_total_notional > max_allowed_notional {
        panic_with_error!(e, TradingError::UtilizationLimitExceeded);
    }

    // Check collateral bounds
    if collateral < market.config.min_collateral {
        panic_with_error!(e, TradingError::CollateralBelowMinimum);
    }
    if collateral > market.config.max_collateral {
        panic_with_error!(e, TradingError::CollateralAboveMaximum);
    }

    // Check minimum leverage (notional_size / collateral >= MIN_LEVERAGE)
    let min_notional = collateral.fixed_mul_ceil(e, &(MIN_LEVERAGE * token_scalar), &token_scalar);
    if notional_size < min_notional {
        panic_with_error!(e, TradingError::LeverageBelowMinimum);
    }

    // Check user position count limit
    let positions = storage::get_user_positions(e, user);
    if positions.len() >= config.max_positions {
        panic_with_error!(e, TradingError::MaxPositionsReached)
    }

    let current_price = load_price(e, &config.oracle, &market.config.asset, config.max_price_age);
    let market_order = entry_price == 0;

    let actual_entry_price = if market_order {
        current_price
    } else {
        // Check if entry price is valid
        if (is_long && entry_price < current_price) || (!is_long && entry_price > current_price) {
            panic_with_error!(e, TradingError::InvalidEntryPrice);
        }
        entry_price
    };

    // Calculate what dominance WOULD be AFTER adding this position
    let new_long = market.data.long_notional_size + if is_long { notional_size } else { 0 };
    let new_short = market.data.short_notional_size + if !is_long { notional_size } else { 0 };

    let would_be_long_dominant = new_long > new_short;
    let would_be_short_dominant = new_short > new_long;

    // For market orders: charge fee if this position would make/keep its side dominant
    // For limit orders: always charge fee upfront (refunded on fill if balancing)
    let should_pay_base_fee = !market_order
        || (would_be_long_dominant && is_long)
        || (would_be_short_dominant && !is_long);

    // If market order, update market stats immediately
    if market_order {
        market.update_stats(notional_size, is_long);
    }

    let interest_index = if is_long {
        market.data.long_interest_index
    } else {
        market.data.short_interest_index
    };

    let id = storage::next_position_id(e);
    let position = Position::new(
        e,
        id,
        user.clone(),
        market_order, // filled = true for market orders, false for limit orders
        asset_index,
        is_long,
        stop_loss,
        take_profit,
        actual_entry_price,
        collateral,
        notional_size,
        interest_index,
    );

    let open_fee = if should_pay_base_fee {
        notional_size.fixed_mul_ceil(e, &market.config.base_fee, &token_scalar)
    } else {
        0 // No base fee for balancing trades
    };

    let price_impact_scalar =
        notional_size.fixed_div_ceil(e, &market.config.price_impact_scalar, &token_scalar);

    // Transfer tokens from user to contract
    let token_client = TokenClient::new(e, &token);
    token_client.transfer(
        user,
        &e.current_contract_address(),
        &(collateral + open_fee + price_impact_scalar),
    );

    // Only pay fee to vault when the position fills
    if market_order {
        let vault_transfer = open_fee + price_impact_scalar;
        // Direct transfer to vault
        token_client.transfer(&e.current_contract_address(), &vault, &vault_transfer);
    }

    market.store(e);
    position.store(e);

    storage::add_user_position(e, user, id);

    if market_order {
        OpenMarket {
            asset_index,
            user: user.clone(),
            position_id: id,
            base_fee: open_fee,
            impact_fee: price_impact_scalar,
        }
        .publish(e);
    } else {
        PlaceLimit {
            asset_index,
            user: user.clone(),
            position_id: id,
            base_fee: open_fee,
            impact_fee: price_impact_scalar,
        }
        .publish(e);
    }

    (id, open_fee + price_impact_scalar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{SCALAR_7, SCALAR_18};
    use crate::testutils::{default_market_data, setup_contract, setup_env, BTC_PRICE};
    use crate::types::Position as PositionType;
    use soroban_sdk::testutils::Address as _;

    // base_fee rate = 0.05% → 0.05% of 1000 tokens = 0.5 tokens
    const BASE_FEE: i128 = 5_000_000;

    fn create_test_position(e: &soroban_sdk::Env, user: &Address, filled: bool) -> PositionType {
        PositionType {
            id: 1,
            user: user.clone(),
            filled,
            asset_index: 0,
            is_long: true,
            stop_loss: 0,
            take_profit: 0,
            entry_price: BTC_PRICE,
            collateral: 100 * SCALAR_7,
            notional_size: 1000 * SCALAR_7,
            interest_index: SCALAR_18,
            created_at: e.ledger().timestamp(),
        }
    }

    // ==========================================
    // Status restriction tests
    // ==========================================

    #[test]
    #[should_panic(expected = "Error(Contract, #761)")]
    fn test_open_position_onice() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            storage::set_status(&e, 1); // OnIce
            execute_create_position(&e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #712)")]
    fn test_open_position_market_disabled() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            let mut config = storage::get_market_config(&e, 0);
            config.enabled = false;
            storage::set_market_config(&e, 0, &config);
            execute_create_position(&e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #762)")]
    fn test_close_position_frozen() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            storage::set_status(&e, 2); // Frozen
            execute_close_position(&e, 1);
        });
    }

    // ==========================================
    // execute_create_position tests
    // ==========================================

    #[test]
    fn test_create_market_order_long() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let mint_amount = 10_000 * SCALAR_7;
        token_client.mint(&user, &mint_amount);

        let collateral = 100 * SCALAR_7;
        let notional = 1000 * SCALAR_7;

        e.as_contract(&contract, || {
            let (id, fee) = execute_create_position(
                &e, &user, 0, collateral, notional, true, 0, 0, 0,
            );

            // Verify returned values — first position on empty market, long is dominant, pays base fee
            assert_eq!(id, 0);
            assert!(fee >= BASE_FEE); // base_fee + negligible impact fee

            // Verify position state
            let pos = storage::get_position(&e, 0);
            assert_eq!(pos.id, 0);
            assert_eq!(pos.user, user);
            assert!(pos.filled);
            assert_eq!(pos.asset_index, 0);
            assert!(pos.is_long);
            assert_eq!(pos.entry_price, BTC_PRICE); // market order uses current price
            assert_eq!(pos.collateral, collateral);
            assert_eq!(pos.notional_size, notional);
            assert_eq!(pos.interest_index, SCALAR_18);
            assert_eq!(pos.take_profit, 0);
            assert_eq!(pos.stop_loss, 0);
            assert_eq!(pos.created_at, e.ledger().timestamp());

            // Verify market data updated
            let data = storage::get_market_data(&e, 0);
            assert_eq!(data.long_notional_size, notional);
            assert_eq!(data.short_notional_size, 0);

            // Verify user position list
            let positions = storage::get_user_positions(&e, &user);
            assert_eq!(positions.len(), 1);
            assert_eq!(positions.get(0), Some(0));

            // Verify token balances: user paid collateral + fees
            let user_balance = token_client.balance(&user);
            assert_eq!(user_balance, mint_amount - collateral - fee);
        });
    }

    #[test]
    fn test_create_limit_order_short() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let mint_amount = 10_000 * SCALAR_7;
        token_client.mint(&user, &mint_amount);

        let collateral = 100 * SCALAR_7;
        let notional = 1000 * SCALAR_7;
        let entry_price = BTC_PRICE - 1000 * SCALAR_7; // below current for short limit

        e.as_contract(&contract, || {
            let (id, fee) = execute_create_position(
                &e, &user, 0, collateral, notional, false, entry_price, 0, 0,
            );

            // Verify returned values — limit orders always charge base fee upfront
            assert_eq!(id, 0);
            assert!(fee >= BASE_FEE);

            // Verify position state
            let pos = storage::get_position(&e, 0);
            assert_eq!(pos.id, 0);
            assert!(!pos.filled); // limit order is pending
            assert!(!pos.is_long);
            assert_eq!(pos.entry_price, entry_price);
            assert_eq!(pos.collateral, collateral);
            assert_eq!(pos.notional_size, notional);
            assert_eq!(pos.interest_index, SCALAR_18); // short interest index
            assert_eq!(pos.take_profit, 0);
            assert_eq!(pos.stop_loss, 0);

            // Verify market data NOT updated (limit order hasn't filled)
            let data = storage::get_market_data(&e, 0);
            assert_eq!(data.long_notional_size, 0);
            assert_eq!(data.short_notional_size, 0);

            // Verify token balances: user paid collateral + fees, fees held by contract (not sent to vault)
            let user_balance = token_client.balance(&user);
            assert_eq!(user_balance, mint_amount - collateral - fee);
        });
    }

    #[test]
    fn test_create_position_balancing_trade_no_base_fee() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(20_000 * SCALAR_7));

        let collateral = 100 * SCALAR_7;
        let notional = 1000 * SCALAR_7;

        e.as_contract(&contract, || {
            // Pre-seed market with existing long dominance
            let mut data = storage::get_market_data(&e, 0);
            data.long_notional_size = 5000 * SCALAR_7;
            storage::set_market_data(&e, 0, &data);

            // Open a short (balancing trade) — should NOT pay base fee
            let (_, fee) = execute_create_position(
                &e, &user, 0, collateral, notional, false, 0, 0, 0,
            );
            assert!(fee < BASE_FEE); // only negligible impact fee, no base fee
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #735)")]
    fn test_create_position_negative_collateral() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            execute_create_position(&e, &user, 0, -1, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #736)")]
    fn test_create_position_collateral_below_min() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            execute_create_position(&e, &user, 0, SCALAR_7 / 2, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #737)")]
    fn test_create_position_collateral_above_max() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            execute_create_position(&e, &user, 0, 2_000_000 * SCALAR_7, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #738)")]
    fn test_create_position_leverage_below_min() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            // collateral=100, notional=100 → 1x leverage, below MIN_LEVERAGE=2
            execute_create_position(&e, &user, 0, 100 * SCALAR_7, 100 * SCALAR_7, true, 0, 0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #739)")]
    fn test_create_position_invalid_entry_price_long() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            // Long limit below current price is invalid
            execute_create_position(
                &e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, true,
                BTC_PRICE - 1000 * SCALAR_7, 0, 0,
            );
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #739)")]
    fn test_create_position_invalid_entry_price_short() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            // Short limit above current price is invalid
            execute_create_position(
                &e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, false,
                BTC_PRICE + 1000 * SCALAR_7, 0, 0,
            );
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #734)")]
    fn test_create_position_max_positions() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        e.as_contract(&contract, || {
            for i in 0..10 {
                storage::add_user_position(&e, &user, i);
            }
            execute_create_position(&e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #770)")]
    fn test_create_position_utilization_limit_exceeded() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            // vault has 100M tokens, max_utilization = 50x → max notional = 5B tokens
            // Pre-seed market right at the limit
            let mut data = storage::get_market_data(&e, 0);
            data.long_notional_size = 5_000_000_000 * SCALAR_7;
            storage::set_market_data(&e, 0, &data);

            // Any new position should exceed the limit
            execute_create_position(&e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    // ==========================================
    // execute_close_position tests
    // ==========================================

    #[test]
    fn test_close_filled_position() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut data = default_market_data();
            data.long_notional_size = position.notional_size;
            data.long_interest_index = SCALAR_18;
            data.short_interest_index = SCALAR_18;
            data.last_update = e.ledger().timestamp();
            storage::set_market_data(&e, 0, &data);

            let (pnl, fee) = execute_close_position(&e, 1);
            assert_eq!(pnl, 0);
            assert!(fee >= 0);
        });
    }

    #[test]
    fn test_close_filled_position_profitable() {
        use sep_40_oracle::testutils::MockPriceOracleClient;
        use soroban_sdk::vec as svec;

        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let collateral = 100 * SCALAR_7;

        // Set up position and market data, grab oracle address
        let oracle_addr = e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut data = default_market_data();
            data.long_notional_size = position.notional_size;
            data.long_interest_index = SCALAR_18;
            data.short_interest_index = SCALAR_18;
            data.last_update = e.ledger().timestamp();
            storage::set_market_data(&e, 0, &data);

            storage::get_config(&e).oracle
        });

        // Move oracle price up 10% (must be outside as_contract for mock auth)
        let oracle_client = MockPriceOracleClient::new(&e, &oracle_addr);
        oracle_client.set_price_stable(&svec![&e, 110_000 * SCALAR_7]);

        // Close — triggers strategy_withdraw (vault pays user)
        e.as_contract(&contract, || {
            let (pnl, fee) = execute_close_position(&e, 1);

            // 10% gain on 1000 token notional = 100 tokens profit
            assert_eq!(pnl, 100 * SCALAR_7);
            assert!(fee >= BASE_FEE);

            // User received more than their collateral (vault paid the difference)
            let user_balance = token_client.balance(&user);
            assert!(user_balance > collateral);
        });
    }

    #[test]
    fn test_cancel_pending_position() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, false);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let (pnl, fee) = execute_close_position(&e, 1);
            assert_eq!(pnl, 0);
            assert_eq!(fee, 0);
        });
    }

    // ==========================================
    // execute_modify_collateral tests
    // ==========================================

    #[test]
    fn test_modify_collateral_deposit() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(1000 * SCALAR_7));

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);

            let mut data = default_market_data();
            data.long_notional_size = position.notional_size;
            storage::set_market_data(&e, 0, &data);

            execute_modify_collateral(&e, 1, 150 * SCALAR_7);
            assert_eq!(storage::get_position(&e, 1).collateral, 150 * SCALAR_7);
        });
    }

    #[test]
    fn test_modify_collateral_withdraw() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);

            let mut data = default_market_data();
            data.long_notional_size = position.notional_size;
            storage::set_market_data(&e, 0, &data);

            execute_modify_collateral(&e, 1, 90 * SCALAR_7);
            assert_eq!(storage::get_position(&e, 1).collateral, 90 * SCALAR_7);
        });
    }

    #[test]
    fn test_modify_collateral_pending_position() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, false); // pending position
            storage::set_position(&e, 1, &position);
            execute_modify_collateral(&e, 1, 150 * SCALAR_7);
            assert_eq!(storage::get_position(&e, 1).collateral, 150 * SCALAR_7);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #740)")]
    fn test_modify_collateral_unchanged() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            // Same collateral as existing (100 * SCALAR_7)
            execute_modify_collateral(&e, 1, 100 * SCALAR_7);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #735)")]
    fn test_modify_collateral_negative() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            execute_modify_collateral(&e, 1, -1);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #736)")]
    fn test_modify_collateral_below_minimum() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            // min_collateral = SCALAR_7 (1 token), set below that
            execute_modify_collateral(&e, 1, SCALAR_7 / 2);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #737)")]
    fn test_modify_collateral_above_maximum() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            // max_collateral = 1_000_000 * SCALAR_7
            execute_modify_collateral(&e, 1, 2_000_000 * SCALAR_7);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #738)")]
    fn test_modify_collateral_leverage_below_minimum() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            // notional_size = 1000, so max collateral for 2x = 500
            storage::set_position(&e, 1, &position);
            execute_modify_collateral(&e, 1, 600 * SCALAR_7);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #741)")]
    fn test_modify_collateral_withdraw_breaks_margin_filled() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);

            let mut data = default_market_data();
            data.long_notional_size = position.notional_size;
            data.long_interest_index = SCALAR_18;
            data.short_interest_index = SCALAR_18;
            data.last_update = e.ledger().timestamp();
            storage::set_market_data(&e, 0, &data);

            // init_margin = 1%, notional = 1000 → required margin = 10 tokens
            // With fees the required equity is ~10 tokens + fees
            // Setting collateral to 2 tokens should break margin
            execute_modify_collateral(&e, 1, 2 * SCALAR_7);
        });
    }

    #[test]
    fn test_modify_collateral_withdraw_pending() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, false); // pending
            storage::set_position(&e, 1, &position);

            // init_margin = 1%, notional = 1000 → required margin = 10
            // 90 tokens is well above 10, should succeed
            execute_modify_collateral(&e, 1, 90 * SCALAR_7);
            assert_eq!(storage::get_position(&e, 1).collateral, 90 * SCALAR_7);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #741)")]
    fn test_modify_collateral_withdraw_breaks_margin_pending() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, false); // pending
            storage::set_position(&e, 1, &position);

            // init_margin = 1%, notional = 1000 → required margin = 10
            // Setting collateral to 2 tokens (below 10) should break margin
            execute_modify_collateral(&e, 1, 2 * SCALAR_7);
        });
    }

    // ==========================================
    // execute_set_triggers tests
    // ==========================================

    #[test]
    fn test_set_triggers_long() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);

            execute_set_triggers(&e, 1, BTC_PRICE + 5000 * SCALAR_7, BTC_PRICE - 5000 * SCALAR_7);

            let pos = storage::get_position(&e, 1);
            assert_eq!(pos.take_profit, BTC_PRICE + 5000 * SCALAR_7);
            assert_eq!(pos.stop_loss, BTC_PRICE - 5000 * SCALAR_7);
        });
    }

    #[test]
    fn test_set_triggers_short() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let mut position = create_test_position(&e, &user, true);
            position.is_long = false;
            storage::set_position(&e, 1, &position);

            execute_set_triggers(&e, 1, BTC_PRICE - 5000 * SCALAR_7, BTC_PRICE + 5000 * SCALAR_7);

            let pos = storage::get_position(&e, 1);
            assert_eq!(pos.take_profit, BTC_PRICE - 5000 * SCALAR_7);
            assert_eq!(pos.stop_loss, BTC_PRICE + 5000 * SCALAR_7);
        });
    }

    #[test]
    fn test_clear_triggers() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let mut position = create_test_position(&e, &user, true);
            position.take_profit = BTC_PRICE + 5000 * SCALAR_7;
            position.stop_loss = BTC_PRICE - 5000 * SCALAR_7;
            storage::set_position(&e, 1, &position);

            execute_set_triggers(&e, 1, 0, 0);

            let pos = storage::get_position(&e, 1);
            assert_eq!(pos.take_profit, 0);
            assert_eq!(pos.stop_loss, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #742)")]
    fn test_set_triggers_invalid_tp_long() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            execute_set_triggers(&e, 1, BTC_PRICE - 1000 * SCALAR_7, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #743)")]
    fn test_set_triggers_invalid_sl_long() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            execute_set_triggers(&e, 1, 0, BTC_PRICE + 1000 * SCALAR_7);
        });
    }

    // ==========================================
    // min_open_time tests
    // ==========================================

    #[test]
    #[should_panic(expected = "Error(Contract, #748)")]
    fn test_close_position_too_new() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            // Set min_open_time to 60 seconds
            let mut config = storage::get_config(&e);
            config.min_open_time = 60;
            storage::set_config(&e, &config);

            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut data = default_market_data();
            data.long_notional_size = position.notional_size;
            data.long_interest_index = SCALAR_18;
            data.short_interest_index = SCALAR_18;
            data.last_update = e.ledger().timestamp();
            storage::set_market_data(&e, 0, &data);

            // Try to close immediately — should panic with PositionTooNew
            execute_close_position(&e, 1);
        });
    }

    #[test]
    fn test_close_position_after_min_open_time() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            // Set min_open_time to 60 seconds
            let mut config = storage::get_config(&e);
            config.min_open_time = 60;
            storage::set_config(&e, &config);

            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut data = default_market_data();
            data.long_notional_size = position.notional_size;
            data.long_interest_index = SCALAR_18;
            data.short_interest_index = SCALAR_18;
            data.last_update = e.ledger().timestamp();
            storage::set_market_data(&e, 0, &data);

            // Advance time past min_open_time
            crate::testutils::jump(&e, e.ledger().timestamp() + 61);

            // Should succeed
            let (pnl, _fee) = execute_close_position(&e, 1);
            assert_eq!(pnl, 0);
        });
    }

    #[test]
    fn test_cancel_pending_ignores_min_open_time() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            // Set min_open_time to 60 seconds
            let mut config = storage::get_config(&e);
            config.min_open_time = 60;
            storage::set_config(&e, &config);

            let position = create_test_position(&e, &user, false); // pending
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            // Cancel immediately — should succeed (pending positions exempt)
            let (pnl, fee) = execute_close_position(&e, 1);
            assert_eq!(pnl, 0);
            assert_eq!(fee, 0);
        });
    }

    #[test]
    fn test_close_position_min_open_time_disabled() {
        let e = setup_env();
        let (contract, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            // min_open_time = 0 (disabled, which is the default)
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut data = default_market_data();
            data.long_notional_size = position.notional_size;
            data.long_interest_index = SCALAR_18;
            data.short_interest_index = SCALAR_18;
            data.last_update = e.ledger().timestamp();
            storage::set_market_data(&e, 0, &data);

            // Close immediately — should succeed (min_open_time disabled)
            let (pnl, _fee) = execute_close_position(&e, 1);
            assert_eq!(pnl, 0);
        });
    }
}
