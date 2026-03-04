use crate::constants::MAINTENANCE_MARGIN_DIVISOR;
use crate::errors::TradingError;
use crate::storage;
use crate::types::{ContractStatus, MarketConfig, Position, TradingConfig};
use soroban_sdk::{panic_with_error, Address, Env};

/// Market must be enabled (for opening new positions / filling limits)
pub fn require_market_enabled(e: &Env, config: &MarketConfig) {
    if !config.enabled {
        panic_with_error!(e, TradingError::MarketDisabled);
    }
}

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

/// Contract must be OnIce or AdminOnIce (for ADL)
pub fn require_on_ice(e: &Env) {
    let status = ContractStatus::from_u32(e, storage::get_status(e));
    match status {
        ContractStatus::OnIce | ContractStatus::AdminOnIce => {}
        _ => panic_with_error!(e, TradingError::NotOnIce),
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

/// Check if position has been open long enough (returns false if too new)
pub fn check_min_open_time(e: &Env, position: &Position, min_open_time: u64) -> bool {
    if min_open_time == 0 {
        return true;
    }
    let earliest_close = position.created_at.saturating_add(min_open_time);
    e.ledger().timestamp() >= earliest_close
}

pub fn require_valid_config(e: &Env, config: &TradingConfig, token: &Address) {
    let token_scalar = storage::get_token_scalar(e, token);

    if config.caller_take_rate < 0
        || config.base_fee_dominant < 0
        || config.base_fee_non_dominant < 0
    {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // caller_take_rate must not exceed 100%
    if config.caller_take_rate > token_scalar {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // vault_skim must be between 0 and 100% (token_scalar)
    if config.vault_skim < 0 || config.vault_skim > token_scalar {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // Collateral bounds
    if config.min_collateral < token_scalar || config.max_collateral <= config.min_collateral {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    if config.max_payout <= 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }
}

pub fn require_valid_market_config(e: &Env, config: &MarketConfig, token: &Address) {
    let token_scalar = storage::get_token_scalar(e, token);

    if config.init_margin <= 0 || config.base_hourly_rate < 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // init_margin must be >= maintenance_margin (hardcoded as token_scalar / MAINTENANCE_MARGIN_DIVISOR)
    let maintenance_margin = token_scalar / MAINTENANCE_MARGIN_DIVISOR;
    if config.init_margin < maintenance_margin {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    if config.price_impact_scalar <= 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }
}