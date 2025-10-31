use crate::constants::{SCALAR_18, SCALAR_7};
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::storage;
use crate::trading::core::Trading;
use crate::trading::position::Position;
use crate::types::PositionStatus;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{contracttype, panic_with_error, Address, Env, Map, Vec};

#[derive(Clone)]
#[contracttype]
pub struct Request {
    pub action: RequestType,
    pub position: u32,
    pub data: Option<i128>,
}

/// The type of request to be made against the pool
#[derive(Clone, PartialEq)]
#[repr(u32)]
#[contracttype]
pub enum RequestType {
    Close = 0,
    Fill = 1,
    StopLoss = 2,
    TakeProfit = 3,
    Liquidation = 4,
    Cancel = 5,
    DepositCollateral = 6,
    WithdrawCollateral = 7,
    SetTakeProfit = 8,
    SetStopLoss = 9,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct SubmitResult {
    pub transfers: Map<Address, i128>,
    pub results: Vec<u32>,
}

impl SubmitResult {
    /// Create an empty set of actions
    pub fn new(e: &Env) -> Self {
        SubmitResult {
            transfers: Map::new(e),
            results: Vec::new(e),
        }
    }

    pub fn add_transfer(&mut self, address: &Address, amount: i128) {
        self.transfers.set(
            address.clone(),
            amount + self.transfers.get(address.clone()).unwrap_or(0),
        );
    }
}

pub fn process_requests(e: &Env, trading: &mut Trading, requests: Vec<Request>) -> SubmitResult {
    let mut result = SubmitResult::new(e);
    let mut authorized_users = Map::<Address, bool>::new(e);

    for request in requests.iter() {
        let mut position = trading.load_position(e, request.position);
        if !position.validate_action(&request.action) {
            // If the action is not valid for this position, we return an error and not panic
            // This can happen when submitting multiple liquidations and one has already been closed
            // in that case we don't want to panic, just return an error acknowledging the invalid action
            result.results.push_back(TradingError::InvalidAction as u32);
            continue;
        }

        let action_result = match request.action {
            RequestType::Close => apply_close(e, &mut result, trading, &mut position),
            RequestType::Fill => apply_fill(e, &mut result, trading, &mut position),
            RequestType::StopLoss => apply_stop_loss(e, &mut result, trading, &mut position),
            RequestType::TakeProfit => apply_take_profit(e, &mut result, trading, &mut position),
            RequestType::Liquidation => apply_liquidation(e, &mut result, trading, &mut position),
            RequestType::Cancel => apply_cancel(e, &mut result, &mut position),

            // On input error we panic
            RequestType::WithdrawCollateral | RequestType::DepositCollateral => {
                let amount = request
                    .data
                    .unwrap_or_else(|| panic_with_error!(e, TradingError::BadRequest));
                apply_update_collateral(e, &mut result, trading, &mut position, amount)
            }
            RequestType::SetTakeProfit => {
                let price = request
                    .data
                    .unwrap_or_else(|| panic_with_error!(e, TradingError::BadRequest));
                apply_set_take_profit(e, trading, &mut position, price)
            }
            RequestType::SetStopLoss => {
                let price = request
                    .data
                    .unwrap_or_else(|| panic_with_error!(e, TradingError::BadRequest));
                apply_set_stop_loss(e, trading, &mut position, price)
            }
        };

        // Check if this action requires authorization
        let requires_auth = matches!(
            request.action,
            RequestType::Close
                | RequestType::Cancel
                | RequestType::WithdrawCollateral
                | RequestType::DepositCollateral
                | RequestType::SetTakeProfit
                | RequestType::SetStopLoss
        );

        // If action succeeded and requires auth, authorize the user (only once per user)
        if action_result == 0
            && requires_auth
            && !authorized_users.contains_key(position.user.clone())
        {
            position.require_auth();
            authorized_users.set(position.user.clone(), true);
        }

        result.results.push_back(action_result);
    }

    trading.store_cached_markets(e);
    trading.store_cached_positions(e);

    result
}

fn handle_close(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    let price = trading.load_price(e, &position.asset);
    let mut market = trading.load_market(e, &position.asset);
    let pnl = position.calculate_pnl(e, price);
    let fee = position.calculate_fee(e, &market);
    let fee_caller = trading.calculate_caller_fee(e, fee).abs(); // Caller fee is always positive

    let net_pnl = if fee >= 0 {
        // Fee is positive, we subtract it from pnl
        pnl - fee
    } else {
        // fee is negative we add the absolute value of fee to pnl but subtract the caller fee
        pnl + fee.abs() - fee_caller
    };

    let payout = position.collateral + net_pnl; // Total payout to user
    if payout > 0 {
        result.add_transfer(&position.user, payout);
    }

    // Handle PnL transfer to/from vault
    // -pnl automatically handles both profit and loss cases:
    // - When pnl > 0 (profit): -pnl < 0, so vault pays out
    // - When pnl < 0 (loss): -pnl > 0, so vault receives
    if pnl != 0 {
        result.add_transfer(&trading.vault, -pnl);
    }

    // Transfer caller fee
    if fee >= 0 && fee_caller > 0 {
        result.add_transfer(&trading.caller, fee_caller);
    }

    if fee < 0 {
        // If the fee is negative, it means we need to transfer from the vault
        result.add_transfer(&trading.vault, fee);
    } else {
        // If the fee is positive, we need to transfer to the vault
        result.add_transfer(&trading.vault, fee - fee_caller);
    }

    storage::remove_user_position(e, &position.user, position.id);
    position.status = PositionStatus::Closed;

    market.update_stats(
        -position.collateral,
        -position.notional_size,
        position.is_long,
    );
    trading.cache_market(&market);
    trading.cache_position(position);

    TradingEvents::close_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        price,
        pnl,
        payout,
        fee,
    );

