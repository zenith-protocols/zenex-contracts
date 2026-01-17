#![allow(clippy::too_many_arguments)]

use crate::trading::ExecuteRequest;
use crate::types::MarketConfig;
use crate::{storage, trading, TradingConfig};
use sep_40_oracle::Asset;
use soroban_sdk::{contract, contractclient, contractimpl, Address, BytesN, Env, String, Vec};
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_macros::{default_impl, only_owner};

#[contract]
pub struct TradingContract;

#[contractclient(name = "TradingClient")]
pub trait Trading {
    /// (Owner only) Initialize the trading contract
    ///
    /// ### Arguments
    /// * `name` - Name of the trading contract
    /// * `vault` - Address of the vault contract
    /// * `config` - Initial trading configuration
    /// ### Panics
    /// If the caller is not the owner
    /// If the contract is already initialized
    /// If the configuration is invalid
    fn initialize(e: Env, name: String, vault: Address, config: TradingConfig);

    /// (Owner only) Queues a configuration update
    ///
    /// ### Arguments
    /// * `config` - New trading configuration
    ///
    /// ### Panics
    /// If the caller is not the owner
    /// If the configuration is invalid
    fn queue_set_config(e: Env, config: TradingConfig);

    /// (Owner only) Cancels a queued configuration update
    ///
    /// ### Panics
    /// If the caller is not the owner
    /// If there is no queued configuration update
    fn cancel_set_config(e: Env);

    /// Update the trading configuration
    ///
    /// ### Panics
    /// If the caller is not the owner or the update is not queued
    fn set_config(e: Env);

    /// (Owner only) Queue setting data for a market
    ///
    /// ### Arguments
    /// * `asset` - The underlying asset to add as a market
    /// * `config` - The MarketConfig for the market
    ///
    /// ### Panics
    /// If the caller is not the owner
    fn queue_set_market(e: Env, asset: Asset, config: MarketConfig);

    /// (Owner only) Cancels a queued market initialization
    ///
    /// ### Arguments
    /// * `asset` - The underlying asset to cancel the market for
    ///
    /// ### Panics
    /// If the caller is not the owner
    /// If the market is not queued for initialization
    fn cancel_set_market(e: Env, asset: Asset);

    /// Executes the queued set of a market
    ///
    /// ### Arguments
    /// * `asset` - The underlying asset to add as a market
    ///
    /// ### Panics
    /// If the market is not queued for initialization
    /// or is already setup
    /// or has invalid metadata
    fn set_market(e: Env, asset: Asset);

    /// (Owner only) Sets the status of the trading contract
    ///
    /// ### Arguments
    /// * `status` - The new status code (0: Active, 1: OnIce, 2: Frozen, 99: Setup)
    ///
    /// ### Panics
    /// If the caller is not the owner
    fn set_status(e: Env, status: u32);

    /// Open a position (long or short)
    ///
    /// # Arguments
    /// * `user` - User address opening position
    /// * `asset` - Asset to trade
    /// * `collateral` - Collateral amount
    /// * `notional_size` - Notional size of the position
    /// * `is_long` - Whether position is long (true) or short (false)
    /// * `entry_price` - Price at which to open position, 0 for market order
    /// * `take_profit` - Take profit price level, 0 if not set
    /// * `stop_loss` - Stop loss price level, 0 if not set
    ///
    /// # Returns
    /// (position_id, fee) tuple
    fn open_position(
        e: Env,
        user: Address,
        asset: Asset,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
        take_profit: i128,
        stop_loss: i128,
    ) -> (u32, i128);

    /// Close a position (handles both open and pending positions)
    ///
    /// For pending positions: cancels and refunds collateral
    /// For open positions: calculates PnL, fees, and settles
    ///
    /// # Arguments
    /// * `position_id` - ID of position to close (requires owner auth)
    ///
    /// # Returns
    /// (pnl, fee) tuple
    fn close_position(e: Env, position_id: u32) -> (i128, i128);

