use crate::constants::{MIN_LEVERAGE, SCALAR_7};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{CancelLimit, ClosePosition, ModifyCollateral, OpenMarket, PlaceLimit, SetTriggers};
use crate::storage;
use crate::trading::market::{load_price, Market};
use crate::trading::position::Position;
use crate::validation::{require_active, require_not_frozen, require_market_enabled};
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

    // Calculate fees using the same formula as open (notional_size based)
    let base_fee = position
        .notional_size
        .fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7);
    let price_impact = position
        .notional_size
        .fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

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

    let price = load_price(e, &config.oracle, &market.config.asset, config.max_price_age);
    let pnl = position.calculate_pnl(e, price);
    let fees = position.calculate_fee_breakdown(e, market);

    // Calculate payouts
    let equity = position.collateral + pnl - fees.total_fee();
    let max_payout = position
        .collateral
        .fixed_mul_floor(e, &market.config.max_payout, &SCALAR_7);
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

    // Check collateral bounds
    if new_collateral < market_config.min_collateral {
        panic_with_error!(e, TradingError::CollateralBelowMinimum);
    }
    if new_collateral > market_config.max_collateral {
        panic_with_error!(e, TradingError::CollateralAboveMaximum);
    }

    // Check minimum leverage
    let min_notional = new_collateral.fixed_mul_ceil(e, &MIN_LEVERAGE, &SCALAR_7);
    if position.notional_size < min_notional {
        panic_with_error!(e, TradingError::LeverageBelowMinimum);
    }

    let token = storage::get_token(e);
    let token_client = TokenClient::new(e, &token);
    let collateral_diff = new_collateral - position.collateral;

    if position.filled {
        // Filled position: need to check margin and update market stats
        let config = storage::get_config(e);
        let mut market = Market::load(e, position.asset_index);
        market.accrue_interest(e);

        if collateral_diff > 0 {
            token_client.transfer(&position.user, &e.current_contract_address(), &collateral_diff);
        } else if collateral_diff < 0 {
            let current_price =
                load_price(e, &config.oracle, &market.config.asset, config.max_price_age);
            let pnl = position.calculate_pnl(e, current_price);
            let fees = position.calculate_fee_breakdown(e, &market);
            let equity = new_collateral + pnl - fees.total_fee();
            let required_margin = position
                .notional_size
                .fixed_mul_floor(e, &market.config.init_margin, &SCALAR_7);

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
        } else if collateral_diff < 0 {
            let required_margin = position
                .notional_size
                .fixed_mul_floor(e, &market_config.init_margin, &SCALAR_7);

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

    // Must be open position
    if !position.filled {
        panic_with_error!(e, TradingError::PositionNotOpen);
    }

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
    let max_allowed_notional = vault_assets.fixed_mul_floor(e, &config.max_utilization, &SCALAR_7);
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
    let min_notional = collateral.fixed_mul_ceil(e, &MIN_LEVERAGE, &SCALAR_7);
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
        notional_size.fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7)
    } else {
        0 // No base fee for balancing trades
    };

    let price_impact_scalar =
        notional_size.fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

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
    use crate::constants::SCALAR_18;
    use crate::testutils::{
        create_oracle, create_token, create_trading, create_vault, default_config,
        default_market, default_market_data, BTC_PRICE,
    };
    use crate::types::{ContractStatus, Position as PositionType};
    use sep_41_token::testutils::MockTokenClient;
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};

    fn setup_env() -> soroban_sdk::Env {
        let e = soroban_sdk::Env::default();
        e.mock_all_auths();
        e.ledger().set(LedgerInfo {
            timestamp: 1000,
            protocol_version: 25,
            sequence_number: 100,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3110400,
        });
        e
    }

    /// Setup full contract state for action tests
    fn setup_contract(e: &soroban_sdk::Env) -> (Address, Address, MockTokenClient) {
        let (contract, owner) = create_trading(e);
        let (oracle, _) = create_oracle(e);
        let (token, token_client) = create_token(e, &owner);
        let (vault, _) = create_vault(e, &token, 100_000_000 * SCALAR_7);

        // Initialize contract state
        e.as_contract(&contract, || {
            use crate::trading::config::execute_initialize;
            use soroban_sdk::String;

            execute_initialize(e, &String::from_str(e, "Test"), &vault, &default_config(&oracle));
            storage::set_status(e, ContractStatus::Active as u32);

            // Set up market
            storage::set_market_config(e, 0, &default_market(e));
            storage::set_market_data(e, 0, &default_market_data());
            storage::next_market_index(e); // Advance counter to 1
        });

        // Mint tokens to contract
        token_client.mint(&contract, &(10_000_000 * SCALAR_7));

        (contract, owner, token_client)
    }

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
    // execute_create_position tests
    // ==========================================

    #[test]
    fn test_create_market_order_long() {
        let e = setup_env();
        let (contract, _, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            let (id, fee) = execute_create_position(
                &e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, true, 0, 0, 0,
            );
            assert_eq!(id, 1);
            assert!(fee > 0);
        });
    }

    #[test]
    fn test_create_limit_order() {
        let e = setup_env();
        let (contract, _, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            let (id, fee) = execute_create_position(
                &e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, true,
                BTC_PRICE + 1000 * SCALAR_7, 0, 0,
            );
            assert_eq!(id, 1);
            assert!(fee > 0);
        });
    }

    #[test]
    fn test_create_short_position() {
        let e = setup_env();
        let (contract, _, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            let (id, _) = execute_create_position(
                &e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, false, 0, 0, 0,
            );
            assert_eq!(id, 1);
            let pos = storage::get_position(&e, 1);
            assert!(!pos.is_long);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_create_position_negative_collateral() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            execute_create_position(&e, &user, 0, -1, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #331)")]
    fn test_create_position_collateral_below_min() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            execute_create_position(&e, &user, 0, SCALAR_7 / 2, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #332)")]
    fn test_create_position_collateral_above_max() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            execute_create_position(&e, &user, 0, 2_000_000 * SCALAR_7, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #334)")]
    fn test_create_position_invalid_entry_price_long() {
        let e = setup_env();
        let (contract, _, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(10_000 * SCALAR_7));

        e.as_contract(&contract, || {
            execute_create_position(
                &e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, true,
                BTC_PRICE - 1000 * SCALAR_7, 0, 0,
            );
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #329)")]
    fn test_create_position_max_positions() {
        let e = setup_env();
        let (contract, _, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        e.as_contract(&contract, || {
            // Fill up max positions (10)
            for i in 0..10 {
                storage::add_user_position(&e, &user, i);
            }
            execute_create_position(&e, &user, 0, 100 * SCALAR_7, 1000 * SCALAR_7, true, 0, 0, 0);
        });
    }

    // ==========================================
    // execute_close_position tests
    // ==========================================

    #[test]
    fn test_close_filled_position() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut data = default_market_data();
            data.long_notional_size = position.notional_size;
            storage::set_market_data(&e, 0, &data);

            let (pnl, fee) = execute_close_position(&e, 1);
            assert_eq!(pnl, 0);
            assert!(fee >= 0);
        });
    }

    #[test]
    fn test_cancel_pending_position() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
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
        let (contract, _, token_client) = setup_contract(&e);
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
        let (contract, _, _) = setup_contract(&e);
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
    #[should_panic(expected = "Error(Contract, #327)")]
    fn test_modify_collateral_not_open() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, false);
            storage::set_position(&e, 1, &position);
            execute_modify_collateral(&e, 1, 150 * SCALAR_7);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_modify_collateral_negative() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            execute_modify_collateral(&e, 1, -1);
        });
    }

    // ==========================================
    // execute_set_triggers tests
    // ==========================================

    #[test]
    fn test_set_triggers_long() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
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
        let (contract, _, _) = setup_contract(&e);
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
        let (contract, _, _) = setup_contract(&e);
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
    #[should_panic(expected = "Error(Contract, #327)")]
    fn test_set_triggers_not_open() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, false);
            storage::set_position(&e, 1, &position);
            execute_set_triggers(&e, 1, BTC_PRICE + 5000 * SCALAR_7, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #340)")]
    fn test_set_triggers_invalid_tp_long() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            execute_set_triggers(&e, 1, BTC_PRICE - 1000 * SCALAR_7, 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #341)")]
    fn test_set_triggers_invalid_sl_long() {
        let e = setup_env();
        let (contract, _, _) = setup_contract(&e);
        let user = Address::generate(&e);

        e.as_contract(&contract, || {
            let position = create_test_position(&e, &user, true);
            storage::set_position(&e, 1, &position);
            execute_set_triggers(&e, 1, 0, BTC_PRICE + 1000 * SCALAR_7);
        });
    }
}
