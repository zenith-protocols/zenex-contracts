use sep_40_oracle::Asset;
use soroban_sdk::{Address, Env, Symbol};

use crate::MarketConfig;

pub struct TradingEvents {}

impl TradingEvents {
    /// Emitted when trading configuration is updated
    ///
    /// - topics - `["set_config"]`
    /// - data - `[oracle: Address, caller_take_rate: i128, max_positions: u32]`
    ///
    /// ### Arguments
    /// * oracle - The oracle address
    /// * caller_take_rate - The fee rate for callers
    /// * max_positions - Maximum positions per user
    pub fn set_config(e: &Env, oracle: Address, caller_take_rate: i128, max_positions: u32) {
        let topics = (Symbol::new(e, "set_config"),);
        e.events()
            .publish(topics, (oracle, caller_take_rate, max_positions));
    }

    /// Emitted when a configuration update is queued
    ///
    /// - topics - ["queue_set_config"]
    /// - data - [oracle: Address, caller_take_rate: i128, max_positions: u32, unlock_time: u64]
    pub fn queue_set_config(
        e: &Env,
        oracle: Address,
        caller_take_rate: i128,
        max_positions: u32,
        unlock_time: u64,
    ) {
        let topics = (Symbol::new(e, "queue_set_config"),);
        e.events().publish(
            topics,
            (oracle, caller_take_rate, max_positions, unlock_time),
        );
    }

    /// Emitted when a queued configuration update is cancelled
    ///
    /// - topics - ["cancel_set_config"]
    /// - data - `()`
    pub fn cancel_set_config(e: &Env) {
        let topics = (Symbol::new(e, "cancel_set_config"),);
        e.events().publish(topics, ());
    }

    /// Emitted when a market configuration is queued
    ///
    /// - topics - `["queue_set_market", asset: Asset]`
    /// - data - `config: MarketConfig`
    ///
    /// ### Arguments
    /// * asset - The asset for the market
    /// * config - The market configuration
    pub fn queue_set_market(e: &Env, asset: Asset, config: MarketConfig) {
        let topics = (Symbol::new(e, "queue_set_market"), asset);
        e.events().publish(topics, config);
    }

    /// Emitted when a queued market configuration is cancelled
    ///
    /// - topics - `["cancel_set_market", asset: Asset]`
    /// - data - `()`
    /// ### Arguments
    /// * asset - The asset for the market
    pub fn cancel_set_market(e: &Env, asset: Asset) {
        let topics = (Symbol::new(e, "cancel_set_market"), asset);
        e.events().publish(topics, ());
    }

    /// Emitted when a queued market configuration is executed
    ///
    /// - topics - `["set_market", asset: Asset]`
    /// - data - `()`
    ///
    /// ### Arguments
    /// * asset - The asset for the market
    pub fn set_market(e: &Env, asset: Asset) {
        let topics = (Symbol::new(e, "set_market"), asset);
        e.events().publish(topics, ());
    }

    /// Emitted when trading status is updated
    ///
    /// - topics - `["set_status"]`
    /// - data - `status: u32`
    ///
    /// ### Arguments
    /// * status - The new trading status
    pub fn set_status(e: &Env, status: u32) {
        let topics = (Symbol::new(e, "set_status"),);
        e.events().publish(topics, status);
    }

    /// Emitted when a new position is opened
    ///
    /// - topics - `["open_position", asset: Asset, user: Address]`
    /// - data - `[position_id: u32]`
    ///
    /// ### Arguments
    /// * user - The user opening the position
    /// * asset - The asset being traded
    /// * position_id - The ID of the new position
    pub fn open_position(e: &Env, user: Address, asset: Asset, position_id: u32) {
        let topics = (Symbol::new(e, "open_position"), asset, user);
        e.events().publish(topics, (position_id,));
    }

    /// Emitted when a position is closed
    ///
    /// - topics - `["close_position", asset: Asset, user: Address]`
    /// - data - `[position_id: u32, price: i128, fee: i128]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * position_id - The position ID
    /// * price - The closing price
    /// * fee - The protocol fee at close (includes base fee, price impact, and interest)
    pub fn close_position(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
        price: i128,
        fee: i128,
    ) {
        let topics = (Symbol::new(e, "close_position"), asset, user);
        e.events()
            .publish(topics, (position_id, price, fee));
    }

