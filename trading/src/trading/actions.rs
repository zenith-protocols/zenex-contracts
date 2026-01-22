use crate::constants::{MAX_PRICE_AGE, SCALAR_7, STATUS_ACTIVE, STATUS_ON_ICE};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{
    CancelPosition, ClosePosition, DepositCollateral, OpenPosition, SetStopLoss, SetTakeProfit,
    WithdrawCollateral,
};
use crate::storage;
use crate::trading::market::Market;
use crate::trading::position::Position;
use crate::types::PositionStatus;
use sep_40_oracle::PriceFeedClient;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, Address, Env};

/// Load the current price for an asset from the oracle
pub fn load_price(e: &Env, oracle: &Address, asset_index: u32) -> i128 {
    let market_config = storage::get_market_config(e, asset_index);
    let price_data = match PriceFeedClient::new(e, oracle).lastprice(&market_config.asset) {
        Some(price) => price,
        None => panic_with_error!(e, TradingError::PriceNotFound),
    };
    if price_data.timestamp + MAX_PRICE_AGE < e.ledger().timestamp() {
        panic_with_error!(e, TradingError::PriceStale);
    }
    price_data.price
}

/// Check status allows user actions (not Setup or Frozen)
fn require_active_or_on_ice(e: &Env) {
    let status = storage::get_status(e);
    if status != STATUS_ACTIVE && status != STATUS_ON_ICE {
        panic_with_error!(e, TradingError::ContractPaused);
    }
}

/// Check status is Active (for opening new positions)
fn require_active(e: &Env) {
    let status = storage::get_status(e);
    if status != STATUS_ACTIVE {
        panic_with_error!(e, TradingError::ContractPaused);
    }
}

/// Close a position (handles both Open and Pending positions)
/// Pending positions are cancelled, Open positions are closed with PnL settlement
/// Returns (pnl, fee) tuple
pub fn execute_close_position(e: &Env, position_id: u32) -> (i128, i128) {
    let mut position = Position::load(e, position_id);
    position.user.require_auth();
    require_active_or_on_ice(e);

    match position.status {
        PositionStatus::Pending => execute_cancel(e, &mut position),
        PositionStatus::Open => execute_close(e, &mut position),
        PositionStatus::Closed => {
            panic_with_error!(e, TradingError::PositionAlreadyClosed);
        }
    }
}

/// Internal cancel logic for pending positions
/// Refunds collateral + base_fee + price_impact that were charged at open
fn execute_cancel(e: &Env, position: &mut Position) -> (i128, i128) {
    let token = storage::get_token(e);
    let token_client = TokenClient::new(e, &token);
    let mut market = Market::load_checked(e, position.asset_index);
    market.update_borrowing_index(e);

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

    position.status = PositionStatus::Closed;
    storage::remove_user_position(e, &position.user, position.id);
    position.store(e);

    CancelPosition {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id: position.id,
    }
    .publish(e);

    // For cancelled pending orders: no PnL, no fee
    (0, 0)
}

/// Internal close logic for open positions
fn execute_close(e: &Env, position: &mut Position) -> (i128, i128) {
    let config = storage::get_config(e);
    let vault = storage::get_vault(e);
    let token = storage::get_token(e);

    let mut market = Market::load_checked(e, position.asset_index);
    market.update_borrowing_index(e);

    let price = load_price(e, &config.oracle, position.asset_index);
    let calc = position.calculate_close(e, price, 0, &market);

    let token_client = TokenClient::new(e, &token);
    let vault_client = VaultClient::new(e, &vault);

    // Handle vault transfer (negative = vault pays, positive = vault receives)
    if calc.vault_transfer < 0 {
        // Vault pays: withdraw from vault to this contract
        vault_client.strategy_withdraw(&e.current_contract_address(), &(-calc.vault_transfer));
    } else if calc.vault_transfer > 0 {
        // Vault receives: direct transfer from this contract to vault
        token_client.transfer(&e.current_contract_address(), &vault, &calc.vault_transfer);
    }

    // Pay user their payout
    if calc.user_payout > 0 {
        token_client.transfer(&e.current_contract_address(), &position.user, &calc.user_payout);
    }

    // Update market stats
    market.update_stats(
        -position.collateral,
        -position.notional_size,
        position.is_long,
    );

    position.status = PositionStatus::Closed;
    storage::remove_user_position(e, &position.user, position.id);

    ClosePosition {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id: position.id,
        price: calc.price,
        fee: calc.fee,
    }
    .publish(e);

    market.store(e);
    position.store(e);

    (calc.pnl, calc.fee)
}

