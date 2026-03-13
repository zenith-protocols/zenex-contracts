use crate::constants::{MAINTENANCE_MARGIN_DIVISOR, SCALAR_7};
use crate::errors::TradingError;
use crate::storage;
use crate::types::{ContractStatus, MarketConfig, Position, TradingConfig};
use soroban_sdk::{panic_with_error, Env};

/// Contract must be Active (for opening new positions)
pub fn require_active(e: &Env) {
    let status = ContractStatus::from_u32(e, storage::get_status(e));
    match status {
        ContractStatus::Active => {}
        _ => panic_with_error!(e, TradingError::ContractOnIce),
    }
} 

/// Contract must not be Frozen (for managing existing positions)
pub fn require_not_frozen(e: &Env) {
    let status = ContractStatus::from_u32(e, storage::get_status(e));
    match status {
        ContractStatus::Active | ContractStatus::OnIce | ContractStatus::AdminOnIce => {}
        _ => panic_with_error!(e, TradingError::ContractFrozen),
    }
}

/// Position must have been open for at least min_open_time seconds (panics on failure)
pub fn require_min_open_time(e: &Env, position: &Position, min_open_time: u64) {
    if min_open_time > 0 {
        let earliest_close = position.created_at.saturating_add(min_open_time);
        if e.ledger().timestamp() < earliest_close {
            panic_with_error!(e, TradingError::PositionTooNew);
        }
    }
}


pub fn require_valid_config(e: &Env, config: &TradingConfig) {
    if config.caller_take_rate < 0
        || config.base_fee_dominant < 0
        || config.base_fee_non_dominant < 0
    {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // caller_take_rate must not exceed 100%
    if config.caller_take_rate > SCALAR_7 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // vault_skim must be between 0 and 100%
    if config.vault_skim < 0 || config.vault_skim > SCALAR_7 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // Collateral bounds
    if config.min_collateral <= 0 || config.max_collateral <= config.min_collateral {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    if config.max_payout <= 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }
}

pub fn require_valid_market_config(e: &Env, config: &MarketConfig) {
    if config.init_margin <= 0 || config.base_hourly_rate < 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // init_margin must be >= maintenance_margin (hardcoded as SCALAR_7 / MAINTENANCE_MARGIN_DIVISOR)
    let maintenance_margin = SCALAR_7 / MAINTENANCE_MARGIN_DIVISOR;
    if config.init_margin < maintenance_margin {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    if config.price_impact_scalar <= 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }
}