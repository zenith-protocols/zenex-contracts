use crate::constants::{
    MAX_CALLER_RATE, MAX_FEE_RATE, MAX_LIQ_FEE, MAX_MARGIN, MAX_R_BORROW,
    MAX_R_VAR, MAX_RATE_HOURLY, MAX_UTIL, MIN_IMPACT,
};
use crate::errors::TradingError;
use crate::storage;
use crate::types::{ContractStatus, MarketConfig, TradingConfig};
use soroban_sdk::{panic_with_error, Env};

/// Contract must be Active (for opening new positions)
pub fn require_active(e: &Env) {
    let status = ContractStatus::from_u32(e, storage::get_status(e));
    match status {
        ContractStatus::Active => {}
        _ => panic_with_error!(e, TradingError::ContractOnIce),
    }
}

/// Contract allows position management (close, modify, cancel, trigger) — everything except Frozen
pub fn require_can_manage(e: &Env) {
    let status = ContractStatus::from_u32(e, storage::get_status(e));
    match status {
        ContractStatus::Active | ContractStatus::OnIce | ContractStatus::AdminOnIce => {}
        _ => panic_with_error!(e, TradingError::ContractFrozen),
    }
}

pub fn require_valid_config(e: &Env, config: &TradingConfig) {
    // Lower bounds
    if config.caller_rate < 0
        || config.fee_dom < 0
        || config.fee_non_dom < 0
        || config.r_base < 0
        || config.r_var < 0
        || config.r_funding < 0
    {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // Upper bounds
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

    // Dominant fee must be >= non-dominant fee (cancel/fill refund logic depends on this)
    if config.fee_dom < config.fee_non_dom {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}

pub fn require_valid_market_config(e: &Env, config: &MarketConfig) {
    // Lower bounds
    if config.margin <= 0
        || config.liq_fee <= 0
        || config.r_borrow < 0
    {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // Upper bounds
    if config.margin > MAX_MARGIN
        || config.liq_fee > MAX_LIQ_FEE
        || config.r_borrow > MAX_R_BORROW
        || config.impact < MIN_IMPACT
        || config.max_util > MAX_UTIL
    {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // margin must be > liq_fee (can't be liquidated before max leverage)
    if config.margin <= config.liq_fee {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    if config.max_util <= 0 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}
