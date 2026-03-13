#![allow(clippy::too_many_arguments)]

use crate::trading::ExecuteRequest;
use crate::types::{MarketConfig, MarketData, Position, TradingConfig};
use soroban_sdk::{contractclient, Address, Bytes, Env, Vec};

#[cfg(not(feature = "library"))]
use crate::errors::TradingError;
#[cfg(not(feature = "library"))]
use crate::trading::price_verifier::PriceVerifierClient;
#[cfg(not(feature = "library"))]
use crate::{storage, trading, ContractStatus};
#[cfg(not(feature = "library"))]
use soroban_sdk::{contract, contractimpl, panic_with_error};
#[cfg(not(feature = "library"))]
use stellar_access::ownable::{self as ownable, Ownable};
#[cfg(not(feature = "library"))]
use stellar_contract_utils::upgradeable::UpgradeableInternal;
#[cfg(not(feature = "library"))]
use stellar_macros::{only_owner, Upgradeable};
#[cfg(not(feature = "library"))]
use crate::validation::require_valid_config;

#[cfg(not(feature = "library"))]
#[derive(Upgradeable)]
#[contract]
pub struct TradingContract;

/// ### Trading
///
/// A perpetual futures trading contract supporting leveraged long/short positions
/// with limit orders, market orders, funding rates, and auto-deleveraging.
#[contractclient(name = "TradingClient")]
pub trait Trading {
    /********** Admin **********/

    /// (Owner only) Update the trading configuration
    ///
    /// ### Arguments
    /// * `config` - The new trading configuration (fees, collateral bounds, payout cap, etc.)
    ///
    /// ### Panics
    /// If the caller is not the owner, or if the config values are invalid
    fn set_config(e: Env, config: TradingConfig);

    /// (Owner only) Add a new market or update an existing market's configuration
    ///
    /// If the market is new, initializes market data with zero open interest and
    /// ADL indices at SCALAR_18. Panics if the maximum number of markets is reached.
    ///
    /// ### Arguments
    /// * `feed_id` - The Pyth feed ID for the market
    /// * `config` - The market configuration (margin, interest rate, price impact, enabled flag)
    ///
    /// ### Panics
    /// If the caller is not the owner, if the config is invalid, or if adding a new
    /// market would exceed the maximum market count
    fn set_market(e: Env, feed_id: u32, config: MarketConfig);

    /// (Owner only) Set the contract status to an admin-level state.
    ///
    /// ### Arguments
    /// * `status` - The new status value (Active, AdminOnIce, or Frozen)
    ///
    /// ### Panics
    /// If the caller is not the owner, or if attempting to set OnIce (use `update_status` instead)
    fn set_status(e: Env, status: u32);

    /// Permissionless status update based on price data.
    ///
    /// - If Active: triggers OnIce circuit breaker when net trader PnL >= 90% of vault balance
    /// - If OnIce: restores to Active when PnL drops below threshold, or triggers ADL if deficit exists
    ///
    /// ### Arguments
    /// * `price` - Signed price update bytes from the price verifier
    ///
    /// ### Panics
    /// If the current status is not Active or OnIce, if the threshold condition is not met
    /// (when Active), or if there is no deficit for ADL (when OnIce and threshold still met)
    fn update_status(e: Env, price: Bytes);

    /********** User Actions **********/

    /// Place a pending limit order. The order is stored unfilled and will be executed by
    /// a keeper when the oracle price reaches the specified entry price. Collateral plus
    /// the worst-case fee (dominant-side base fee + price impact) is transferred from the
    /// user to the contract.
    ///
    /// Returns the position ID and total fees charged
    ///
    /// ### Arguments
    /// * `user` - The address placing the order (must authorize)
    /// * `feed_id` - The Pyth Lazer feed ID for the market
    /// * `collateral` - The collateral amount in token decimals
    /// * `notional_size` - The notional position size in token decimals
    /// * `is_long` - True for long, false for short
    /// * `entry_price` - The desired entry price in price decimals
    /// * `take_profit` - Take profit trigger price in price decimals (0 to disable)
    /// * `stop_loss` - Stop loss trigger price in price decimals (0 to disable)
    ///
    /// ### Panics
    /// If the contract is not Active, if the market is disabled, if collateral or leverage
    /// is out of bounds, if values are negative, or if the user has reached max positions
    fn place_limit(
        e: Env,
        user: Address,
        feed_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
        take_profit: i128,
        stop_loss: i128,
    ) -> (u32, i128);

