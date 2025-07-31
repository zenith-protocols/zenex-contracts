use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::storage;
use crate::trading::position::Position;
use crate::trading::trading::Trading;
use crate::types::PositionStatus;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{contracttype, panic_with_error, Address, Env, Map, Vec};
use soroban_sdk::testutils::arbitrary::std::println;
use crate::constants::SCALAR_7;

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
            // In the below cases we do panic with error
            // This is an input error and should not happen
            RequestType::WithdrawCollateral => {
                let amount = request
                    .data
                    .unwrap_or_else(|| panic_with_error!(e, TradingError::BadRequest));
                apply_withdraw_collateral(e, &mut result, trading, &mut position, amount)
            }
            RequestType::DepositCollateral => {
                let amount = request
                    .data
                    .unwrap_or_else(|| panic_with_error!(e, TradingError::BadRequest));
                apply_deposit_collateral(e, &mut result, trading, &mut position, amount)
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

    println!("[Trading] Closing position {}: pnl={}, fee={}, fee_caller={}", position.id, pnl, fee, fee_caller);

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

    if pnl > 0 {
        result.add_transfer(&trading.vault, -pnl);
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
    position.close_price = price;

    market.update_stats(
        e,
        -position.collateral,
        -position.notional_size,
        position.is_long,
    );
    trading.cache_market(&market);
    trading.cache_position(&position);

    0
}

fn apply_close(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    position.require_auth();
    handle_close(e, result, trading, position)
}

fn apply_fill(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);
    let mut market = trading.load_market(e, &position.asset);

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
        e,
        position.collateral,
        position.notional_size,
        position.is_long,
    );

    // The base fee is in the current contract address, so we need to transfer it to the vault
    // and give the caller their fee
    let base_fee = position.collateral.fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7);
    let caller_fee = trading.calculate_caller_fee(e, base_fee); // 1% of collateral for now
    result.add_transfer(&trading.caller, caller_fee);
    result.add_transfer(&trading.vault, base_fee - caller_fee);

    trading.cache_market(&market);
    trading.cache_position(&position);

    TradingEvents::fill_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
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
    let required_margin = position.notional_size.fixed_mul_floor(e, &market.config.maintenance_margin, &SCALAR_7);

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
    );

    market.update_stats(
        e,
        -position.collateral,
        -position.notional_size,
        position.is_long,
    );

    position.status = PositionStatus::Closed;
    position.close_price = current_price;

    storage::remove_user_position(e, &position.user, position.id);
    trading.cache_market(&market);
    trading.cache_position(&position);
    0
}

fn apply_cancel(e: &Env, result: &mut SubmitResult, position: &mut Position) -> u32 {
    position.require_auth();
    result.add_transfer(&position.user, position.collateral);

    position.status = PositionStatus::Closed;
    storage::remove_user_position(e, &position.user, position.id);
    TradingEvents::cancel_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id
    );
    0
}

fn apply_set_take_profit(
    e: &Env,
    trading: &mut Trading,
    position: &mut Position,
    price: i128,
) -> u32 {
    position.require_auth();
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
    position.require_auth();

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

    TradingEvents::set_stop_loss(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
    );
    0
}

fn apply_withdraw_collateral(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
    amount: i128,
) -> u32 {
    position.require_auth();
    if amount <= 0 {
        return TradingError::BadRequest as u32;
    }
    if amount > position.collateral {
        return TradingError::BadRequest as u32;
    }

    let current_price = trading.load_price(e, &position.asset);
    let mut market = trading.load_market(e, &position.asset);

    let pnl = position.calculate_pnl(e, current_price);
    let fee = position.calculate_fee(e, &market);

    // Substract the withdrawal amount from collateral
    let equity = position.collateral - amount + pnl - fee;
    let required_margin = position.notional_size.fixed_mul_floor(e, &market.config.init_margin, &SCALAR_7);

    if equity < required_margin {
        return TradingError::BadRequest as u32; // Not eligible for liquidation
    }

    position.collateral -= amount;
    result.add_transfer(&position.user, amount);

    market.update_stats(
        e,
        -amount,
        0, // No change in notional size
        position.is_long,
    );
    trading.cache_market(&market);
    trading.cache_position(&position);
    TradingEvents::withdraw_collateral(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        amount,
    );
    0
}

fn apply_deposit_collateral(
    e: &Env,
    result: &mut SubmitResult,
    trading: &mut Trading,
    position: &mut Position,
    amount: i128,
) -> u32 {
    position.require_auth();
    let mut market = trading.load_market(e, &position.asset);
    if amount <= 0 {
        return TradingError::BadRequest as u32;
    }

    //TODO: Adjust interest index since deposit amount changes.

    position.collateral += amount;
    result.add_transfer(&position.user, -amount);

    market.update_stats(
        e,
        amount,
        0, // No change in notional size
        position.is_long,
    );
    trading.cache_market(&market);
    trading.cache_position(&position);

    TradingEvents::deposit_collateral(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        amount,
    );
    0
}
