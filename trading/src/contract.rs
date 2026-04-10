#![allow(clippy::too_many_arguments)]

use crate::dependencies::PriceVerifierClient;
use crate::errors::TradingError;
use crate::types::{MarketConfig, MarketData, Position, TradingConfig};
use crate::{storage, trading, ContractStatus};
use crate::validation::require_valid_config;
use soroban_sdk::{contract, contractclient, contractimpl, panic_with_error, Address, Bytes, Env, Vec};
use soroban_sdk::unwrap::UnwrapOptimized;
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_contract_utils::upgradeable::{self as upgradeable, Upgradeable};
use stellar_macros::only_owner;

#[contract]
pub struct TradingContract;

#[contractclient(name = "TradingClient")]
pub trait Trading {
    /// (Owner only) Replace the global trading configuration.
    ///
    /// # Parameters
    /// - `config` - New [`TradingConfig`]
    ///
    /// # Panics
    /// - `TradingError::InvalidConfig` (700) if bounds check fails
    /// - `TradingError::NegativeValueNotAllowed` (723) if any rate/fee is negative
    fn set_config(e: Env, config: TradingConfig);

    /// (Owner only) Register a new market or update an existing market's configuration.
    ///
    /// On first call for a `market_id`, initializes `MarketData` with zero notional and
    /// ADL indices at `SCALAR_18`. `config.feed_id` is immutable after creation.
    ///
    /// # Parameters
    /// - `market_id` - Market identifier (u32)
    /// - `config` - Per-market parameters (see [`MarketConfig`], includes `feed_id`)
    ///
    /// # Panics
    /// - `TradingError::MaxMarketsReached` (703) if `MAX_ENTRIES` markets exist
    /// - `TradingError::InvalidConfig` (700) if market config bounds fail or feed_id changed
    /// - `TradingError::NegativeValueNotAllowed` (723) if any rate/fee is negative
    fn set_market(e: Env, market_id: u32, config: MarketConfig);

    /// (Owner only) Remove a market. Subtracts remaining OI from total_notional
    /// and cleans up market config and data storage.
    ///
    /// # Parameters
    /// - `market_id` - Market to remove
    ///
    /// # Panics
    /// - `TradingError::MarketNotFound` (701) if market_id not registered
    fn del_market(e: Env, market_id: u32);

    /// (Owner only) Set contract status to an admin-level state.
    ///
    /// Valid targets: `Active` (0), `AdminOnIce` (2), `Frozen` (3).
    ///
    /// # Panics
    /// - `TradingError::InvalidStatus` (740) if status is `OnIce`
    fn set_status(e: Env, status: u32);

    /// Permissionless circuit breaker and ADL trigger.
    ///
    /// Anyone can call with current price data for all markets.
    /// - **Active**: PnL >= 95% → set `OnIce`. PnL > 100% → also run ADL.
    /// - **OnIce**: PnL < 90% → restore `Active`. PnL > 100% → run ADL.
    /// - **AdminOnIce**: PnL > 100% → run ADL (status stays AdminOnIce).
    /// - **Frozen**: panics.
    ///
    /// # Parameters
    /// - `price` - Binary-encoded Pyth Lazer price payload covering all registered markets
    ///
    /// # Panics
    /// - `TradingError::ThresholdNotMet` (750) if PnL below trigger threshold
    /// - `TradingError::InvalidStatus` (740) if contract is Frozen
    /// - `TradingError::InvalidPrice` (710) if feeds don't match registered markets
    fn update_status(e: Env, price: Bytes);

