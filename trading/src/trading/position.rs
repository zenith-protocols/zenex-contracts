use sep_40_oracle::Asset;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{Address, Env, panic_with_error};
use soroban_sdk::token::TokenClient;
use crate::constants::SCALAR_7;
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::storage;
use crate::trading::actions::RequestType;
use crate::trading::trading::Trading;
pub(crate) use crate::types::{Position, PositionStatus};

/// Implementation of position-related methods
impl Position {
    pub fn load(e: &Env, position_id: u32) -> Self {
        storage::get_position(e, position_id)
    }

    pub fn store(&self, e: &Env, position_id: u32) {
        storage::set_position(e, position_id, &self);
    }

    pub fn require_auth(&self) {
        self.user.require_auth();
    }

    /// Check if the requested action is allowed based on this position's status
    ///
    /// Returns true if the action is allowed, false otherwise
    pub fn validate_action(&self, action: &RequestType) -> bool {
        match action {
            RequestType::Close => self.status == PositionStatus::Open,
            RequestType::Fill => self.status == PositionStatus::Pending,
            RequestType::StopLoss => self.status == PositionStatus::Open,
            RequestType::TakeProfit => self.status == PositionStatus::Open,
            RequestType::Liquidation => self.status == PositionStatus::Open,
            RequestType::Cancel => self.status == PositionStatus::Pending,
        }
    }

    /// Set stop loss and take profit levels
    pub fn set_risk_params(&mut self, stop_loss: i128, take_profit: i128) {
        self.stop_loss = stop_loss;
        self.take_profit = take_profit;
    }

    pub fn set_status(&mut self, status: PositionStatus) {
        self.status = status;
    }

    /// Calculate profit/loss for a position
    pub fn calculate_pnl(&self, e: &Env, current_price: i128) -> (i128, i128) {
        let price_diff = if self.is_long {
            current_price - self.entry_price
        } else {
            self.entry_price - current_price
        };

        let pnl = if price_diff == 0 {
            0
        } else {
            // Calculate percentage change
            let percent_change = price_diff.fixed_div_floor(e, &self.entry_price, &SCALAR_7);
            let leveraged_percent = percent_change.fixed_mul_floor(e, &(self.leverage as i128), &100);
            self.collateral.fixed_mul_floor(e, &leveraged_percent, &SCALAR_7)
        };

        // #TODO: Calculate total fee based on market conditions
        let total_fee = 0;
        (pnl, total_fee)
    }

    pub fn check_take_profit(&self, current_price: i128) -> bool {
        if self.take_profit == 0 {
            return false;
        }

        if self.is_long {
            current_price >= self.take_profit
        } else {
            current_price <= self.take_profit
        }
    }

    pub fn check_stop_loss(&self, current_price: i128) -> bool {
        if self.stop_loss == 0 {
            return false;
        }

        if self.is_long {
            current_price <= self.stop_loss
        } else {
            current_price >= self.stop_loss
        }
    }
}

pub fn execute_create_position(
    e: &Env,
    user: &Address,
    asset: &Asset,
    collateral: i128,
    leverage: u32,
    is_long: bool,
    entry_price: i128
) -> u32 {
    user.require_auth();
    let mut trading = Trading::load(e);
    let mut market = trading.load_market(e, asset, true);

    // Validate parameters
    if !market.is_collateral_valid(collateral) || !market.is_leverage_valid(leverage) || entry_price < 0 {
        panic_with_error!(e, TradingError::BadRequest);
    }

    // Check user position count limit
    let positions = storage::get_user_positions(e, user);
    if !trading.check_max_positions(positions) {
        panic_with_error!(e, TradingError::MaxPositions)
    }

    // Transfer tokens from user to contract
    let token_client = TokenClient::new(e, &storage::get_token(e));
    token_client.transfer(user, &e.current_contract_address(), &collateral);
    
    
    let current_price = trading.load_price(e, asset);
    let mut status = PositionStatus::Open;
    let actual_entry_price = if entry_price == 0 {
        current_price
    } else if (is_long && entry_price < current_price) || (!is_long && entry_price > current_price) {
        status = PositionStatus::Pending;
        entry_price
    } else {
        current_price
    };

    // If market order, update market stats immediately
    if status == PositionStatus::Open {
        let size = collateral.fixed_mul_floor(e, &(leverage as i128), &100);
        let borrowed = size - collateral;
        market.update_stats(e, collateral, borrowed, is_long);

        // Update market in storage
        trading.cache_market(&market);
        trading.store_cached_markets(e);
    }

    let position_index = if is_long {
        market.data.long_interest_index
    } else {
        market.data.short_interest_index
    };
    
    let id = storage::bump_position_id(e);

    let position = Position {
        id,
        user: user.clone(),
        status: status.clone(),
        asset: asset.clone(),
        is_long,
        stop_loss: 0,
        take_profit: 0,
        entry_price: actual_entry_price,
        leverage,
        collateral,
        position_index,
        timestamp: e.ledger().timestamp(),
    };
    position.store(e, id);
    storage::add_user_position(e, user, id);

    TradingEvents::open_position(e, user.clone(), asset.clone(), id, collateral, leverage.clone(), is_long, actual_entry_price);
    id
}

pub fn execute_modify_risk(e: &Env, position_id: u32, stop_loss: i128, take_profit: i128) {
    let mut position = Position::load(e, position_id);
    position.require_auth();

    let mut trading = Trading::load(e);
    let current_price = trading.load_price(e, &position.asset);

    // Validate stop_loss if it's set (non-zero)
    if stop_loss != 0 {
        if (position.is_long && stop_loss > current_price) ||
            (!position.is_long && stop_loss < current_price) {
            panic_with_error!(e, TradingError::BadRequest);
        }
    }

    // Validate take_profit if it's set (non-zero)
    if take_profit != 0 {
        if (position.is_long && take_profit < current_price) ||
            (!position.is_long && take_profit > current_price) {
            panic_with_error!(e, TradingError::BadRequest);
        }
    }

    position.set_risk_params(stop_loss, take_profit);
    position.store(e, position_id);
    TradingEvents::modify_risk(e, position.user.clone(), position_id, stop_loss, take_profit);
}