use sep_40_oracle::Asset;
use soroban_sdk::{Address, Env, Symbol};

use crate::MarketConfig;

pub struct TradingEvents {}

impl TradingEvents {
    /// Emitted when trading configuration is updated
    ///
    /// - topics - `["update_config"]`
    /// - data - `[oracle: Address, caller_take_rate: i128, max_positions: u32]`
    ///
    /// ### Arguments
    /// * oracle - The oracle address
    /// * caller_take_rate - The fee rate for callers
    /// * max_positions - Maximum positions per user
    pub fn set_config(
        e: &Env,
        oracle: Address,
        caller_take_rate: i128,
        max_positions: u32,
    ) {
        let topics = (Symbol::new(e, "set_config"),);
        e.events()
            .publish(topics, (oracle, caller_take_rate, max_positions));
    }

    /// Emitted when a market configuration is queued
    ///
    /// - topics - `["queue_set_market"]`
    /// - data - `[asset: Asset, config: MarketConfig]`
    ///
    /// ### Arguments
    /// * asset - The asset for the market
    /// * config - The market configuration
    pub fn queue_set_market(e: &Env, asset: Asset, config: MarketConfig) {
        let topics = (Symbol::new(e, "queue_set_market"),);
        e.events().publish(topics, (asset, config));
    }

    /// Emitted when a queued market configuration is cancelled
    ///
    /// - topics - `["cancel_set_market"]`
    /// - data - `[asset: Asset]`
    /// ### Arguments
    /// * asset - The asset for the market
    pub fn cancel_set_market(e: &Env, asset: Asset) {
        let topics = (Symbol::new(e, "cancel_set_market"),);
        e.events().publish(topics, asset);
    }

    /// Emitted when a queued market configuration is executed
    ///
    /// - topics - `["set_market"]`
    /// - data - `[asset: Asset]`
    ///
    /// ### Arguments
    /// * asset - The asset for the market
    pub fn set_market(e: &Env, asset: Asset) {
        let topics = (Symbol::new(e, "set_market"),);
        e.events().publish(topics, asset);
    }

    /// Emitted when trading status is updated
    ///
    /// - topics - `["set_status"]`
    /// - data - `status: u32`
    ///
    /// ### Arguments
    /// * owner - The owner setting the status
    /// * status - The new trading status
    pub fn set_status(e: &Env, status: u32) {
        let topics = (Symbol::new(e, "set_status"),);
        e.events().publish(topics, status);
    }

    /// Emitted when a new position is opened
    ///
    /// - topics - `["open_position", user: Address, asset: Asset]`
    /// - data - `[position_id: u32]`
    ///
    /// ### Arguments
    /// * user - The user opening the position
    /// * asset - The asset being traded
    /// * position_id - The ID of the new position
    pub fn open_position(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
    ) {
        let topics = (Symbol::new(e, "open_position"), user, asset);
        e.events().publish(
            topics,
            (position_id),
        );
    }

    /// Emitted when a position is closed (manually by user)
    ///
    /// - topics - `["close_position", user: Address, asset: Asset]`
    /// - data - `[position_id: u32, pnl: i128, fee: i128]`
    ///
    /// ### Arguments
    /// * user - The user closing the position
    /// * asset - The asset being traded
    /// * position_id - The position ID
    /// * pnl - The profit/loss from the position
    /// * fee - The fee charged
    pub fn close_position(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
        pnl: i128,
        fee: i128,
    ) {
        let topics = (Symbol::new(e, "close_position"), user, asset);
        e.events()
            .publish(topics, (position_id, pnl, fee));
    }

    /// Emitted when a limit order is filled
    ///
    /// - topics - `["fill_position", user: Address, asset: Asset]`
    /// - data - `[position_id: u32, fill_price: i128, caller_fee: i128]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * position_id - The position ID
    /// * caller_fee - The fee paid to the caller
    pub fn fill_position(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
    ) {
        let topics = (Symbol::new(e, "fill_position"), user, asset);
        e.events()
            .publish(topics, (position_id));
    }

    /// Emitted when a position is liquidated
    ///
    /// - topics - `["liquidation", user: Address, asset: Address]`
    /// - data - `[position_id: u32]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * position_id - The position ID
    pub fn liquidation(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
    ) {
        let topics = (Symbol::new(e, "liquidation"), user, asset);
        e.events()
            .publish(topics, (position_id));
    }

    /// Emitted when a pending position is cancelled
    ///
    /// - topics - `["cancel_position", user: Address, asset: Asset]`
    /// - data - `[position_id: u32, collateral_returned: i128]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * position_id - The position ID
    /// * collateral_returned - The collateral returned to the user
    pub fn cancel_position(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
    ) {
        let topics = (Symbol::new(e, "cancel_position"), user, asset);
        e.events()
            .publish(topics, (position_id));
    }

    /// Emitted when collateral is withdrawn from a position
    ///
    /// - topics - `["withdraw_collateral", user: Address, asset: Asset]`
    /// - data - `[position_id: u32, amount: i128, remaining_collateral: i128]`
    ///
    /// ### Arguments
    /// * user - The user withdrawing collateral
    /// * asset - The asset being traded
    /// * position_id - The position ID
    /// * amount - The amount withdrawn
    pub fn withdraw_collateral(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
        amount: i128,
    ) {
        let topics = (Symbol::new(e, "withdraw_collateral"), user, asset);
        e.events()
            .publish(topics, (position_id, amount));
    }

    /// Emitted when collateral is deposited to a position
    ///
    /// - topics - `["deposit_collateral", user: Address, asset: Asset]`
    /// - data - `[position_id: u32, amount: i128]`
    ///
    /// ### Arguments
    /// * user - The user depositing collateral
    /// * asset - The asset being traded
    /// * position_id - The position ID
    /// * amount - The amount deposited
    pub fn deposit_collateral(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
        amount: i128,
    ) {
        let topics = (Symbol::new(e, "deposit_collateral"), user, asset);
        e.events()
            .publish(topics, (position_id, amount));
    }

    /// Emitted when take profit is set for a position
    ///
    /// - topics - `["set_take_profit", user: Address, asset: Asset]`
    /// - data - `[position_id: u32, price: i128]`
    ///
    /// ### Arguments
    /// * user - The user setting take profit
    /// * asset - The asset being traded
    /// * position_id - The position ID
    /// * price - The take profit price level
    pub fn set_take_profit(e: &Env, user: Address, asset: Asset, position_id: u32) {
        let topics = (Symbol::new(e, "set_take_profit"), user, asset);
        e.events().publish(topics, (position_id));
    }

    /// Emitted when stop loss is set for a position
    ///
    /// - topics - `["set_stop_loss", user: Address, asset: Asset]`
    /// - data - `[position_id: u32, price: i128]`
    ///
    /// ### Arguments
    /// * user - The user setting stop loss
    /// * asset - The asset being traded
    /// * position_id - The position ID
    /// * price - The stop loss price level
    pub fn set_stop_loss(e: &Env, user: Address, asset: Asset, position_id: u32) {
        let topics = (Symbol::new(e, "set_stop_loss"), user, asset);
        e.events().publish(topics, (position_id));
    }
}