    /// Open a market order that is filled immediately at the current oracle price.
    /// Collateral plus fees are transferred from the user; fees are forwarded to the vault.
    ///
    /// Returns the position ID and total fees charged
    ///
    /// ### Arguments
    /// * `user` - The address opening the position (must authorize)
    /// * `feed_id` - The Pyth Lazer feed ID for the market
    /// * `collateral` - The collateral amount in token decimals
    /// * `notional_size` - The notional position size in token decimals
    /// * `is_long` - True for long, false for short
    /// * `take_profit` - Take profit trigger price in price decimals (0 to disable)
    /// * `stop_loss` - Stop loss trigger price in price decimals (0 to disable)
    /// * `price` - Signed price update bytes from the price verifier
    ///
    /// ### Panics
    /// If the contract is not Active, if the market is disabled, if collateral or leverage
    /// is out of bounds, if values are negative, or if the user has reached max positions
    fn open_market(
        e: Env,
        user: Address,
        feed_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        take_profit: i128,
        stop_loss: i128,
        price: Bytes,
    ) -> (u32, i128);

    /// Cancel a pending (unfilled) limit order. Refunds the full collateral plus fees
    /// that were charged at placement.
    ///
    /// ### Arguments
    /// * `position_id` - The ID of the pending limit order to cancel
    ///
    /// ### Panics
    /// If the contract is Frozen, if the position is already filled, or if the caller
    /// is not the position owner
    fn cancel_limit(e: Env, position_id: u32);

    /// Close a filled position at the current oracle price. Settles PnL, deducts fees
    /// (base fee, price impact, funding), and transfers the payout to the user. If the
    /// position is profitable, the vault covers the profit; if at a loss, remaining
    /// collateral flows to the vault.
    ///
    /// Returns `(pnl, total_fees)` where pnl is the realized profit/loss and total_fees
    /// is the sum of all fees deducted
    ///
    /// ### Arguments
    /// * `position_id` - The ID of the position to close
    /// * `price` - Signed price update bytes from the price verifier
    ///
    /// ### Panics
    /// If the contract is Frozen, if the position is not filled, if the caller is not
    /// the position owner, or if `min_open_time` has not elapsed
    fn close_position(e: Env, position_id: u32, price: Bytes) -> (i128, i128);

