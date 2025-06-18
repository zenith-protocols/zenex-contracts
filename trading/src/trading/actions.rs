use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{contracttype, panic_with_error, Address, Env, Map, Vec};
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::storage;
use crate::trading::trading::Trading;
use crate::trading::position::Position;
use crate::types::PositionStatus;

#[derive(Clone)]
#[contracttype]
pub struct Request {
    pub action: RequestType,
    pub position: u32,
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
}

impl RequestType {
    /// Convert a u32 to a RequestType
    ///
    /// ### Panics
    /// If the value is not a valid RequestType
    pub fn from_u32(e: &Env, value: u32) -> Self {
        match value {
            0 => RequestType::Close,
            1 => RequestType::Fill,
            2 => RequestType::StopLoss,
            3 => RequestType::TakeProfit,
            4 => RequestType::Liquidation,
            5 => RequestType::Cancel,
            _ => panic_with_error!(e, TradingError::BadRequest),
        }
    }
}

pub struct Actions {
    pub spender_transfer: i128,
    pub vault_transfer: i128,
    pub owner_transfers: Map<Address, i128>,
    pub caller: Address,
}

impl Actions {
    /// Create an empty set of actions
    pub fn new(e: &Env, caller: Address) -> Self {
        Actions {
            spender_transfer: 0,
            vault_transfer: 0,
            owner_transfers: Map::new(e),
            caller,
        }
    }

    /// Add tokens the sender needs to transfer to the pool
    pub fn add_for_spender_transfer(&mut self, amount: i128) {
        self.spender_transfer += amount;
    }

    pub fn add_vault_transfer(&mut self, amount: i128) {
        self.vault_transfer += amount;
    }

    pub fn add_for_owner_transfer(&mut self, owner: &Address, amount: i128) {
        self.owner_transfers.set(owner.clone(), amount + self.owner_transfers.get(owner.clone()).unwrap_or(0));
    }
}

pub fn build_actions_from_request(e: &Env, trading: &mut Trading, requests: Vec<Request>, caller: Address) -> Actions {
    let mut actions = Actions::new(e, caller);
    let mut updated_positions: Map<u32, Position> = Map::new(e);

    for request in requests.iter() {
        let mut position = Position::load(e, request.position);

        // Skip if position already processed in this batch or action not allowed
        // #TODO: Check if double update is possible
        if !position.validate_action(&request.action) || updated_positions.contains_key(request.position) {
            panic_with_error!(e, TradingError::BadRequest);
        }
        match request.action {
            RequestType::Close => {
                apply_close(e, &mut actions, trading, &mut position);
            },
            RequestType::Fill => {
                apply_fill(e, &mut actions, trading, &mut position);
            },
            RequestType::StopLoss => {
                apply_stop_loss(e, &mut actions, trading, &mut position);
            },
            RequestType::TakeProfit => {
                apply_take_profit(e, &mut actions, trading, &mut position);
            },
            RequestType::Liquidation => {
                apply_liquidation(e, &mut actions, trading, &mut position);
            },
            RequestType::Cancel => {
                apply_cancel(e, &mut actions, &mut position);
            },
        }
        updated_positions.set(request.position, position.clone());
    }

    // Store all updated positions
    for (id, pos) in updated_positions.iter() {
        pos.store(e, id);
    }

    // Store updated markets
    trading.store_cached_markets(e);

    actions
}

fn handle_close(
    e: &Env,
    actions: &mut Actions,
    trading: &mut Trading,
    position: &mut Position,
    status: PositionStatus,
) {
    // Get current price
    let price = trading.load_price(e, &position.asset);
    let (pnl, fee) = position.calculate_pnl(e, price);
    let net_pnl = pnl - fee;

    // Emit the appropriate event based on status
    match status {
        PositionStatus::UserClosed => {
            TradingEvents::close_position(
                e,
                position.user.clone(),
                position.asset.clone(),
                position.id,
                pnl,
                fee,
                price,
            );
        },
        PositionStatus::StopLossClosed => {
            TradingEvents::stop_loss_triggered(
                e,
                position.user.clone(),
                position.asset.clone(),
                actions.caller.clone(),
                position.id,
                pnl,
                fee,
                price,
            );
        },
        PositionStatus::TakeProfitClosed => {
            TradingEvents::take_profit_triggered(
                e,
                position.user.clone(),
                position.asset.clone(),
                actions.caller.clone(),
                position.id,
                pnl,
                fee,
                price,
            );
        },
        _ => {} // No event for other statuses
    }

    let payout = if net_pnl < 0 {
        // Loss scenario
        let loss = net_pnl.abs();
        if loss >= position.collateral {
            // Loss exceeds collateral, user gets nothing
            0
        } else {
            // User gets collateral minus loss
            position.collateral - loss
        }
    } else {
        // Profit scenario - add profit to collateral
        position.collateral + net_pnl
    };

    // Calculate spender fee
    let spender_fee = trading.calculate_spender_fee(e, fee);
    actions.add_for_spender_transfer(spender_fee);

    // Calculate net vault transfer (single operation)
    let vault_fee = fee - spender_fee;
    if net_pnl > 0 {
        // Profitable: need to borrow profit from vault
        actions.add_vault_transfer(net_pnl - vault_fee);
    } else if payout < position.collateral {
        // Loss: repay remaining collateral to vault after payout
        let amount_to_repay = position.collateral - payout - vault_fee;
        if amount_to_repay > 0 {
            actions.add_vault_transfer(-amount_to_repay);
        }
    }

    if payout > 0 {
        actions.add_for_owner_transfer(&position.user, payout);
    }

    // Update market data
    let size = position.collateral.fixed_mul_floor(e, &(position.leverage as i128), &100);
    let mut market = trading.load_market(e, &position.asset, true);
    market.update_stats(e, -position.collateral, -(size - position.collateral), position.is_long);
    trading.cache_market(&market);
    storage::remove_user_position(e, &position.user, position.id);

    // Update position status to the specified status
    position.set_status(status);
}

