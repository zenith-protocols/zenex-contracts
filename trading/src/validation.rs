use crate::constants::SCALAR_18;
use crate::errors::TradingError;
use crate::storage;
use crate::types::{ContractStatus, MarketConfig, Position, TradingConfig};
use sep_40_oracle::PriceFeedClient;
use soroban_sdk::{panic_with_error, Env};

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

/// Contract must be Active or OnIce (for managing existing positions)
pub fn require_not_frozen(e: &Env) {
    let status = ContractStatus::from_u32(e, storage::get_status(e));
    match status {
        ContractStatus::Active | ContractStatus::OnIce => {}
        _ => panic_with_error!(e, TradingError::ContractFrozen),
    }
}

/// Position must have been open for at least min_open_time seconds (panics on failure)
pub fn require_min_open_time(e: &Env, position: &Position) {
    let config = storage::get_config(e);
    if config.min_open_time > 0 {
        let earliest_close = position.created_at.saturating_add(config.min_open_time);
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

pub fn require_valid_config(e: &Env, config: &TradingConfig) {
    let token_scalar = storage::get_token_scalar(e);

    if config.caller_take_rate < 0 || config.max_utilization <= 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // caller_take_rate must not exceed 100%
    if config.caller_take_rate > token_scalar {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // max_utilization must be between 1x and 100x
    let max_utilization_cap = 100 * token_scalar;
    if config.max_utilization < token_scalar || config.max_utilization > max_utilization_cap {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // max_price_age must be greater than oracle resolution
    let oracle_resolution = PriceFeedClient::new(e, &config.oracle).resolution();
    if config.max_price_age <= oracle_resolution {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}

pub fn require_valid_market_config(e: &Env, config: &MarketConfig) {
    let token_scalar = storage::get_token_scalar(e);

    // Check for negative/zero values first
    if config.maintenance_margin <= 0
        || config.init_margin <= 0
        || config.base_fee < 0
        || config.base_hourly_rate < 0
        || config.price_impact_scalar <= 0
    {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // Collateral bounds (positive value validation)
    if config.min_collateral < token_scalar || config.max_collateral <= config.min_collateral {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // Margin relationship validation
    if config.init_margin < config.maintenance_margin {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // ratio_cap must be between 1x and 5x (SCALAR_18 to 5 * SCALAR_18)
    // - Minimum 1x ensures the interest rate mechanism can function
    // - Maximum 5x provides economic bounds on funding rate imbalance
    const MAX_RATIO_CAP: i128 = 5 * SCALAR_18;
    if config.ratio_cap < SCALAR_18 || config.ratio_cap > MAX_RATIO_CAP {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}