use crate::constants::{SCALAR_18, SCALAR_7};
use crate::errors::TradingError;
use crate::storage;
use crate::types::{ContractStatus, MarketConfig, TradingConfig};
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
        _ => panic_with_error!(e, TradingError::ContractPaused),
    }
}

/// Contract must be Active or OnIce (for managing existing positions)
pub fn require_not_frozen(e: &Env) {
    let status = ContractStatus::from_u32(e, storage::get_status(e));
    match status {
        ContractStatus::Active | ContractStatus::OnIce => {}
        _ => panic_with_error!(e, TradingError::ContractPaused),
    }
}

pub fn require_valid_config(e: &Env, config: &TradingConfig) {
    if config.caller_take_rate < 0 || config.max_utilization <= 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // caller_take_rate must not exceed 100%
    if config.caller_take_rate > SCALAR_7 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // max_utilization must be between 1x and 100x (SCALAR_7 to 100 * SCALAR_7)
    const MAX_UTILIZATION_CAP: i128 = 100 * SCALAR_7;
    if config.max_utilization < SCALAR_7 || config.max_utilization > MAX_UTILIZATION_CAP {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // max_price_age must be greater than oracle resolution
    let oracle_resolution = PriceFeedClient::new(e, &config.oracle).resolution();
    if config.max_price_age <= oracle_resolution {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}

pub fn require_valid_market_config(e: &Env, config: &MarketConfig) {
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
    if config.min_collateral < SCALAR_7 || config.max_collateral <= config.min_collateral {
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

/// Unit tests for require_valid_market_config - pure validation that doesn't need external contracts
/// Storage-dependent tests (require_active, require_market_enabled, require_valid_config)
/// are in test-suites/tests/test_trading_validation.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{SCALAR_7, SCALAR_18};
    use crate::testutils::default_market;
    use soroban_sdk::Env;

    // ==========================================
    // require_valid_market_config Tests
    // Pure validation - no storage or external contracts needed
    // ==========================================

    #[test]
    fn test_valid_config() {
        let e = Env::default();
        let config = default_market(&e);
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_zero_maintenance_margin() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.maintenance_margin = 0;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_negative_maintenance_margin() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.maintenance_margin = -1;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_zero_init_margin() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.init_margin = 0;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_init_below_maintenance() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.init_margin = 0_0040000;
        config.maintenance_margin = 0_0050000;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_min_collateral_below_min() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.min_collateral = SCALAR_7 - 1;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_max_below_min_collateral() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.min_collateral = 100 * SCALAR_7;
        config.max_collateral = 50 * SCALAR_7;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_max_equals_min_collateral() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.min_collateral = 100 * SCALAR_7;
        config.max_collateral = 100 * SCALAR_7;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_negative_base_fee() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.base_fee = -1;
        require_valid_market_config(&e, &config);
    }

    #[test]
    fn test_zero_base_fee() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.base_fee = 0;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_negative_hourly_rate() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.base_hourly_rate = -1;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_zero_price_impact() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.price_impact_scalar = 0;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_negative_price_impact() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.price_impact_scalar = -1;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_below_min_ratio() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.ratio_cap = SCALAR_18 - 1;
        require_valid_market_config(&e, &config);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_above_max_ratio() {
        let e = Env::default();
        let mut config = default_market(&e);
        config.ratio_cap = 5 * SCALAR_18 + 1;
        require_valid_market_config(&e, &config);
    }
}