    0
}

fn apply_close(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    handle_close(e, result, trading, position)
}

fn apply_fill(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);
    //let mut market = trading.load_market(e, &position.asset);

    let can_fill = if position.is_long {
        current_price <= position.entry_price
    } else {
        current_price >= position.entry_price
    };

    if !can_fill {
        return TradingError::BadRequest as u32;
    }

    position.status = PositionStatus::Open;
    position.entry_price = current_price;

    let mut market = trading.load_market(e, &position.asset);
    market.update_stats(
        position.collateral,
        position.notional_size,
        position.is_long,
    );

    // The base fee is in the current contract address, so we need to transfer it to the vault
    // and give the caller their fee
    let base_fee = position
        .collateral
        .fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7);
    let caller_fee = trading.calculate_caller_fee(e, base_fee); // 1% of collateral for now
    result.add_transfer(&trading.caller, caller_fee);
    result.add_transfer(&trading.vault, base_fee - caller_fee);

    trading.cache_market(&market);
    trading.cache_position(position);

    TradingEvents::fill_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        current_price,
        caller_fee,
    );
    0
}

fn apply_stop_loss(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);
    if !position.check_stop_loss(current_price) {
        return TradingError::BadRequest as u32;
    }

    handle_close(e, result, trading, position)
}

fn apply_take_profit(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);
    if !position.check_take_profit(current_price) {
        return TradingError::BadRequest as u32;
    }

    handle_close(e, result, trading, position)
}

fn apply_liquidation(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);
    let mut market = trading.load_market(e, &position.asset);

    let pnl = position.calculate_pnl(e, current_price);
    let fee = position.calculate_fee(e, &market);

    let equity = position.collateral + pnl - fee;
    let required_margin =
        position
            .notional_size
            .fixed_mul_floor(e, &market.config.maintenance_margin, &SCALAR_7);

    if equity >= required_margin {
        return TradingError::BadRequest as u32; // Not eligible for liquidation
    }

    let caller_fee = trading.calculate_caller_fee(e, fee);
    result.add_transfer(&trading.caller, caller_fee);

    let vault_amount = position.collateral - caller_fee;
    result.add_transfer(&trading.vault, vault_amount);

    TradingEvents::liquidation(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        current_price,
    );

    market.update_stats(
        -position.collateral,
        -position.notional_size,
        position.is_long,
    );

    position.status = PositionStatus::Closed;
    // close price is not stored on Position struct; price emitted via event

    storage::remove_user_position(e, &position.user, position.id);
    trading.cache_market(&market);
    trading.cache_position(position);
    0
}