    /// Place a pending limit order. Collateral is transferred to the contract immediately.
    /// The order is filled later by a keeper via `execute` when the market price
    /// reaches the specified `entry_price`.
    ///
    /// # Parameters
    /// - `user` - Position owner (must `require_auth`)
    /// - `market_id` - Market identifier
    /// - `collateral` - Collateral amount (token_decimals)
    /// - `notional_size` - Position notional (token_decimals)
    /// - `is_long` - `true` for long, `false` for short
    /// - `entry_price` - Desired fill price (price_scalar units)
    /// - `take_profit` - TP trigger price, 0 = not set (price_scalar units)
    /// - `stop_loss` - SL trigger price, 0 = not set (price_scalar units)
    ///
    /// # Returns
    /// Position ID.
    ///
    /// # Panics
    /// - `TradingError::ContractOnIce` (741) if contract is not Active
    /// - `TradingError::NegativeValueNotAllowed` (723) if any value <= 0
    /// - `TradingError::NotionalBelowMinimum` (724) / `NotionalAboveMaximum` (725)
    /// - `TradingError::LeverageAboveMaximum` (726) if notional * margin > collateral
    /// - `TradingError::MarketDisabled` (702) if market is not enabled
    fn place_limit(
        e: Env,
        user: Address,
        market_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
        take_profit: i128,
        stop_loss: i128,
    ) -> u32;

    /// Open a market order, filled immediately at the current oracle price.
    ///
    /// Fees (base + impact) are deducted from collateral before validation.
    /// Market indices are accrued, and the position snapshots current funding/borrowing
    /// indices at fill time.
    ///
    /// # Parameters
    /// - `user` - Position owner (must `require_auth`)
    /// - `market_id` - Market identifier
    /// - `collateral` - Collateral amount (token_decimals)
    /// - `notional_size` - Position notional (token_decimals)
    /// - `is_long` - `true` for long, `false` for short
    /// - `take_profit` - TP trigger price, 0 = not set (price_scalar units)
    /// - `stop_loss` - SL trigger price, 0 = not set (price_scalar units)
    /// - `price` - Binary-encoded price payload
    ///
    /// # Returns
    /// Position ID.
    ///
    /// # Panics
    /// - `TradingError::ContractOnIce` (741) if contract is not Active
    /// - `TradingError::NegativeValueNotAllowed` (723) if any value <= 0
    /// - `TradingError::NotionalBelowMinimum` (724) / `NotionalAboveMaximum` (725)
    /// - `TradingError::LeverageAboveMaximum` (726) if notional * margin > collateral
    /// - `TradingError::MarketDisabled` (702) if market is not enabled
    /// - `TradingError::InvalidPrice` (710) if feed_id mismatch
    /// - `TradingError::UtilizationExceeded` (751) if per-market or global cap exceeded
    fn open_market(
        e: Env,
        user: Address,
        market_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        take_profit: i128,
        stop_loss: i128,
        price: Bytes,
    ) -> u32;

    /// Cancel a position and refund collateral. No settlement or fees applied.
    ///
    /// - **Pending** (unfilled): requires user auth, cancels the limit order.
    /// - **Filled + market deleted**: permissionless cleanup — anyone can trigger
    ///   the refund for stranded positions after `del_market`.
    ///
    /// # Parameters
    /// - `position_id` - ID of the position to cancel
    ///
    /// # Returns
    /// Collateral amount returned to the user (token_decimals).
    ///
    /// # Panics
    /// - `TradingError::PositionNotPending` (721) if position is filled and market still exists (use `close_position` instead)
    /// - `TradingError::ContractFrozen` (742) if contract is Frozen
    /// - `TradingError::PositionNotFound` (720) if position_id is invalid
    fn cancel_position(e: Env, position_id: u32) -> i128;

    /// Close a filled position at the current oracle price with full settlement.
    ///
    /// # Parameters
    /// - `position_id` - ID of the position
    /// - `price` - Binary-encoded price payload (ignored for disabled/deleted markets)
    ///
    /// # Returns
    /// User payout (token_decimals).
    ///
    /// # Panics
    /// - `TradingError::ContractFrozen` (742) if contract is Frozen
    /// - `TradingError::PositionTooNew` (732) if MIN_OPEN_TIME not elapsed (normal path only)
    /// - `TradingError::InvalidPrice` (710) if feed_id mismatch (normal path only)
    fn close_position(e: Env, position_id: u32, price: Bytes) -> i128;

