use crate::errors::TradingError;
use soroban_sdk::{contracttype, panic_with_error, Address, Env};

/// Global trading parameters set by the admin via `set_config`.
///
/// Controls fee structure, rate parameters, and position size limits.
/// Validated by [`require_valid_config`](crate::validation::require_valid_config).
#[contracttype]
#[derive(Clone, Debug)]
pub struct TradingConfig {
    /// Keeper's share of trading fees, paid on execute actions (SCALAR_7, e.g. 1e6 = 10%).
    pub caller_rate: i128,
    /// Minimum notional size per position (token_decimals). Prevents dust positions.
    pub min_notional: i128,
    /// Maximum notional size per position (token_decimals). Limits single-position exposure.
    pub max_notional: i128,
    /// Trading fee rate for the dominant (heavier) side on open/close (SCALAR_7, e.g. 5e4 = 0.5%).
    pub fee_dom: i128,
    /// Trading fee rate for the non-dominant side on open/close (SCALAR_7). Must be <= fee_dom.
    pub fee_non_dom: i128,
    /// Global utilization cap: total_notional / vault_balance (SCALAR_7, e.g. 9e6 = 90%).
    pub max_util: i128,
    /// Base hourly funding rate (SCALAR_18). Scaled by OI imbalance to get actual rate.
    pub r_funding: i128,
    /// Base hourly borrowing rate (SCALAR_18). Multiplied by utilization curve.
    pub r_base: i128,
    /// Variable borrowing multiplier at full utilization (SCALAR_7, e.g. 1e7 = rate doubles).
    pub r_var: i128,
}

/// Per-market parameters set by the admin via `set_market`.
///
/// Each market (identified by Pyth feed_id) has its own risk parameters.
/// Validated by [`require_valid_market_config`](crate::validation::require_valid_market_config).
#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketConfig {
    /// Whether the market accepts new positions. Existing positions unaffected.
    pub enabled: bool,
    /// Per-market utilization cap: market_notional / vault_balance (SCALAR_7).
    pub max_util: i128,
    /// Per-market borrowing weight (SCALAR_7). 1e7 = 1x base rate, 2e7 = 2x for volatile markets.
    pub r_borrow: i128,
    /// Initial margin requirement (SCALAR_7). Max leverage = 1 / margin (e.g. 1e6 = 10% = 10x).
    pub margin: i128,
    /// Liquidation fee and threshold (SCALAR_7). Position is liquidatable when equity < notional * liq_fee.
    /// Must be strictly less than margin to prevent immediate liquidation at max leverage.
    pub liq_fee: i128,
    /// Price-impact fee divisor (SCALAR_7). Impact fee = notional / impact. Higher = less impact.
    pub impact: i128,
}

/// Per-market mutable state, updated on every position action and accrual.
///
/// Contains open interest totals, cumulative rate indices, and ADL state.
/// Positions snapshot the indices at fill time; the delta at settlement gives accrued fees.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketData {
    /// Total long open interest (token_decimals).
    pub l_notional: i128,
    /// Total short open interest (token_decimals).
    pub s_notional: i128,
    /// Cumulative long funding index (SCALAR_18). Increases when longs pay.
    pub l_fund_idx: i128,
    /// Cumulative short funding index (SCALAR_18). Increases when shorts pay.
    pub s_fund_idx: i128,
    /// Cumulative long borrowing index (SCALAR_18). Only increases when longs are dominant.
    pub l_borr_idx: i128,
    /// Cumulative short borrowing index (SCALAR_18). Only increases when shorts are dominant.
    pub s_borr_idx: i128,
    /// Sum of (notional / entry_price) for all long positions. Used for O(1) side-level PnL in ADL.
    pub l_entry_wt: i128,
    /// Sum of (notional / entry_price) for all short positions. Used for O(1) side-level PnL in ADL.
    pub s_entry_wt: i128,
    /// Current funding rate: positive = longs pay shorts, negative = shorts pay longs (SCALAR_18).
    pub fund_rate: i128,
    /// Timestamp of last accrual (seconds since epoch).
    pub last_update: u64,
    /// Long ADL reduction index (SCALAR_18, starts at 1e18). Decreases when longs are deleveraged.
    pub l_adl_idx: i128,
    /// Short ADL reduction index (SCALAR_18, starts at 1e18). Decreases when shorts are deleveraged.
    pub s_adl_idx: i128,
}