fn apply_cancel(e: &Env, result: &mut SubmitResult, position: &mut Position) -> u32 {
    result.add_transfer(&position.user, position.collateral);

    position.status = PositionStatus::Closed;
    storage::remove_user_position(e, &position.user, position.id);
    TradingEvents::cancel_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
    );
    0
}

fn apply_set_take_profit(
    e: &Env,
    trading: &mut Trading,
    position: &mut Position,
    price: i128,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);

    if position.is_long {
        // For long positions, take profit must be above current price
        if price <= current_price {
            return TradingError::BadRequest as u32;
        }
    } else {
        // For short positions, take profit must be below current price
        if price >= current_price {
            return TradingError::BadRequest as u32;
        }
    }

    position.take_profit = price;

    trading.cache_position(position);

    TradingEvents::set_take_profit(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
    );
    0
}

fn apply_set_stop_loss(
    e: &Env,
    trading: &mut Trading,
    position: &mut Position,
    price: i128,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);

    if position.is_long {
        // For long positions, stop loss must be below current price
        if price >= current_price {
            return TradingError::BadRequest as u32;
        }
    } else {
        // For short positions, stop loss must be above current price
        if price <= current_price {
            return TradingError::BadRequest as u32;
        }
    }

    position.stop_loss = price;

    trading.cache_position(position);

    TradingEvents::set_stop_loss(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
    );
    0
}

fn apply_update_collateral(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
    amount: i128,
) -> u32 {
    if amount == 0 {
        return TradingError::BadRequest as u32;
    }
    let mut market = trading.load_market(e, &position.asset);
    let current_price = trading.load_price(e, &position.asset);

    let index_difference = if position.is_long {
        market.data.long_interest_index - position.interest_index
    } else {
        market.data.short_interest_index - position.interest_index
    };

    let interest_fee = position
        .notional_size
        .fixed_mul_floor(e, &index_difference, &SCALAR_18);

    if interest_fee > 0 {
        position.collateral -= interest_fee;
        market.update_stats(-interest_fee, 0, position.is_long);
        result.add_transfer(&trading.vault, interest_fee);
    } else if interest_fee < 0 {
        position.collateral += interest_fee.abs();
        market.update_stats(interest_fee.abs(), 0, position.is_long);
        result.add_transfer(&trading.vault, -interest_fee);
    }

    //set interest index
    position.interest_index = if position.is_long {
        market.data.long_interest_index
    } else {
        market.data.short_interest_index
    };

    if amount > 0 {
        // Deposit collateral
        position.collateral += amount;
        result.add_transfer(&position.user, -amount);
        market.update_stats(amount, 0, position.is_long);
        TradingEvents::deposit_collateral(
            e,
            position.user.clone(),
            position.asset.clone(),
            position.id,
            amount,
        );
    } else {
        // Withdraw collateral
        let withdraw_amount = -amount;
        if withdraw_amount > position.collateral {
            return TradingError::BadRequest as u32;
        }
        let pnl = position.calculate_pnl(e, current_price);
        let equity = position.collateral - withdraw_amount + pnl - interest_fee;
        let required_margin =
            position
                .notional_size
                .fixed_mul_floor(e, &market.config.init_margin, &SCALAR_7);
        if equity < required_margin {
            return TradingError::BadRequest as u32;
        }
        position.collateral -= withdraw_amount;
        result.add_transfer(&position.user, withdraw_amount);
        market.update_stats(-withdraw_amount, 0, position.is_long);
        TradingEvents::withdraw_collateral(
            e,
            position.user.clone(),
            position.asset.clone(),
            position.id,
            withdraw_amount,
        );
    }
    trading.cache_market(&market);
    trading.cache_position(position);
    0
}
