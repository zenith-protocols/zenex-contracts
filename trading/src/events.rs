use sep_40_oracle::Asset;
use soroban_sdk::{Address, Env, Symbol};

use crate::{MarketConfig};

pub struct TradingEvents {}

impl TradingEvents {
    /// Emitted when a new admin is proposed
    ///
    /// - topics - `["propose_admin", current_admin: Address]`
    /// - data - `proposed_admin: Address`
    ///
    /// ### Arguments
    /// * current_admin - The current admin proposing the change
    /// * proposed_admin - The proposed new admin
    pub fn propose_admin(e: &Env, current_admin: Address, proposed_admin: Address) {
        let topics = (Symbol::new(&e, "propose_admin"), current_admin);
        e.events().publish(topics, proposed_admin);
    }

    /// Emitted when a proposed admin accepts the role
    ///
    /// - topics - `["accept_admin"]`
    /// - data - `previous_admin: Address`
    ///
    /// ### Arguments
    /// * new_admin - The new admin accepting the role
    pub fn accept_admin(e: &Env, new_admin: Address) {
        let topics = (Symbol::new(&e, "accept_admin"),);
        e.events().publish(topics, new_admin);
    }

    /// Emitted when the vault address is set
    ///
    /// - topics - `["set_vault", admin: Address]`
    /// - data - `vault: Address`
    ///
    /// ### Arguments
    /// * admin - The admin setting the vault
    /// * vault - The vault address
    /// * token - The token address associated with the vault
    pub fn set_vault(e: &Env, admin: Address, vault: Address, token: Address) {
        let topics = (Symbol::new(&e, "set_vault"), admin);
        e.events().publish(topics, (vault, token));
    }

    /// Emitted when trading configuration is updated
    ///
    /// - topics - `["update_config", admin: Address]`
    /// - data - `[oracle: Address, caller_take_rate: i128, max_positions: u32]`
    ///
    /// ### Arguments
    /// * admin - The admin updating the config
    /// * oracle - The oracle address
    /// * caller_take_rate - The fee rate for callers
    /// * max_positions - Maximum positions per user
    pub fn update_config(
        e: &Env,
        admin: Address,
        oracle: Address,
        caller_take_rate: i128,
        max_positions: u32,
    ) {
        let topics = (Symbol::new(&e, "update_config"), admin);
        e.events()
            .publish(topics, (oracle, caller_take_rate, max_positions));
    }

    /// Emitted when a market configuration is queued
    ///
    /// - topics - `["queue_set_market", admin: Address]`
    /// - data - `[asset: Asset, config: MarketConfig]`
    ///
    /// ### Arguments
    /// * admin - The admin queuing the market
    /// * asset - The asset for the market
    /// * config - The market configuration
    pub fn queue_set_market(e: &Env, admin: Address, asset: Asset, config: MarketConfig) {
        let topics = (Symbol::new(&e, "queue_set_market"), admin);
        e.events().publish(topics, (asset, config));
    }

    /// Emitted when a queued market configuration is executed
    ///
    /// - topics - `["set_market"]`
    /// - data - `[asset: Asset]`
    ///
    /// ### Arguments
    /// * asset - The asset for the market
    pub fn set_market(e: &Env, asset: Asset) {
        let topics = (Symbol::new(&e, "set_market"),);
        e.events().publish(topics, asset);
    }

    /// Emitted when trading status is updated
    ///
    /// - topics - `["set_status", admin: Address]`
    /// - data - `status: u32`
    ///
    /// ### Arguments
    /// * admin - The admin setting the status
    /// * status - The new trading status
    pub fn set_status(e: &Env, admin: Address, status: u32) {
        let topics = (Symbol::new(&e, "set_status"), admin);
        e.events().publish(topics, status);
    }

    /// Emitted when a new position is opened
    ///
    /// - topics - `["open_position", user: Address, asset: Asset]`
    /// - data - `[position_id: u32, collateral: i128, leverage: u32, is_long: bool, entry_price: i128]`
    ///
    /// ### Arguments
    /// * user - The user opening the position
    /// * asset - The asset being traded
    /// * position_id - The ID of the new position
    /// * collateral - The collateral amount
    /// * leverage - The leverage multiplier
    /// * is_long - Whether it's a long position
    /// * entry_price - The entry price (0 for market order)
    pub fn open_position(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
        collateral: i128,
        leverage: u32,
        is_long: bool,
        entry_price: i128,
    ) {
        let topics = (Symbol::new(&e, "open_position"), user, asset);
        e.events()
            .publish(topics, (position_id, collateral, leverage, is_long, entry_price));
    }

    /// Emitted when position risk parameters are modified
    ///
    /// - topics - `["modify_risk", user: Address]`
    /// - data - `[position_id: u32, stop_loss: i128, take_profit: i128]`
    ///
    /// ### Arguments
    /// * user - The user modifying the position
    /// * position_id - The position ID
    /// * stop_loss - The stop loss price (0 to keep, -1 to remove)
    /// * take_profit - The take profit price (0 to keep, -1 to remove)
    pub fn modify_risk(
        e: &Env,
        user: Address,
        position_id: u32,
        stop_loss: i128,
        take_profit: i128,
    ) {
        let topics = (Symbol::new(&e, "modify_risk"), user);
        e.events()
            .publish(topics, (position_id, stop_loss, take_profit));
    }