    /// Add or withdraw collateral on an open position. When withdrawing from a filled
    /// position, a price proof is required to verify the position remains above the
    /// initial margin requirement after the withdrawal.
    ///
    /// ### Arguments
    /// * `position_id` - The ID of the position to modify
    /// * `new_collateral` - The desired new collateral amount in token decimals
    /// * `price` - Signed price update bytes (required when withdrawing from a filled position)
    ///
    /// ### Panics
    /// If the contract is Frozen, if the caller is not the position owner, if the new
    /// collateral is unchanged/zero/out-of-bounds, if leverage drops below minimum, or
    /// if the withdrawal would break the margin requirement
    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128, price: Bytes);

    /// Update the take profit and stop loss trigger prices on a position. Set to 0 to
    /// disable a trigger.
    ///
    /// ### Arguments
    /// * `position_id` - The ID of the position to update
    /// * `take_profit` - New take profit price in price decimals (0 to disable)
    /// * `stop_loss` - New stop loss price in price decimals (0 to disable)
    ///
    /// ### Panics
    /// If the contract is Frozen, if the caller is not the position owner, or if
    /// trigger values are negative
    fn set_triggers(e: Env, position_id: u32, take_profit: i128, stop_loss: i128);

    /********** Keeper Actions **********/

    /// Execute a batch of keeper actions. Supported request types:
    /// * `0` Fill - fill a pending limit order when oracle price reaches the entry price
    /// * `1` StopLoss - close a position when its stop loss is triggered
    /// * `2` TakeProfit - close a position when its take profit is triggered
    /// * `3` Liquidate - liquidate an underwater position
    ///
    /// The caller receives a portion of fees (`caller_take_rate`) as a keeper incentive.
    ///
    /// ### Arguments
    /// * `caller` - The keeper address receiving fee incentives
    /// * `requests` - A Vec of `ExecuteRequest` structs containing `request_type` and `position_id`
    /// * `price` - Signed price update bytes (verified once, cached for all requests)
    ///
    /// ### Panics
    /// If the contract is Frozen or any request is invalid
    fn execute(e: Env, caller: Address, requests: Vec<ExecuteRequest>, price: Bytes);

    /********** Permissionless Maintenance **********/

    /// Apply funding across all markets. Accrues funding payments between longs and shorts
    /// based on the current funding rate, then recalculates each market's funding rate from
    /// the updated open interest imbalance. Can only be called once per hour.
    ///
    /// ### Panics
    /// If less than one hour has elapsed since the last funding application
    fn apply_funding(e: Env);

    /********** Getters **********/

    /// Fetch a position by its ID
    fn get_position(e: Env, position_id: u32) -> Position;

    /// Fetch all position IDs owned by a user
    fn get_user_positions(e: Env, user: Address) -> Vec<u32>;

    /// Fetch a market's configuration
    fn get_market_config(e: Env, feed_id: u32) -> MarketConfig;

    /// Fetch a market's data (open interest, funding, ADL indices)
    fn get_market_data(e: Env, feed_id: u32) -> MarketData;

    /// Fetch the list of all registered market feed IDs
    fn get_markets(e: Env) -> Vec<u32>;

    /// Fetch the current trading configuration
    fn get_config(e: Env) -> TradingConfig;

    /// Fetch the current contract status
    fn get_status(e: Env) -> u32;

    /// Fetch the vault address
    fn get_vault(e: Env) -> Address;

    /// Fetch the price verifier contract address
    fn get_price_verifier(e: Env) -> Address;

    /// Fetch the treasury contract address
    fn get_treasury(e: Env) -> Address;

    /// Fetch the collateral token address
    fn get_token(e: Env) -> Address;
}

#[cfg(not(feature = "library"))]
#[contractimpl]
impl TradingContract {
    pub fn __constructor(
        e: Env,
        owner: Address,
        token: Address,
        vault: Address,
        price_verifier: Address,
        treasury: Address,
        config: TradingConfig,
    ) {
        require_valid_config(&e, &config);
        ownable::set_owner(&e, &owner);
        storage::set_vault(&e, &vault);
        storage::set_token(&e, &token);
        storage::set_price_verifier(&e, &price_verifier);
        storage::set_treasury(&e, &treasury);
        storage::set_config(&e, &config);
        storage::set_status(&e, ContractStatus::Active as u32);
    }
}

#[cfg(not(feature = "library"))]
#[contractimpl]
impl Trading for TradingContract {
    #[only_owner]
    fn set_config(e: Env, config: TradingConfig) {
        storage::extend_instance(&e);
        trading::execute_set_config(&e, &config);
    }

    #[only_owner]
    fn set_market(e: Env, feed_id: u32, config: MarketConfig) {
        storage::extend_instance(&e);
        trading::execute_set_market(&e, feed_id, &config);
    }

    #[only_owner]
    fn set_status(e: Env, status: u32) {
        storage::extend_instance(&e);
        trading::execute_set_status(&e, status);
    }

    fn update_status(e: Env, price: Bytes) {
        storage::extend_instance(&e);
        let feeds = PriceVerifierClient::new(&e, &storage::get_price_verifier(&e)).verify_prices(&price);
        trading::execute_update_status(&e, &feeds);
    }

