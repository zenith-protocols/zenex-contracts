#![allow(clippy::too_many_arguments)]

use crate::errors::TradingError;
use crate::events::SetStatus;
use crate::trading::ExecuteRequest;
use crate::types::{ContractStatus, MarketConfig, MarketData, Position};
use crate::{storage, trading, TradingConfig};
use sep_40_oracle::Asset;
use soroban_sdk::panic_with_error;
use soroban_sdk::{contract, contractclient, contractimpl, Address, Env, String, Vec};
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_contract_utils::upgradeable::UpgradeableInternal;
use stellar_macros::{only_owner, Upgradeable};

#[derive(Upgradeable)]
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
    /// If the caller is not the owner
    /// If there is no queued configuration update
    fn set_config(e: Env);

    /// (Owner only) Queue setting data for a market
    ///
    /// ### Arguments
    /// * `config` - The MarketConfig for the market
    ///
    /// ### Panics
    /// If the caller is not the owner
    fn queue_set_market(e: Env, config: MarketConfig);

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

    /// Open a position
    ///
    /// # Arguments
    /// * `user` - User address opening position
    /// * `asset_index` - Index of the asset to trade
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
        asset_index: u32,
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
    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128);

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

    /// Get a position by ID
    ///
    /// # Arguments
    /// * `position_id` - The unique identifier of the position
    ///
    /// # Returns
    /// The Position struct containing all position data
    ///
    /// # Panics
    /// If the position does not exist
    fn get_position(e: Env, position_id: u32) -> Position;

    /// Get all position IDs for a user
    ///
    /// # Arguments
    /// * `user` - The address of the user
    ///
    /// # Returns
    /// Vector of position IDs owned by the user
    fn get_user_positions(e: Env, user: Address) -> Vec<u32>;

    /// Get market configuration by asset index
    ///
    /// # Arguments
    /// * `asset_index` - The index of the market
    ///
    /// # Returns
    /// The MarketConfig struct containing market parameters
    ///
    /// # Panics
    /// If the market does not exist
    fn get_market_config(e: Env, asset_index: u32) -> MarketConfig;

    /// Get market data (open interest, funding indices) by asset index
    ///
    /// # Arguments
    /// * `asset_index` - The index of the market
    ///
    /// # Returns
    /// The MarketData struct containing current market state
    ///
    /// # Panics
    /// * `MarketNotFound` - If no market exists at the given asset_index
    fn get_market_data(e: Env, asset_index: u32) -> MarketData;

    /// Get the trading configuration
    ///
    /// # Returns
    /// The TradingConfig struct containing global trading parameters
    fn get_config(e: Env) -> TradingConfig;

    /// Get the contract status
    ///
    /// # Returns
    /// The current status code (0: Active, 1: OnIce, 2: Frozen, 99: Setup)
    fn get_status(e: Env) -> u32;

    /// Get the vault address
    ///
    /// # Returns
    /// The address of the vault contract
    fn get_vault(e: Env) -> Address;

    /// Get the collateral token address
    ///
    /// # Returns
    /// The address of the collateral token contract
    fn get_token(e: Env) -> Address;
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
    fn queue_set_market(e: Env, config: MarketConfig) {
        storage::extend_instance(&e);
        trading::execute_queue_set_market(&e, &config);
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
        let status_enum = ContractStatus::from_u32(&e, status);
        match status_enum {
            ContractStatus::Setup => panic_with_error!(&e, TradingError::InvalidStatus),
            _ => {}
        }
        storage::extend_instance(&e);
        storage::set_status(&e, status);
        SetStatus { status }.publish(&e);
    }

    fn open_position(
        e: Env,
        user: Address,
        asset_index: u32,
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
            asset_index,
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

    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128) {
        storage::extend_instance(&e);
        trading::execute_modify_collateral(&e, position_id, new_collateral);
    }

    fn set_triggers(e: Env, position_id: u32, take_profit: i128, stop_loss: i128) {
        storage::extend_instance(&e);
        trading::execute_set_triggers(&e, position_id, take_profit, stop_loss);
    }

    fn execute(e: Env, caller: Address, requests: Vec<ExecuteRequest>) -> Vec<u32> {
        storage::extend_instance(&e);
        trading::execute_trigger(&e, &caller, requests)
    }

    fn get_position(e: Env, position_id: u32) -> Position {
        storage::get_position(&e, position_id)
    }

    fn get_user_positions(e: Env, user: Address) -> Vec<u32> {
        storage::get_user_positions(&e, &user)
    }

    fn get_market_config(e: Env, asset_index: u32) -> MarketConfig {
        storage::get_market_config(&e, asset_index)
    }

    fn get_market_data(e: Env, asset_index: u32) -> MarketData {
        storage::get_market_data(&e, asset_index)
    }

    fn get_config(e: Env) -> TradingConfig {
        storage::get_config(&e)
    }

    fn get_status(e: Env) -> u32 {
        storage::get_status(&e)
    }

    fn get_vault(e: Env) -> Address {
        storage::get_vault(&e)
    }

    fn get_token(e: Env) -> Address {
        storage::get_token(&e)
    }
}

#[contractimpl(contracttrait)]
impl Ownable for TradingContract {}

impl UpgradeableInternal for TradingContract {
    fn _require_auth(e: &Env, operator: &Address) {
        operator.require_auth();
        let owner = ownable::get_owner(e).expect("owner not set");
        if *operator != owner {
            panic_with_error!(e, TradingError::Unauthorized)
        }
    }
}
