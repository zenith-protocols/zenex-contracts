use crate::events::TradingEvents;
use crate::trading::{Request, SubmitResult};
use crate::types::MarketConfig;
use crate::{storage, trading, TradingConfig};
use sep_40_oracle::Asset;
use soroban_sdk::{
    contract, contractclient, contractimpl, unwrap::UnwrapOptimized, Address, BytesN, Env, String,
    Symbol, Vec,
};
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
    fn initialize(
        e: Env,
        name: String,
        vault: Address,
        config: TradingConfig,
    );

    /// (Owner only) Update the trading configuration
    ///
    /// ### Arguments
    /// * `oracle` - The oracle address
    /// * `caller_take_rate` - The take rate for the caller
    /// * `max_positions` - The maximum number of positions
    ///
    /// ### Panics
    /// If the caller is not the owner
    fn set_config(e: Env, config: TradingConfig);

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
    /// * `status` - The new status code (0: Normal, 1: Paused, etc.)
    ///
    /// ### Panics
    /// If the caller is not the owner
    fn set_status(e: Env, status: u32);

    /// Create a position (long or short)
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
    fn create_position(
        e: Env,
        user: Address,
        asset: Asset,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
    ) -> u32;

    /// Executes a batch of trading actions
    ///
    /// # Arguments
    /// * `caller` - Address of the caller executing the actions
    /// * `requests` - Vector of requests to process
    ///
    /// # Returns
    /// Results of the requests processed
    fn submit(e: Env, caller: Address, requests: Vec<Request>) -> SubmitResult;

    fn upgrade(e: Env, wasm_hash: BytesN<32>);
}

#[contractimpl]
impl TradingContract {
    pub fn __constructor(
        e: Env,
        owner: Address,
    ) {
        ownable::set_owner(&e, &owner);
    }
}

#[contractimpl]
impl Trading for TradingContract {

    #[only_owner]
    fn initialize(
        e: Env,
        name: String,
        vault: Address,
        config: TradingConfig,
    ) {
        storage::extend_instance(&e);
        trading::execute_initialize(
            &e,
            &name,
            &vault,
            &config,
        );
    }

    #[only_owner]
    fn set_config(e: Env, config: TradingConfig) {
        storage::extend_instance(&e);
        trading::execute_set_config(&e, &config);
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

    fn create_position(
        e: Env,
        user: Address,
        asset: Asset,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
    ) -> u32 {
        storage::extend_instance(&e);
        trading::execute_create_position(
            &e,
            &user,
            &asset,
            collateral,
            notional_size,
            is_long,
            entry_price,
        )
    }

    fn submit(e: Env, caller: Address, requests: Vec<Request>) -> SubmitResult {
        storage::extend_instance(&e);
        trading::execute_submit(&e, &caller, requests)
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