    /// Modify collateral on an open position
    ///
    /// # Arguments
    /// * `position_id` - ID of position to modify (requires owner auth)
    /// * `new_collateral` - New collateral amount for the position
    ///
    /// # Returns
    /// Interest fee settled (positive = paid, negative = received)
    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128) -> i128;

    /// Set take profit and stop loss triggers
    ///
    /// # Arguments
    /// * `position_id` - ID of position (requires owner auth)
    /// * `take_profit` - Take profit price (0 to clear)
    /// * `stop_loss` - Stop loss price (0 to clear)
    fn set_triggers(e: Env, position_id: u32, take_profit: i128, stop_loss: i128);

    /// Execute batch of keeper actions (Fill, StopLoss, TakeProfit, Liquidate)
    ///
    /// This function is permissionless - anyone can call it to trigger these actions.
    /// Callers receive fees for successful keeper actions.
    ///
    /// # Arguments
    /// * `caller` - Address of the keeper executing actions (receives fees)
    /// * `requests` - Vector of execute requests
    ///
    /// # Returns
    /// Vec<u32> with result codes for each action (0 = success, error code otherwise)
    fn execute(e: Env, caller: Address, requests: Vec<ExecuteRequest>) -> Vec<u32>;

    fn upgrade(e: Env, wasm_hash: BytesN<32>);
}

#[contractimpl]
impl TradingContract {
    pub fn __constructor(e: Env, owner: Address) {
        ownable::set_owner(&e, &owner);
    }
}

#[contractimpl]
impl Trading for TradingContract {
    #[only_owner]
    fn initialize(e: Env, name: String, vault: Address, config: TradingConfig) {
        storage::extend_instance(&e);
        trading::execute_initialize(&e, &name, &vault, &config);
    }

    #[only_owner]
    fn queue_set_config(e: Env, config: TradingConfig) {
        storage::extend_instance(&e);
        trading::execute_queue_set_config(&e, &config);
    }

    #[only_owner]
    fn cancel_set_config(e: Env) {
        storage::extend_instance(&e);
        trading::execute_cancel_set_config(&e);
    }

    fn set_config(e: Env) {
        storage::extend_instance(&e);
        trading::execute_set_config(&e);
    }

    #[only_owner]
    fn queue_set_market(e: Env, asset: Asset, config: MarketConfig) {
        storage::extend_instance(&e);
        trading::execute_queue_set_market(&e, &asset, &config);
    }

    #[only_owner]
    fn cancel_set_market(e: Env, asset: Asset) {
        storage::extend_instance(&e);
        trading::execute_cancel_queued_market(&e, &asset);
    }

    fn set_market(e: Env, asset: Asset) {
        storage::extend_instance(&e);
        trading::execute_set_market(&e, &asset);
    }

    #[only_owner]
    fn set_status(e: Env, status: u32) {
        storage::extend_instance(&e);
        storage::set_status(&e, status);
    }

    fn open_position(
        e: Env,
        user: Address,
        asset: Asset,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
        take_profit: i128,
        stop_loss: i128,
    ) -> (u32, i128) {
        storage::extend_instance(&e);
        trading::execute_create_position(
            &e,
            &user,
            &asset,
            collateral,
            notional_size,
            is_long,
            entry_price,
            take_profit,
            stop_loss,
        )
    }

    fn close_position(e: Env, position_id: u32) -> (i128, i128) {
        storage::extend_instance(&e);
        trading::execute_close_position(&e, position_id)
    }

    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128) -> i128 {
        storage::extend_instance(&e);
        trading::execute_modify_collateral(&e, position_id, new_collateral)
    }

    fn set_triggers(e: Env, position_id: u32, take_profit: i128, stop_loss: i128) {
        storage::extend_instance(&e);
        trading::execute_set_triggers(&e, position_id, take_profit, stop_loss);
    }

    fn execute(e: Env, caller: Address, requests: Vec<ExecuteRequest>) -> Vec<u32> {
        storage::extend_instance(&e);
        trading::execute_trigger(&e, &caller, requests)
    }

    #[only_owner]
    fn upgrade(e: Env, wasm_hash: BytesN<32>) {
        storage::extend_instance(&e);
        e.deployer().update_current_contract_wasm(wasm_hash.clone());
    }
}

#[default_impl]
#[contractimpl]
impl Ownable for TradingContract {}