    /// Add or withdraw collateral on an open (filled) position.
    ///
    /// Adding: transfers additional collateral from user to contract.
    /// Withdrawing: checks that remaining equity stays above margin requirement,
    /// then transfers difference back to user.
    ///
    /// # Parameters
    /// - `position_id` - ID of the filled position
    /// - `new_collateral` - Desired collateral amount after modification (token_decimals)
    /// - `price` - Binary-encoded price payload (needed for margin check on withdrawal)
    ///
    /// # Panics
    /// - `TradingError::ContractFrozen` (742) if contract is Frozen
    /// - `TradingError::ActionNotAllowedForStatus` (733) if position is not filled
    /// - `TradingError::CollateralUnchanged` (727) if new_collateral == current
    /// - `TradingError::WithdrawalBreaksMargin` (728) if withdrawal leaves insufficient margin
    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128, price: Bytes);

    /// Update take-profit and stop-loss trigger prices on an existing position.
    ///
    /// Set a trigger to 0 to clear it. TP/SL are pure price triggers — no
    /// entry-price validation. Invalid values simply never fire.
    ///
    /// # Parameters
    /// - `position_id` - ID of the position
    /// - `take_profit` - New TP price, 0 = clear (price_scalar units)
    /// - `stop_loss` - New SL price, 0 = clear (price_scalar units)
    ///
    /// # Panics
    /// - `TradingError::ContractFrozen` (742) if contract is Frozen
    fn set_triggers(e: Env, position_id: u32, take_profit: i128, stop_loss: i128);

    /// Execute a batch of keeper actions for positions in a single market.
    ///
    /// The contract auto-detects the action for each position:
    /// - **Not filled** → fill limit order (if price crossed entry)
    /// - **Filled** → priority: liquidate > stop-loss > take-profit
    ///
    /// All positions must be in the same market as the provided price.
    ///
    /// # Parameters
    /// - `caller` - Keeper address (receives `caller_rate` share of trading fees)
    /// - `position_ids` - Position IDs to process
    /// - `price` - Binary-encoded price payload (single feed)
    ///
    /// # Panics
    /// - `TradingError::ContractFrozen` (742) if contract is Frozen
    /// - `TradingError::InvalidPrice` (710) if position feed doesn't match price feed
    /// - `TradingError::NotActionable` (731) if no valid action for the position
    fn execute(e: Env, caller: Address, market_id: u32, position_ids: Vec<u32>, price: Bytes);

    /// Recalculate and store funding rates for all markets. Permissionless, callable
    /// once per hour.
    ///
    /// Accrues borrowing + funding indices for each market to current timestamp,
    /// then recalculates funding rates based on current OI imbalance.
    ///
    /// # Panics
    /// - `TradingError::FundingTooEarly` (752) if < 1 hour since last call
    fn apply_funding(e: Env);

    /// Returns the position for the given ID.
    fn get_position(e: Env, position_id: u32) -> Position;

    /// Returns all position IDs owned by the given user.
    fn get_user_positions(e: Env, user: Address) -> Vec<u32>;

    /// Returns the market configuration for the given market.
    fn get_market_config(e: Env, market_id: u32) -> MarketConfig;

    /// Returns the mutable market data (notionals, indices) for the given market.
    fn get_market_data(e: Env, market_id: u32) -> MarketData;

    /// Returns all registered market IDs.
    fn get_markets(e: Env) -> Vec<u32>;

    /// Returns the global trading configuration.
    fn get_config(e: Env) -> TradingConfig;

    /// Returns the current contract status (0=Active, 1=OnIce, 2=AdminOnIce, 3=Frozen).
    fn get_status(e: Env) -> u32;

    /// Returns the strategy-vault address.
    fn get_vault(e: Env) -> Address;

    /// Returns the price-verifier contract address.
    fn get_price_verifier(e: Env) -> Address;

    /// Returns the treasury contract address.
    fn get_treasury(e: Env) -> Address;

    /// Returns the collateral token address.
    fn get_token(e: Env) -> Address;
}