    fn place_limit(
        e: Env,
        user: Address,
        feed_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
        take_profit: i128,
        stop_loss: i128,
    ) -> (u32, i128) {
        storage::extend_instance(&e);
        trading::execute_create_limit(
            &e, &user, feed_id, collateral, notional_size, is_long,
            entry_price, take_profit, stop_loss,
        )
    }

    fn open_market(
        e: Env,
        user: Address,
        feed_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        take_profit: i128,
        stop_loss: i128,
        price: Bytes,
    ) -> (u32, i128) {
        storage::extend_instance(&e);
        let price_data = PriceVerifierClient::new(&e, &storage::get_price_verifier(&e)).verify_prices(&price).get(0).unwrap();
        if price_data.feed_id != feed_id {
            panic_with_error!(e, TradingError::PriceNotFound);
        }
        trading::execute_create_market(
            &e, &user, feed_id, collateral, notional_size, is_long,
            take_profit, stop_loss, &price_data,
        )
    }

    fn cancel_limit(e: Env, position_id: u32) {
        storage::extend_instance(&e);
        trading::execute_cancel_limit(&e, position_id);
    }

    fn close_position(e: Env, position_id: u32, price: Bytes) -> (i128, i128) {
        storage::extend_instance(&e);
        let position = storage::get_position(&e, position_id);
        let price_data = PriceVerifierClient::new(&e, &storage::get_price_verifier(&e)).verify_prices(&price).get(0).unwrap();
        if price_data.feed_id != position.feed_id {
            panic_with_error!(e, TradingError::PriceNotFound);
        }
        trading::execute_close_position(&e, position_id, &price_data)
    }

    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128, price: Bytes) {
        storage::extend_instance(&e);
        let position = storage::get_position(&e, position_id);
        let price_data = PriceVerifierClient::new(&e, &storage::get_price_verifier(&e)).verify_prices(&price).get(0).unwrap();
        if price_data.feed_id != position.feed_id {
            panic_with_error!(e, TradingError::PriceNotFound);
        }
        trading::execute_modify_collateral(&e, position_id, new_collateral, &price_data);
    }

    fn set_triggers(e: Env, position_id: u32, take_profit: i128, stop_loss: i128) {
        storage::extend_instance(&e);
        trading::execute_set_triggers(&e, position_id, take_profit, stop_loss);
    }

    fn execute(e: Env, caller: Address, requests: Vec<ExecuteRequest>, price: Bytes) {
        storage::extend_instance(&e);
        let feeds = PriceVerifierClient::new(&e, &storage::get_price_verifier(&e)).verify_prices(&price);
        trading::execute_trigger(&e, &caller, requests, &feeds);
    }

    fn apply_funding(e: Env) {
        storage::extend_instance(&e);
        trading::execute_apply_funding(&e);
    }

    fn get_position(e: Env, position_id: u32) -> Position {
        storage::get_position(&e, position_id)
    }

    fn get_user_positions(e: Env, user: Address) -> Vec<u32> {
        storage::get_user_positions(&e, &user)
    }

    fn get_market_config(e: Env, feed_id: u32) -> MarketConfig {
        storage::get_market_config(&e, feed_id)
    }

    fn get_market_data(e: Env, feed_id: u32) -> MarketData {
        storage::get_market_data(&e, feed_id)
    }

    fn get_markets(e: Env) -> Vec<u32> {
        storage::get_markets(&e)
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

    fn get_price_verifier(e: Env) -> Address {
        storage::get_price_verifier(&e)
    }

    fn get_treasury(e: Env) -> Address {
        storage::get_treasury(&e)
    }

    fn get_token(e: Env) -> Address {
        storage::get_token(&e)
    }

}

#[cfg(not(feature = "library"))]
#[contractimpl(contracttrait)]
impl Ownable for TradingContract {}

#[cfg(not(feature = "library"))]
impl UpgradeableInternal for TradingContract {
    fn _require_auth(e: &Env, operator: &Address) {
        operator.require_auth();
        let owner = ownable::get_owner(e).expect("owner not set");
        if *operator != owner {
            panic_with_error!(e, TradingError::Unauthorized)
        }
    }
}
