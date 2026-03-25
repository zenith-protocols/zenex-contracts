#![allow(clippy::too_many_arguments)]

use crate::trading::ExecuteRequest;
use crate::types::{MarketConfig, MarketData, Position, TradingConfig};
use soroban_sdk::{contractclient, Address, Bytes, Env, Vec};

/// # Trading
///
/// Perpetual futures trading contract on Stellar/Soroban. Supports leveraged
/// long/short positions with limit orders, market orders, funding rates,
/// borrowing fees, and automatic deleveraging (ADL).
///
/// See: Protocol Spec -- `docs/audit/PROTOCOL-SPEC.md`
#[contractclient(name = "TradingClient")]
pub trait Trading {
    /********** Admin **********/

    /// (Owner only) Replace the global trading configuration.
    ///
    /// # Parameters
    /// - `config` - New [`TradingConfig`] (all fields in SCALAR_7 / SCALAR_18 as documented)
    ///
    /// # Panics
    /// - `TradingError::Unauthorized` (1) if caller is not the owner
    /// - `TradingError::InvalidConfig` (702) if bounds check fails
    /// - `TradingError::NegativeValueNotAllowed` (735) if any rate/fee is negative
    fn set_config(e: Env, config: TradingConfig);

    /// (Owner only) Register a new market or update an existing market's configuration.
    ///
    /// On first call for a `feed_id`, initializes `MarketData` with zero notional and
    /// ADL indices at `SCALAR_18`. Also seeds the funding update timestamp on the first market.
    ///
    /// # Parameters
    /// - `feed_id` - Pyth price feed identifier (u32)
    /// - `config` - Per-market parameters (see [`MarketConfig`])
    ///
    /// # Panics
    /// - `TradingError::Unauthorized` (1) if caller is not the owner
    /// - `TradingError::MaxMarketsReached` (770) if `MAX_ENTRIES` (50) markets exist
    /// - `TradingError::InvalidConfig` (702) if market config bounds fail
    fn set_market(e: Env, feed_id: u32, config: MarketConfig);

    /// (Owner only) Remove a market. Fails if any open interest remains.
    ///
    /// # Parameters
    /// - `feed_id` - Market to remove
    ///
    /// # Panics
    /// - `TradingError::Unauthorized` (1) if caller is not the owner
    /// - `TradingError::MarketNotFound` (710) if feed_id not registered
    /// - `TradingError::MarketHasOpenPositions` (771) if l_notional or s_notional != 0
    fn del_market(e: Env, feed_id: u32);

    /// (Owner only) Set contract status to an admin-level state.
    ///
    /// Valid targets: `Active` (0), `AdminOnIce` (2), `Frozen` (3).
    /// Cannot set `OnIce` (1) -- that is reserved for the permissionless circuit breaker.
    ///
    /// # Panics
    /// - `TradingError::Unauthorized` (1) if caller is not the owner
    /// - `TradingError::InvalidStatus` (760) if status is `OnIce`
    fn set_status(e: Env, status: u32);

    /// Permissionless circuit breaker and ADL trigger.
    ///
    /// Anyone can call with current price data for all markets. If net trader PnL
    /// exceeds the vault's capacity (95% threshold), triggers auto-deleveraging (ADL)
    /// on winning positions and sets status to `OnIce`. If already `OnIce` and PnL
    /// dropped below 90%, restores `Active`.
    ///
    /// # Parameters
    /// - `price` - Binary-encoded Pyth Lazer price payload covering all registered markets
    ///
    /// # Panics
    /// - `TradingError::ThresholdNotMet` (780) if PnL below circuit-breaker threshold
    /// - `TradingError::InvalidStatus` (760) if contract is in admin-controlled state
    /// - `TradingError::InvalidPrice` (720) if any market feed is missing from price data
    fn update_status(e: Env, price: Bytes);

    /********** User Actions **********/