#[contractimpl]
impl TradingContract {
    /// Initialize the trading contract with all external dependencies and configuration.
    ///
    /// # Parameters
    /// - `owner` - Admin address (receives `#[only_owner]` privileges)
    /// - `token` - Collateral token address
    /// - `vault` - Strategy-vault address (holds collateral, ERC-4626)
    /// - `price_verifier` - price-verifier contract address
    /// - `treasury` - Treasury contract for protocol fee collection
    /// - `config` - Global trading parameters (see [`TradingConfig`])
    ///
    /// # Panics
    /// - `TradingError::InvalidConfig` (700) if config fails validation bounds
    /// - `TradingError::NegativeValueNotAllowed` (723) if any rate/fee is negative
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

#[contractimpl]
impl Trading for TradingContract {
    #[only_owner]
    fn set_config(e: Env, config: TradingConfig) {
        storage::extend_instance(&e);
        trading::execute_set_config(&e, &config);
    }

    #[only_owner]
    fn set_market(e: Env, market_id: u32, config: MarketConfig) {
        storage::extend_instance(&e);
        trading::execute_set_market(&e, market_id, &config);
    }

    #[only_owner]
    fn del_market(e: Env, market_id: u32) {
        storage::extend_instance(&e);
        trading::execute_del_market(&e, market_id);
    }

    #[only_owner]
    fn set_status(e: Env, status: u32) {
        storage::extend_instance(&e);
        trading::execute_set_status(&e, status);
    }

    fn update_status(e: Env, price: Bytes) {
        storage::extend_instance(&e);
        let pv = PriceVerifierClient::new(&e, &storage::get_price_verifier(&e));
        trading::execute_update_status(&e, &pv.verify_prices(&price));
    }

    fn place_limit(
        e: Env,
        user: Address,
        market_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
        take_profit: i128,
        stop_loss: i128,
    ) -> u32 {
        storage::extend_instance(&e);
        trading::execute_create_limit(
            &e, &user, market_id, collateral, notional_size, is_long,
            entry_price, take_profit, stop_loss,
        )
    }

    fn open_market(
        e: Env,
        user: Address,
        market_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        take_profit: i128,
        stop_loss: i128,
        price: Bytes,
    ) -> u32 {
        storage::extend_instance(&e);
        let pv = PriceVerifierClient::new(&e, &storage::get_price_verifier(&e));
        let pd = pv.verify_price(&price);
        trading::execute_create_market(
            &e, &user, market_id, collateral, notional_size, is_long,
            take_profit, stop_loss, &pd,
        )
    }

    fn cancel_position(e: Env, position_id: u32) -> i128 {
        storage::extend_instance(&e);
        trading::execute_cancel_position(&e, position_id)
    }

    fn close_position(e: Env, position_id: u32, price: Bytes) -> i128 {
        storage::extend_instance(&e);
        trading::execute_close_position(&e, position_id, price)
    }

    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128, price: Bytes) {
        storage::extend_instance(&e);
        let pv = PriceVerifierClient::new(&e, &storage::get_price_verifier(&e));
        trading::execute_modify_collateral(&e, position_id, new_collateral, &pv.verify_price(&price));
    }

    fn set_triggers(e: Env, position_id: u32, take_profit: i128, stop_loss: i128) {
        storage::extend_instance(&e);
        trading::execute_set_triggers(&e, position_id, take_profit, stop_loss);
    }

    fn execute(e: Env, caller: Address, market_id: u32, position_ids: Vec<u32>, price: Bytes) {
        storage::extend_instance(&e);
        let pv = PriceVerifierClient::new(&e, &storage::get_price_verifier(&e));
        trading::execute_trigger(&e, &caller, market_id, position_ids, &pv.verify_price(&price));
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

    fn get_market_config(e: Env, market_id: u32) -> MarketConfig {
        storage::get_market_config(&e, market_id)
    }

    fn get_market_data(e: Env, market_id: u32) -> MarketData {
        storage::get_market_data(&e, market_id)
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

#[contractimpl(contracttrait)]
impl Ownable for TradingContract {}

#[contractimpl]
impl Upgradeable for TradingContract {
    fn upgrade(e: &Env, new_wasm_hash: soroban_sdk::BytesN<32>, operator: Address) {
        operator.require_auth();
        let owner = ownable::get_owner(e).unwrap_optimized();
        if operator != owner {
            panic_with_error!(e, TradingError::Unauthorized)
        }
        upgradeable::upgrade(e, &new_wasm_hash);
    }
}
