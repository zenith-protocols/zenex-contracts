use sep_40_oracle::Asset;
use soroban_sdk::{contract, contractclient, contractimpl, Address, Env, Vec, BytesN, String};
use crate::{storage, trading};
use crate::events::TradingEvents;
use crate::trading::Request;
use crate::types::MarketConfig;

#[contract]
pub struct TradingContract;

#[contractclient(name = "TradingClient")]
pub trait Trading {
    /// (Admin only) Set a new address to become the admin of the pool. This
    /// must be accepted by the new admin w/ `accept_admin` to take effect.
    ///
    /// ### Arguments
    /// * `new_admin` - The new admin address
    ///
    /// ### Panics
    /// If the caller is not the admin
    fn propose_admin(e: Env, new_admin: Address);

    /// (Proposed admin only) Accept the admin role. Ensures the new admin
    /// can safely submit transactions before taking over the pool admin role.
    ///
    /// ### Panics
    /// If the caller is not the proposed admin
    fn accept_admin(e: Env);

    /// (Admin only) Set the vault address for the trading contract
    /// can only be called during initialization when status is 0.
    ///
    /// ### Arguments
    /// * `vault` - The vault address
    ///
    /// ### Panics
    /// If the caller is not the admin
    fn set_vault(e: Env, vault: Address);

    /// (Admin only) update the trading configuration
    ///
    /// ### Arguments
    /// * `oracle` - The oracle address
    /// * `caller_take_rate` - The take rate for the caller
    /// * `max_positions` - The maximum number of positions
    ///
    /// ### Panics
    /// If the caller is not the admin
    fn update_config(e: Env, oracle: Address, caller_take_rate: i128, max_positions: u32);

    /// (Admin only) Queues setting data for a market
    ///
    /// ### Arguments
    /// * `asset` - The underlying asset to add as a market
    /// * `config` - The MarketConfig for the market
    ///
    /// ### Panics
    /// If the caller is not the admin
    fn queue_set_market(e: Env, asset: Asset, config: MarketConfig);

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

    /// (Admin only) Sets the status of the trading contract
    ///
    /// ### Arguments
    /// * `status` - The new status code (0: Normal, 1: Paused, etc.)
    ///
    /// ### Panics
    /// If the caller is not the admin
    fn set_status(e: Env, status: u32);

    /// Opens a position (long or short)
    ///
    /// # Arguments
    /// * `user` - User address opening position
    /// * `asset` - Asset to trade
    /// * `collateral` - Collateral amount
    /// * `leverage` - Leverage multiplier
    /// * `is_long` - Whether position is long (true) or short (false)
    /// * `entry_price` - Price at which to open position 0 for market order
    ///
    /// # Returns
    /// Position ID of the newly opened position
    fn open_position(
        e: Env,
        user: Address,
        asset: Asset,
        collateral: i128,
        leverage: u32,
        is_long: bool,
        entry_price: i128,
    ) -> u32;

    /// Modifies position risk parameters (stop loss and/or take profit)
    ///
    /// # Arguments
    /// * `position_id` - Position ID
    /// * `stop_loss` - Stop loss price level (0 to keep current value, -1 to remove)
    /// * `take_profit` - Take profit price level (0 to keep current value, -1 to remove)
    fn modify_risk(e: Env, position_id: u32, stop_loss: i128, take_profit: i128);

    /// Executes a batch of trading actions
    ///
    /// # Arguments
    /// * `caller` - Address of the caller executing the actions
    /// * `request` - Vector of requests to process
    ///
    /// # Returns
    /// Amount of fees earned by the caller
    fn submit(e: Env, caller: Address, request: Vec<Request>) -> i128;

    /// (Admin only) Upgrade the contract to a new WASM binary
    ///
    /// This function allows the contract admin to update the contract's code while
    /// preserving its state. The upgrade is performed by providing the hash of a
    /// pre-deployed WASM binary.
    ///
    /// ### Arguments
    /// * `wasm_hash` - The hash of the new WASM binary
    ///
    /// ### Panics
    /// If the caller is not the admin
    fn upgrade_wasm(e: Env, wasm_hash: BytesN<32>);
}

#[contractimpl]
impl TradingContract {
    /// Constructor for initializing the contract when deployed
    pub fn __constructor(e: Env, name: String, admin: Address, oracle: Address, caller_take_rate: i128, max_positions: u32) {
        admin.require_auth();
        trading::execute_initialize(&e, &name, &admin, &oracle, caller_take_rate, max_positions);
    }
}

#[contractimpl]
impl Trading for TradingContract {
    fn propose_admin(e: Env, new_admin: Address) {
        storage::extend_instance(&e);
        let admin = storage::get_admin(&e);
        admin.require_auth();

        storage::set_proposed_admin(&e, &new_admin);
        TradingEvents::propose_admin(&e, admin.clone(), new_admin.clone());
    }

    fn accept_admin(e: Env) {
        storage::extend_instance(&e);
        let proposed_admin = storage::get_proposed_admin(&e).unwrap();
        proposed_admin.require_auth();
        storage::set_admin(&e, &proposed_admin);
        TradingEvents::accept_admin(&e, proposed_admin.clone());
    }

    fn set_vault(e: Env, vault: Address) {
        storage::extend_instance(&e);
        let admin = storage::get_admin(&e);
        admin.require_auth();

        trading::execute_set_vault(&e, &admin, &vault);
    }

    fn update_config(e: Env, oracle: Address, caller_take_rate: i128, max_positions: u32) {
        storage::extend_instance(&e);
        let admin = storage::get_admin(&e);
        admin.require_auth();

        trading::execute_update_config(&e, &admin, &oracle, caller_take_rate, max_positions);
    }

    fn queue_set_market(e: Env, asset: Asset, config: MarketConfig) {
        storage::extend_instance(&e);
        let admin = storage::get_admin(&e);
        admin.require_auth();

        trading::execute_queue_set_market(&e, &admin, &asset, &config);
    }

    fn set_market(e: Env, asset: Asset) {
        storage::extend_instance(&e);
        trading::execute_set_market(&e, &asset);
    }

    fn set_status(e: Env, status: u32) {
        storage::extend_instance(&e);
        let admin = storage::get_admin(&e);
        admin.require_auth();
        
        trading::execute_set_status(&e, &admin, status);
    }

    fn open_position(
        e: Env,
        user: Address,
        asset: Asset,
        collateral: i128,
        leverage: u32,
        is_long: bool,
        entry_price: i128,
    ) -> u32 {
        storage::extend_instance(&e);
        trading::execute_create_position(&e, &user, &asset, collateral, leverage, is_long, entry_price)
    }

    fn modify_risk(e: Env, position_id: u32, stop_loss: i128, take_profit: i128) {
        storage::extend_instance(&e);
        trading::execute_modify_risk(&e, position_id, stop_loss, take_profit);
    }

    fn submit(e: Env, caller: Address, requests: Vec<Request>) -> i128 {
        storage::extend_instance(&e);
        trading::execute_submit(&e, &caller, requests)
    }

    fn upgrade_wasm(e: Env, wasm_hash: BytesN<32>) {
        storage::extend_instance(&e);
        let admin = storage::get_admin(&e);
        admin.require_auth();

        e.deployer().update_current_contract_wasm(wasm_hash.clone());
        TradingEvents::upgrade_wasm(&e, admin.clone(), wasm_hash);
    }
}