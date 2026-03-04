#![allow(clippy::too_many_arguments)]

use crate::errors::TradingError;
use crate::trading::ExecuteRequest;
use crate::trading::market::Market;
use crate::types::{MarketConfig, Position};
use crate::{storage, trading, TradingConfig};
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
    /// * `oracle` - Address of the oracle contract (immutable after init)
    /// * `config` - Initial trading configuration
    /// ### Panics
    /// If the caller is not the owner
    /// If the contract is already initialized
    /// If the configuration is invalid
    fn initialize(e: Env, name: String, vault: Address, oracle: Address, config: TradingConfig);

    /// (Owner only) Update the trading configuration
    ///
    /// ### Arguments
    /// * `config` - New trading configuration
    ///
    /// ### Panics
    /// If the caller is not the owner
    /// If the configuration is invalid
    fn set_config(e: Env, config: TradingConfig);

    /// (Owner only) Add a new market
    ///
    /// ### Arguments
    /// * `config` - The MarketConfig for the market
    ///
    /// ### Panics
    /// If the caller is not the owner
    /// If the configuration is invalid
    /// If max markets reached
    fn set_market(e: Env, config: MarketConfig);

    /// (Owner only) Sets the status of the trading contract
    ///
    /// ### Arguments
    /// * `status` - The new status code (0: Active, 2: AdminOnIce, 3: Frozen, 99: Setup)
    ///
    /// ### Panics
    /// If the caller is not the owner
    /// If status is OnIce (1) — use set_on_ice instead
    fn set_status(e: Env, status: u32);

    /// Permissionless circuit breaker. Sets contract to OnIce when
    /// net trader PnL >= 90% of vault assets.
    ///
    /// ### Panics
    /// If contract is not Active
    /// If utilization threshold is not met
    fn set_on_ice(e: Env);

    /// Permissionless restore. Sets contract back to Active when
    /// net trader PnL < 90% of vault assets.
    ///
    /// ### Panics
    /// If contract is not in permissionless OnIce state
    /// If utilization threshold is still met
    fn restore_active(e: Env);

    /// Place a limit order
    ///
    /// All positions start as pending limit orders. Keepers fill them via `execute`
    /// when the oracle price satisfies the entry_price condition.
    /// For instant execution, set entry_price to a level that's immediately fillable.
    ///
    /// # Arguments
    /// * `user` - User address placing the order
    /// * `asset_index` - Index of the asset to trade
    /// * `collateral` - Collateral amount
    /// * `notional_size` - Notional size of the position
    /// * `is_long` - Whether position is long (true) or short (false)
    /// * `entry_price` - Limit price (must be > 0). Longs fill when oracle <= entry_price, shorts when oracle >= entry_price
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

    /// Permissionless funding application — accrues funding and refreshes rates for all markets.
    /// No-op if less than one hour has elapsed since the last global funding update.
    fn apply_funding(e: Env);

    /// Permissionless ADL trigger. Computes vault deficit from aggregates,
    /// then reduces winning-side positions uniformly to restore solvency.
    ///
    /// # Panics
    /// * `NoDeficit` - If the vault is healthy (no deficit exists)
    fn trigger_adl(e: Env);

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
    /// The Market struct containing config and data
    ///
    /// # Panics
    /// * `MarketNotFound` - If no market exists at the given asset_index
    fn get_market(e: Env, asset_index: u32) -> Market;

    /// Get the trading configuration
    ///
    /// # Returns
    /// The TradingConfig struct containing global trading parameters
    fn get_config(e: Env) -> TradingConfig;

    /// Get the contract status
    ///
    /// # Returns
    /// The current status code (0: Active, 1: OnIce, 2: AdminOnIce, 3: Frozen, 99: Setup)
    fn get_status(e: Env) -> u32;

    /// Get the vault address
    ///
    /// # Returns
    /// The address of the vault contract
    fn get_vault(e: Env) -> Address;

    /// Get the oracle address
    ///
    /// # Returns
    /// The address of the oracle contract
    fn get_oracle(e: Env) -> Address;

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
    fn initialize(e: Env, name: String, vault: Address, oracle: Address, config: TradingConfig) {
        storage::extend_instance(&e);
        trading::execute_initialize(&e, &name, &vault, &oracle, &config);
    }

    #[only_owner]
    fn set_config(e: Env, config: TradingConfig) {
        storage::extend_instance(&e);
        trading::execute_set_config(&e, &config);
    }

    #[only_owner]
    fn set_market(e: Env, config: MarketConfig) {
        storage::extend_instance(&e);
        trading::execute_set_market(&e, &config);
    }

    #[only_owner]
    fn set_status(e: Env, status: u32) {
        storage::extend_instance(&e);
        trading::execute_set_status(&e, status);
    }

    fn set_on_ice(e: Env) {
        storage::extend_instance(&e);
        trading::execute_set_on_ice(&e);
    }

    fn restore_active(e: Env) {
        storage::extend_instance(&e);
        trading::execute_restore_active(&e);
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

    fn apply_funding(e: Env) {
        storage::extend_instance(&e);
        trading::execute_apply_funding(&e);
    }

    fn trigger_adl(e: Env) {
        storage::extend_instance(&e);
        trading::execute_trigger_adl(&e);
    }

    fn get_position(e: Env, position_id: u32) -> Position {
        storage::get_position(&e, position_id)
    }

    fn get_user_positions(e: Env, user: Address) -> Vec<u32> {
        storage::get_user_positions(&e, &user)
    }

    fn get_market(e: Env, asset_index: u32) -> Market {
        Market::load(&e, asset_index)
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

    fn get_oracle(e: Env) -> Address {
        storage::get_oracle(&e)
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