    /// Emitted when a limit order is filled
    ///
    /// - topics - `["fill_position", asset: Asset, user: Address]`
    /// - data - `[position_id: u32]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * position_id - The position ID
    pub fn fill_position(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
    ) {
        let topics = (Symbol::new(e, "fill_position"), asset, user);
        e.events().publish(topics, (position_id,));
    }

    /// Emitted when a position is liquidated
    ///
    /// - topics - `["liquidation", asset: Asset, user: Address]`
    /// - data - `[position_id: u32, price: i128, fee: i128]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * position_id - The position ID
    /// * price - The liquidation price
    /// * fee - The fee charged (includes base fee, price impact, and interest)
    pub fn liquidation(e: &Env, user: Address, asset: Asset, position_id: u32, price: i128, fee: i128) {
        let topics = (Symbol::new(e, "liquidation"), asset, user);
        e.events().publish(topics, (position_id, price, fee));
    }

    /// Emitted when a position's take profit is hit
    ///
    /// - topics - `["take_profit", asset: Asset, user: Address]`
    /// - data - `[position_id: u32, price: i128, fee: i128]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * position_id - The position ID
    /// * price - The price when take profit was triggered
    /// * fee - The fee charged (includes base fee, price impact, and interest)
    pub fn take_profit(e: &Env, user: Address, asset: Asset, position_id: u32, price: i128, fee: i128) {
        let topics = (Symbol::new(e, "take_profit"), asset, user);
        e.events().publish(topics, (position_id, price, fee));
    }

    /// Emitted when a position's stop loss is hit
    ///
    /// - topics - `["stop_loss", asset: Asset, user: Address]`
    /// - data - `[position_id: u32, price: i128, fee: i128]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * position_id - The position ID
    /// * price - The price when stop loss was triggered
    /// * fee - The fee charged (includes base fee, price impact, and interest)
    pub fn stop_loss(e: &Env, user: Address, asset: Asset, position_id: u32, price: i128, fee: i128) {
        let topics = (Symbol::new(e, "stop_loss"), asset, user);
        e.events().publish(topics, (position_id, price, fee));
    }

    /// Emitted when a pending position is cancelled
    ///
    /// - topics - `["cancel_position", asset: Asset, user: Address]`
    /// - data - `[position_id: u32]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * position_id - The position ID
    pub fn cancel_position(e: &Env, user: Address, asset: Asset, position_id: u32) {
        let topics = (Symbol::new(e, "cancel_position"), asset, user);
        e.events().publish(topics, (position_id,));
    }

    /// Emitted when collateral is withdrawn from a position
    ///
    /// - topics - `["withdraw_collateral", asset: Asset, user: Address]`
    /// - data - `[position_id: u32, amount: i128]`
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
        let topics = (Symbol::new(e, "withdraw_collateral"), asset, user);
        e.events().publish(topics, (position_id, amount));
    }

    /// Emitted when collateral is deposited to a position
    ///
    /// - topics - `["deposit_collateral", asset: Asset, user: Address]`
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
        let topics = (Symbol::new(e, "deposit_collateral"), asset, user);
        e.events().publish(topics, (position_id, amount));
    }

    /// Emitted when take profit is set for a position
    ///
    /// - topics - `["set_take_profit", asset: Asset, user: Address]`
    /// - data - `[position_id: u32, price: i128]`
    ///
    /// ### Arguments
    /// * user - The user setting take profit
    /// * asset - The asset being traded
    /// * position_id - The position ID
    /// * price - The take profit price level
    pub fn set_take_profit(e: &Env, user: Address, asset: Asset, position_id: u32, price: i128) {
        let topics = (Symbol::new(e, "set_take_profit"), asset, user);
        e.events().publish(topics, (position_id, price));
    }

    /// Emitted when stop loss is set for a position
    ///
    /// - topics - `["set_stop_loss", asset: Asset, user: Address]`
    /// - data - `[position_id: u32, price: i128]`
    ///
    /// ### Arguments
    /// * user - The user setting stop loss
    /// * asset - The asset being traded
    /// * position_id - The position ID
    /// * price - The stop loss price level
    pub fn set_stop_loss(e: &Env, user: Address, asset: Asset, position_id: u32, price: i128) {
        let topics = (Symbol::new(e, "set_stop_loss"), asset, user);
        e.events().publish(topics, (position_id, price));
    }
}