/// Modify collateral on a position to a new absolute value
pub fn execute_modify_collateral(e: &Env, position_id: u32, new_collateral: i128) {
    let mut position = Position::load(e, position_id);
    position.user.require_auth();
    require_active_or_on_ice(e);

    // Must be open position
    if position.status != PositionStatus::Open {
        panic_with_error!(e, TradingError::PositionNotOpen);
    }

    if new_collateral <= 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    let config = storage::get_config(e);
    let token = storage::get_token(e);

    let mut market = Market::load_checked(e, position.asset_index);
    market.update_borrowing_index(e);
    let current_price = load_price(e, &config.oracle, position.asset_index);

    let token_client = TokenClient::new(e, &token);

    // Calculate the difference between new and current collateral
    let collateral_diff = new_collateral - position.collateral;

    if collateral_diff > 0 {
        // Deposit collateral
        token_client.transfer(&position.user, &e.current_contract_address(), &collateral_diff);
        position.collateral = new_collateral;
        market.update_stats(collateral_diff, 0, position.is_long);

        DepositCollateral {
            asset_index: position.asset_index,
            user: position.user.clone(),
            position_id: position.id,
            amount: collateral_diff,
        }
        .publish(e);
    } else if collateral_diff < 0 {
        // Withdraw collateral
        let withdraw_amount = -collateral_diff;

        let pnl = position.calculate_pnl(e, current_price);
        let equity = new_collateral + pnl;
        let required_margin = position
            .notional_size
            .fixed_mul_floor(e, &market.config.init_margin, &SCALAR_7);

        if equity < required_margin {
            panic_with_error!(e, TradingError::WithdrawalBreaksMargin);
        }

        position.collateral = new_collateral;
        token_client.transfer(&e.current_contract_address(), &position.user, &withdraw_amount);
        market.update_stats(-withdraw_amount, 0, position.is_long);

        WithdrawCollateral {
            asset_index: position.asset_index,
            user: position.user.clone(),
            position_id: position.id,
            amount: withdraw_amount,
        }
        .publish(e);
    }
    // If collateral_diff == 0, no transfer needed

    market.store(e);
    position.store(e);
}

/// Set take profit and stop loss triggers
/// Use 0 to clear/disable a trigger
pub fn execute_set_triggers(e: &Env, position_id: u32, take_profit: i128, stop_loss: i128) {
    let mut position = Position::load(e, position_id);
    position.user.require_auth();
    require_active_or_on_ice(e);

    // Must be open position
    if position.status != PositionStatus::Open {
        panic_with_error!(e, TradingError::PositionNotOpen);
    }

    let oracle = storage::get_config(e).oracle;
    let current_price = load_price(e, &oracle, position.asset_index);

    // Validate and set take profit
    if take_profit > 0 {
        if position.is_long {
            // For long positions, take profit must be above current price
            if take_profit <= current_price {
                panic_with_error!(e, TradingError::InvalidTakeProfitPrice);
            }
        } else {
            // For short positions, take profit must be below current price
            if take_profit >= current_price {
                panic_with_error!(e, TradingError::InvalidTakeProfitPrice);
            }
        }
        position.take_profit = take_profit;

        SetTakeProfit {
            asset_index: position.asset_index,
            user: position.user.clone(),
            position_id: position.id,
            price: take_profit,
        }
        .publish(e);
    } else if take_profit == 0 {
        // Clear take profit
        position.take_profit = 0;
    }

    // Validate and set stop loss
    if stop_loss > 0 {
        if position.is_long {
            // For long positions, stop loss must be below current price
            if stop_loss >= current_price {
                panic_with_error!(e, TradingError::InvalidStopLossPrice);
            }
        } else {
            // For short positions, stop loss must be above current price
            if stop_loss <= current_price {
                panic_with_error!(e, TradingError::InvalidStopLossPrice);
            }
        }
        position.stop_loss = stop_loss;

        SetStopLoss {
            asset_index: position.asset_index,
            user: position.user.clone(),
            position_id: position.id,
            price: stop_loss,
        }
        .publish(e);
    } else if stop_loss == 0 {
        // Clear stop loss
        position.stop_loss = 0;
    }

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
    user.require_auth();
    require_active(e);

    let config = storage::get_config(e);
    let vault = storage::get_vault(e);
    let token = storage::get_token(e);

    let mut market = Market::load_checked(e, asset_index);
    market.update_borrowing_index(e);

    if collateral < 0 || notional_size < 0 || entry_price < 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // Check utilization limit: total_notional must not exceed vault_assets * max_utilization
    if config.max_utilization > 0 {
        let vault_client = VaultClient::new(e, &vault);
        let vault_assets = vault_client.total_assets();
        let current_total_notional =
            market.data.long_notional_size + market.data.short_notional_size;
        let new_total_notional = current_total_notional + notional_size;

        let max_allowed_notional =
            vault_assets.fixed_mul_floor(e, &config.max_utilization, &SCALAR_7);
        if new_total_notional > max_allowed_notional {
            panic_with_error!(e, TradingError::UtilizationLimitExceeded);
        }
    }

    // Check collateral bounds
    if collateral < market.config.min_collateral {
        panic_with_error!(e, TradingError::CollateralBelowMinimum);
    }
    if collateral > market.config.max_collateral {
        panic_with_error!(e, TradingError::CollateralAboveMaximum);
    }

    // Check user position count limit
    let positions = storage::get_user_positions(e, user);
    if positions.len() >= config.max_positions {
        panic_with_error!(e, TradingError::MaxPositionsReached)
    }

    let current_price = load_price(e, &config.oracle, asset_index);
    let market_order = entry_price == 0;
    let status = if market_order {
        PositionStatus::Open
    } else {
        PositionStatus::Pending
    };

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
        market.update_stats(collateral, notional_size, is_long);
    }

    let interest_index = if is_long {
        market.data.long_interest_index
    } else {
        market.data.short_interest_index
    };

    let id = storage::bump_position_id(e);
    let position = Position::new(
        e,
        id,
        user.clone(),
        status,
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

    OpenPosition {
        asset_index,
        user: user.clone(),
        position_id: id,
    }
    .publish(e);

    (id, open_fee + price_impact_scalar)
}
