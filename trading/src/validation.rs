use crate::constants::{
    MAX_CALLER_RATE, MAX_FEE_RATE, MAX_LIQ_FEE, MAX_MARGIN, MAX_R_VAR_MARKET,
    MAX_R_VAR, MAX_RATE_HOURLY, MAX_UTIL, MIN_IMPACT,
};
use crate::errors::TradingError;
use crate::storage;
use crate::types::{ContractStatus, MarketConfig, TradingConfig};
use soroban_sdk::{panic_with_error, Env};

/// Guard: contract must be `Active` to open new positions.
///
/// OnIce, AdminOnIce, and Frozen all block new opens. Existing positions
/// can still be managed (closed, liquidated) under OnIce/AdminOnIce.
///
/// # Panics
/// - `TradingError::ContractOnIce` (741)
pub fn require_active(e: &Env) {
    let status = ContractStatus::from_u32(e, storage::get_status(e));
    match status {
        ContractStatus::Active => {}
        _ => panic_with_error!(e, TradingError::ContractOnIce),
    }
}

/// Guard: contract allows position management (close, modify, cancel, triggers).
///
/// Only `Frozen` blocks management. All other states (Active, OnIce, AdminOnIce)
/// permit existing position operations so users can always exit.
///
/// # Panics
/// - `TradingError::ContractFrozen` (742)
pub fn require_can_manage(e: &Env) {
    let status = ContractStatus::from_u32(e, storage::get_status(e));
    match status {
        ContractStatus::Active | ContractStatus::OnIce | ContractStatus::AdminOnIce => {}
        _ => panic_with_error!(e, TradingError::ContractFrozen),
    }
}

/// Validate global trading configuration parameters against safety bounds.
///
/// # Panics
/// - `TradingError::NegativeValueNotAllowed` (723) if any rate/fee is negative
/// - `TradingError::InvalidConfig` (700) if any value exceeds its upper bound or
///   if min_notional/max_notional/max_util are logically invalid
pub fn require_valid_config(e: &Env, config: &TradingConfig) {
    // Lower bounds: rates and fees must be non-negative
    if config.caller_rate < 0
        || config.fee_dom < 0
        || config.fee_non_dom < 0
        || config.r_base < 0
        || config.r_var < 0
        || config.r_funding < 0
    {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // Upper bounds: each parameter capped to prevent misconfiguration
    if config.caller_rate > MAX_CALLER_RATE
        || config.fee_dom > MAX_FEE_RATE
        || config.fee_non_dom > MAX_FEE_RATE
        || config.r_base > MAX_RATE_HOURLY
        || config.r_var > MAX_R_VAR
        || config.r_funding > MAX_RATE_HOURLY
        || config.max_util > MAX_UTIL
    {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    if config.min_notional <= 0 || config.max_notional <= config.min_notional {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    if config.max_util <= 0 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // fee_dom >= fee_non_dom dominant side should pay more.
    if config.fee_dom < config.fee_non_dom {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}

/// Validate per-market configuration parameters against safety bounds.
///
/// # Panics
/// - `TradingError::NegativeValueNotAllowed` (723) if margin or liq_fee <= 0
/// - `TradingError::InvalidConfig` (700) if bounds exceeded or margin <= liq_fee
pub fn require_valid_market_config(e: &Env, config: &MarketConfig) {
    // feed_id must be a valid Pyth feed identifier (non-zero)
    if config.feed_id == 0 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // margin > 0 required because leverage = 1/margin; margin <= 0 is undefined.
    // liq_fee > 0 required because it doubles as the liquidation threshold.
    if config.margin <= 0
        || config.liq_fee <= 0
        || config.r_var_market < 0
    {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    if config.margin > MAX_MARGIN
        || config.liq_fee > MAX_LIQ_FEE
        || config.r_var_market > MAX_R_VAR_MARKET
        || config.impact < MIN_IMPACT
        || config.max_util > MAX_UTIL
    {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // margin must strictly exceed liq_fee. If margin <= liq_fee, a position
    // opened at max leverage would be immediately liquidatable (equity at margin
    // equals the liquidation threshold). The gap between them is the safety buffer.
    if config.margin <= config.liq_fee {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    if config.max_util <= 0 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}