    /// Place a pending limit order. Collateral is transferred to the contract immediately.
    /// The order is filled later by a keeper via `execute` when the market price
    /// reaches the specified `entry_price`.
    ///
    /// # Parameters
    /// - `user` - Position owner (must `require_auth`)
    /// - `feed_id` - Market feed identifier
    /// - `collateral` - Collateral amount (token_decimals)
    /// - `notional_size` - Position notional (token_decimals)
    /// - `is_long` - `true` for long, `false` for short
    /// - `entry_price` - Desired fill price (price_scalar units)
    /// - `take_profit` - TP trigger price, 0 = not set (price_scalar units)
    /// - `stop_loss` - SL trigger price, 0 = not set (price_scalar units)
    ///
    /// # Returns
    /// Position ID (monotonically increasing u32).
    ///
    /// # Panics
    /// - `TradingError::ContractOnIce` (761) if contract is not Active
    /// - `TradingError::NegativeValueNotAllowed` (735) if any value <= 0
    /// - `TradingError::NotionalBelowMinimum` (736) / `NotionalAboveMaximum` (737)
    /// - `TradingError::LeverageAboveMaximum` (739) if notional * margin > collateral
    /// - `TradingError::MarketDisabled` (712) if market is disabled
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
    ) -> u32;

    /// Open a market order, filled immediately at the current oracle price.
    ///
    /// Fees (base + impact) are deducted from collateral before validation.
    /// Market indices are accrued, and the position snapshots current funding/borrowing
    /// indices at fill time.
    ///
    /// # Parameters
    /// - `user` - Position owner (must `require_auth`)
    /// - `feed_id` - Market feed identifier (must match the verified price feed)
    /// - `collateral` - Collateral amount (token_decimals)
    /// - `notional_size` - Position notional (token_decimals)
    /// - `is_long` - `true` for long, `false` for short
    /// - `take_profit` - TP trigger price, 0 = not set (price_scalar units)
    /// - `stop_loss` - SL trigger price, 0 = not set (price_scalar units)
    /// - `price` - Binary-encoded Pyth Lazer price payload
    ///
    /// # Returns
    /// Position ID.
    ///
    /// # Panics
    /// - All panics from `place_limit` plus:
    /// - `TradingError::InvalidPrice` (720) if feed_id mismatch
    /// - `TradingError::UtilizationExceeded` (791) if per-market or global cap exceeded
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
    ) -> u32;

    /// Cancel a pending (unfilled) limit order. Returns full collateral to user.
    ///
    /// # Parameters
    /// - `position_id` - ID of the pending limit order
    ///
    /// # Returns
    /// Collateral amount returned to the user (token_decimals).
    ///
    /// # Panics
    /// - `TradingError::ContractFrozen` (762) if contract is Frozen
    /// - `TradingError::PositionNotPending` (733) if position is already filled
    /// - `TradingError::PositionNotFound` (730) if position_id is invalid
    fn cancel_limit(e: Env, position_id: u32) -> i128;

    /// Close a filled position at the current oracle price. Settles PnL and all
    /// accrued fees (funding, borrowing, base, impact).
    ///
    /// # Parameters
    /// - `position_id` - ID of the filled position
    /// - `price` - Binary-encoded Pyth Lazer price payload
    ///
    /// # Returns
    /// User payout (token_decimals), clamped to >= 0.
    ///
    /// # Panics
    /// - `TradingError::ContractFrozen` (762) if contract is Frozen
    /// - `TradingError::PositionTooNew` (748) if MIN_OPEN_TIME (30s) not elapsed
    /// - `TradingError::InvalidPrice` (720) if feed_id mismatch
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
    /// - `price` - Binary-encoded Pyth Lazer price payload (needed for margin check on withdrawal)
    ///
    /// # Panics
    /// - `TradingError::ContractFrozen` (762) if contract is Frozen
    /// - `TradingError::ActionNotAllowedForStatus` (750) if position is not filled
    /// - `TradingError::CollateralUnchanged` (740) if new_collateral == current
    /// - `TradingError::WithdrawalBreaksMargin` (741) if withdrawal leaves insufficient margin
    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128, price: Bytes);

    /// Update take-profit and stop-loss trigger prices on an existing position.
    ///
    /// Set a trigger to 0 to clear it. Validates that TP > entry for longs
    /// (TP < entry for shorts), and SL < entry for longs (SL > entry for shorts).
    ///
    /// # Parameters
    /// - `position_id` - ID of the position
    /// - `take_profit` - New TP price, 0 = clear (price_scalar units)
    /// - `stop_loss` - New SL price, 0 = clear (price_scalar units)
    ///
    /// # Panics
    /// - `TradingError::ContractFrozen` (762) if contract is Frozen
    /// - `TradingError::InvalidTakeProfitPrice` (742) if TP on wrong side of entry
    /// - `TradingError::InvalidStopLossPrice` (743) if SL on wrong side of entry
    fn set_triggers(e: Env, position_id: u32, take_profit: i128, stop_loss: i128);

    /********** Keeper Actions **********/

    /// Execute a batch of keeper actions (fill limit, stop-loss, take-profit, liquidate)
    /// for positions in a single market.
    ///
    /// All requests must reference positions in the same market as the provided price.
    /// Transfers are batched: vault pays out first, then user/treasury/caller payouts,
    /// then vault receives remaining collateral.
    ///
    /// # Parameters
    /// - `caller` - Keeper address (receives `caller_rate` share of trading fees)
    /// - `requests` - Vector of [`ExecuteRequest`] (request_type + position_id)
    /// - `price` - Binary-encoded Pyth Lazer price payload (single feed)
    ///
    /// # Panics
    /// - `TradingError::ContractFrozen` (762) if contract is Frozen
    /// - Various per-request errors (see `apply_fill`, `apply_liquidation`, etc.)
    fn execute(e: Env, caller: Address, requests: Vec<ExecuteRequest>, price: Bytes);

    /// Recalculate and store funding rates for all markets. Permissionless, callable
    /// once per hour.
    ///
    /// Accrues borrowing + funding indices for each market to current timestamp,
    /// then recalculates funding rates based on current OI imbalance.
    ///
    /// # Panics
    /// - `TradingError::FundingTooEarly` (790) if < 1 hour since last call
    fn apply_funding(e: Env);

    /********** Getters **********/

    /// Returns the position for the given ID.
    fn get_position(e: Env, position_id: u32) -> Position;
    /// Returns all position IDs owned by the given user.
    fn get_user_positions(e: Env, user: Address) -> Vec<u32>;
    /// Returns the market configuration for the given feed.
    fn get_market_config(e: Env, feed_id: u32) -> MarketConfig;
    /// Returns the mutable market data (notionals, indices) for the given feed.
    fn get_market_data(e: Env, feed_id: u32) -> MarketData;
    /// Returns all registered market feed IDs.
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