    /// Emitted when a position is closed (manually by user)
    ///
    /// - topics - `["close_position", user: Address, asset: Asset]`
    /// - data - `[position_id: u32, pnl: i128, fee: i128, exit_price: i128]`
    ///
    /// ### Arguments
    /// * user - The user closing the position
    /// * asset - The traded asset
    /// * position_id - The position ID
    /// * pnl - The profit/loss
    /// * fee - The fee charged
    /// * exit_price - The exit price
    pub fn close_position(
        e: &Env,
        user: Address,
        asset: Asset,
        position_id: u32,
        pnl: i128,
        fee: i128,
        exit_price: i128,
    ) {
        let topics = (Symbol::new(&e, "close_position"), user, asset);
        e.events()
            .publish(topics, (position_id, pnl, fee, exit_price));
    }

    /// Emitted when a limit order is filled
    ///
    /// - topics - `["fill_position", user: Address, asset: Asset, caller: Address]`
    /// - data - `[position_id: u32, fill_price: i128, caller_fee: i128]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * caller - The caller who triggered the fill
    /// * position_id - The position ID
    /// * fill_price - The actual fill price
    /// * caller_fee - The fee paid to the caller
    pub fn fill_position(
        e: &Env,
        user: Address,
        asset: Asset,
        caller: Address,
        position_id: u32,
        fill_price: i128,
        caller_fee: i128,
    ) {
        let topics = (Symbol::new(&e, "fill_position"), user, asset, caller);
        e.events()
            .publish(topics, (position_id, fill_price, caller_fee));
    }

    /// Emitted when a stop loss is triggered
    ///
    /// - topics - `["stop_loss_triggered", user: Address, asset: Asset, caller: Address]`
    /// - data - `[position_id: u32, pnl: i128, fee: i128, exit_price: i128]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * caller - The caller who triggered the stop loss
    /// * position_id - The position ID
    /// * pnl - The profit/loss
    /// * fee - The fee charged
    /// * exit_price - The exit price
    pub fn stop_loss_triggered(
        e: &Env,
        user: Address,
        asset: Asset,
        caller: Address,
        position_id: u32,
        pnl: i128,
        fee: i128,
        exit_price: i128,
    ) {
        let topics = (Symbol::new(&e, "stop_loss_triggered"), user, asset, caller);
        e.events()
            .publish(topics, (position_id, pnl, fee, exit_price));
    }

    /// Emitted when a take profit is triggered
    ///
    /// - topics - `["take_profit_triggered", user: Address, asset: Asset, caller: Address]`
    /// - data - `[position_id: u32, pnl: i128, fee: i128, exit_price: i128]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * caller - The caller who triggered the take profit
    /// * position_id - The position ID
    /// * pnl - The profit/loss
    /// * fee - The fee charged
    /// * exit_price - The exit price
    pub fn take_profit_triggered(
        e: &Env,
        user: Address,
        asset: Asset,
        caller: Address,
        position_id: u32,
        pnl: i128,
        fee: i128,
        exit_price: i128,
    ) {
        let topics = (Symbol::new(&e, "take_profit_triggered"), user, asset, caller);
        e.events()
            .publish(topics, (position_id, pnl, fee, exit_price));
    }

    /// Emitted when a position is liquidated
    ///
    /// - topics - `["liquidation", user: Address, asset: Address, liquidator: Address]`
    /// - data - `[position_id: u32, collateral: i128, loss: i128, liquidator_fee: i128]`
    ///
    /// ### Arguments
    /// * user - The position owner
    /// * asset - The traded asset
    /// * liquidator - The liquidator address
    /// * position_id - The position ID
    /// * collateral - The collateral amount
    /// * loss - The total loss
    /// * liquidator_fee - The fee paid to the liquidator
    pub fn liquidation(
        e: &Env,
        user: Address,
        asset: Asset,
        liquidator: Address,
        position_id: u32,
        collateral: i128,
        loss: i128,
        liquidator_fee: i128,
    ) {
        let topics = (Symbol::new(&e, "liquidation"), user, asset, liquidator);
        e.events()
            .publish(topics, (position_id, collateral, loss, liquidator_fee));
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
        collateral_returned: i128,
    ) {
        let topics = (Symbol::new(&e, "cancel_position"), user, asset);
        e.events()
            .publish(topics, (position_id, collateral_returned));
    }

    /// Emitted when the contract is upgraded
    ///
    /// - topics - `["upgrade_wasm", admin: Address]`
    /// - data - `wasm_hash: BytesN<32>`
    ///
    /// ### Arguments
    /// * admin - The admin performing the upgrade
    /// * wasm_hash - The new WASM hash
    pub fn upgrade_wasm(e: &Env, admin: Address, wasm_hash: soroban_sdk::BytesN<32>) {
        let topics = (Symbol::new(&e, "upgrade_wasm"), admin);
        e.events().publish(topics, wasm_hash);
    }
}