fn apply_close(
    e: &Env,
    actions: &mut Actions,
    trading: &mut Trading,
    position: &mut Position
) {
    position.require_auth();
    handle_close(e, actions, trading, position, PositionStatus::UserClosed);
}

fn apply_fill(
    e: &Env,
    actions: &mut Actions,
    trading: &mut Trading,
    position: &mut Position,
) {
    let current_price = trading.load_price(e, &position.asset);
    let can_fill = if position.is_long {
        current_price <= position.entry_price
    } else {
        current_price >= position.entry_price
    };

    if !can_fill {
        panic_with_error!(e, TradingError::BadRequest);
    }

    position.set_status(PositionStatus::Open);
    position.entry_price = current_price; // Use actual fill price

    let size = position.collateral.fixed_mul_floor(e, &(position.leverage as i128), &100);
    let mut market = trading.load_market(e, &position.asset, true);
    market.update_stats(e, position.collateral, size - position.collateral, position.is_long);
    trading.cache_market(&market);

    let caller_fee = trading.calculate_spender_fee(e, position.collateral / 100); // 1% of collateral for now
    actions.add_for_spender_transfer(caller_fee);

    TradingEvents::fill_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        actions.caller.clone(),
        position.id,
        current_price,
        caller_fee,
    );
}

fn apply_stop_loss(
    e: &Env,
    actions: &mut Actions,
    trading: &mut Trading,
    position: &mut Position
) {
    let current_price = trading.load_price(e, &position.asset);
    if !position.check_stop_loss(current_price) {
        panic_with_error!(e, TradingError::BadRequest);
    }

    handle_close(e, actions, trading, position, PositionStatus::StopLossClosed);
}

fn apply_take_profit(
    e: &Env,
    actions: &mut Actions,
    trading: &mut Trading,
    position: &mut Position
) {
    let current_price = trading.load_price(e, &position.asset);
    if !position.check_take_profit(current_price) {
        panic_with_error!(e, TradingError::BadRequest);
    }

    handle_close(e, actions, trading, position, PositionStatus::TakeProfitClosed);
}

fn apply_liquidation(
    e: &Env,
    actions: &mut Actions,
    trading: &mut Trading,
    position: &mut Position,
) {
    let current_price = trading.load_price(e, &position.asset);
    let market = trading.load_market(e, &position.asset, false);
    let (pnl, fee) = position.calculate_pnl(e, current_price);

    let net_pnl = pnl - fee;
    if !market.can_liquidate(e, position.collateral, net_pnl) {
        panic_with_error!(e, TradingError::BadRequest);
    }

    let spender_fee = trading.calculate_spender_fee(e, fee);
    actions.add_for_spender_transfer(spender_fee);

    let vault_amount = position.collateral - spender_fee;
    actions.add_vault_transfer(vault_amount);

    let loss = if net_pnl < 0 { net_pnl.abs() } else { 0 };
    TradingEvents::liquidation(
        e,
        position.user.clone(),
        position.asset.clone(),
        actions.caller.clone(),
        position.id,
        position.collateral,
        loss,
        spender_fee,
    );

    let size = position.collateral.fixed_mul_floor(e, &(position.leverage as i128), &100);
    let mut market = trading.load_market(e, &position.asset, true);
    market.update_stats(e, -position.collateral, -(size - position.collateral), position.is_long);
    trading.cache_market(&market);
    storage::remove_user_position(e, &position.user, position.id);

    position.set_status(PositionStatus::Liquidated);
}

fn apply_cancel(
    e: &Env,
    actions: &mut Actions,
    position: &mut Position
) {
    position.require_auth();
    actions.add_for_owner_transfer(&position.user, position.collateral);

    position.set_status(PositionStatus::Cancelled);
    storage::remove_user_position(e, &position.user, position.id);
    TradingEvents::cancel_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        position.collateral,
    );

}