/// A leveraged perpetual position (pending limit order or filled).
///
/// Created via `place_limit` (pending) or `open_market` (immediately filled).
/// When filled, the position snapshots the current funding/borrowing/ADL indices
/// from `MarketData`. At settlement, the delta between current and snapshotted
/// indices determines accrued fees.
#[contracttype]
#[derive(Clone)]
pub struct Position {
    /// Position owner address.
    pub user: Address,
    /// `false` = pending limit order, `true` = filled (active position).
    pub filled: bool,
    /// Pyth price feed ID for this position's market.
    pub feed: u32,
    /// Direction: `true` = long, `false` = short.
    pub long: bool,
    /// Stop-loss trigger price (price_scalar units). 0 = not set.
    pub sl: i128,
    /// Take-profit trigger price (price_scalar units). 0 = not set.
    pub tp: i128,
    /// Entry price at fill (price_scalar units). For limits, updated to market price on fill.
    pub entry_price: i128,
    /// Current collateral (token_decimals). Reduced by open fees, modifiable via modify_collateral.
    pub col: i128,
    /// Notional size (token_decimals). May be reduced by ADL.
    pub notional: i128,
    /// Funding index snapshot at fill time (SCALAR_18). Delta to current = accrued funding.
    pub fund_idx: i128,
    /// Borrowing index snapshot at fill time (SCALAR_18). Delta to current = accrued borrowing.
    pub borr_idx: i128,
    /// ADL index snapshot at fill time (SCALAR_18). Ratio to current = notional reduction factor.
    pub adl_idx: i128,
    /// Timestamp of creation (pending) or fill (active). Used for MIN_OPEN_TIME enforcement.
    pub created_at: u64,
}

/// Batch execution request for keeper triggers (used with `execute`).
#[contracttype]
#[derive(Clone)]
pub struct ExecuteRequest {
    /// Action type: 0=Fill, 1=StopLoss, 2=TakeProfit, 3=Liquidate.
    pub request_type: u32,
    /// Target position ID.
    pub position_id: u32,
}

/// Typed variant of `ExecuteRequest.request_type`.
#[derive(Clone, PartialEq, Debug)]
#[repr(u32)]
pub enum ExecuteRequestType {
    /// Fill a pending limit order (price reached entry).
    Fill = 0,
    /// Trigger stop-loss on a filled position.
    StopLoss = 1,
    /// Trigger take-profit on a filled position.
    TakeProfit = 2,
    /// Liquidate an underwater filled position.
    Liquidate = 3,
}

impl ExecuteRequestType {
    pub fn from_u32(e: &Env, value: u32) -> Self {
        match value {
            0 => ExecuteRequestType::Fill,
            1 => ExecuteRequestType::StopLoss,
            2 => ExecuteRequestType::TakeProfit,
            3 => ExecuteRequestType::Liquidate,
            _ => panic_with_error!(e, TradingError::InvalidRequestType),
        }
    }
}

/// Contract operational state. Controls which actions are permitted.
///
/// State transitions:
/// - `Active` -> `OnIce`: permissionless via `update_status` when ADL threshold met
/// - `OnIce` -> `Active`: permissionless via `update_status` when PnL drops below 90%
/// - `Active`/`OnIce` -> `AdminOnIce`/`Frozen`: admin via `set_status`
/// - `AdminOnIce`/`Frozen` -> `Active`: admin via `set_status`
/// - Admin cannot set `OnIce` (reserved for circuit breaker)
#[derive(Clone, PartialEq, Debug)]
#[repr(u32)]
pub enum ContractStatus {
    /// Normal operation. All actions permitted.
    Active = 0,
    /// Permissionless circuit breaker. New opens blocked, management allowed. Set by ADL.
    OnIce = 1,
    /// Admin-set restriction. Same permissions as OnIce. Only admin can lift.
    AdminOnIce = 2,
    /// Admin-set full freeze. All position operations blocked.
    Frozen = 3,
}

impl ContractStatus {
    pub fn from_u32(e: &Env, value: u32) -> Self {
        match value {
            0 => ContractStatus::Active,
            1 => ContractStatus::OnIce,
            2 => ContractStatus::AdminOnIce,
            3 => ContractStatus::Frozen,
            _ => panic_with_error!(e, TradingError::InvalidStatus),
        }
    }
}
