use crate::constants::{MAX_MARKETS, SCALAR_18, UTILIZATION_THRESHOLD_DEN, UTILIZATION_THRESHOLD_NUM};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::trading::oracle::{get_price_scalar, load_price};
use soroban_fixed_point_math::SorobanFixedPoint;
use crate::events::{SetConfig, SetMarket, SetStatus};
use crate::types::{ContractStatus, MarketConfig, TradingConfig};
use crate::validation::{require_valid_config, require_valid_market_config};
use crate::{storage, MarketData};
use soroban_sdk::{panic_with_error, Address, Env, String};

pub fn execute_initialize(e: &Env, name: &String, vault: &Address, oracle: &Address, config: &TradingConfig) {
    if storage::has_name(e) {
        panic_with_error!(e, TradingError::AlreadyInitialized);
    }
    storage::set_name(e, name);
    let vault_client = VaultClient::new(e, vault);
    let token = vault_client.query_asset();
    storage::set_vault(e, vault);
    storage::set_token(e, &token);
    storage::set_oracle(e, oracle);

    require_valid_config(e, config, &token);
    storage::set_config(e, config);

    storage::set_status(e, ContractStatus::Setup as u32);
}

pub fn execute_set_config(e: &Env, config: &TradingConfig) {
    let token = storage::get_token(e);
    require_valid_config(e, config, &token);
    storage::set_config(e, config);
    SetConfig {
        config: config.clone(),
    }
    .publish(e);
}



pub fn execute_set_market(e: &Env, config: &MarketConfig) {
    let token = storage::get_token(e);
    require_valid_market_config(e, config, &token);

    // Enforce market cap
    if storage::get_market_count(e) >= MAX_MARKETS {
        panic_with_error!(e, TradingError::MaxMarketsReached);
    }

    // Get next market index from counter
    let asset_index = storage::next_market_index(e);

    // Store market config
    storage::set_market_config(e, asset_index, config);

    // Initialize MarketData with default values
    let initial_market_data = MarketData {
        long_notional_size: 0,
        short_notional_size: 0,
        long_funding_index: 0,
        short_funding_index: 0,
        last_update: e.ledger().timestamp(),
        funding_rate: 0,
        long_entry_weighted: 0,
        short_entry_weighted: 0,
        long_adl_index: SCALAR_18,
        short_adl_index: SCALAR_18,
    };
    storage::set_market_data(e, asset_index, &initial_market_data);

    // Initialize global funding timestamp on first market
    if asset_index == 0 {
        storage::set_last_funding_update(e, e.ledger().timestamp());
    }

    SetMarket {
        asset: config.asset.clone(),
        asset_index,
    }
    .publish(e);
}

/// Admin-only status changes: AdminOnIce, Frozen, Active (from admin states)
pub fn execute_set_status(e: &Env, status: u32) {
    let new_status = ContractStatus::from_u32(e, status);
    // Admin cannot set permissionless OnIce (use set_on_ice instead)
    if new_status == ContractStatus::OnIce {
        panic_with_error!(e, TradingError::InvalidStatus);
    }
    storage::set_status(e, status);
    SetStatus { status }.publish(e);
}

/// Permissionless circuit breaker: anyone can call when net trader PnL >= 90% of vault
pub fn execute_set_on_ice(e: &Env) {
    let current = ContractStatus::from_u32(e, storage::get_status(e));
    // Only transition from Active
    if current != ContractStatus::Active {
        panic_with_error!(e, TradingError::InvalidStatus);
    }

    let (net_pnl, vault_balance) = compute_net_pnl_and_vault(e);

    // net_pnl * 10 >= vault_balance * 9 → net_pnl >= 90% of vault
    if net_pnl * UTILIZATION_THRESHOLD_DEN < vault_balance * UTILIZATION_THRESHOLD_NUM {
        panic_with_error!(e, TradingError::ThresholdNotMet);
    }

    let status = ContractStatus::OnIce as u32;
    storage::set_status(e, status);
    SetStatus { status }.publish(e);
}

/// Permissionless restore: anyone can call when net trader PnL < 90% of vault
pub fn execute_restore_active(e: &Env) {
    let current = ContractStatus::from_u32(e, storage::get_status(e));
    // Only restore from permissionless OnIce
    if current != ContractStatus::OnIce {
        panic_with_error!(e, TradingError::InvalidStatus);
    }

    let (net_pnl, vault_balance) = compute_net_pnl_and_vault(e);

    // net_pnl * 10 < vault_balance * 9 → net_pnl < 90% of vault
    if net_pnl * UTILIZATION_THRESHOLD_DEN >= vault_balance * UTILIZATION_THRESHOLD_NUM {
        panic_with_error!(e, TradingError::ThresholdStillMet);
    }

    let status = ContractStatus::Active as u32;
    storage::set_status(e, status);
    SetStatus { status }.publish(e);
}

/// Compute net trader PnL across all markets and vault balance
fn compute_net_pnl_and_vault(e: &Env) -> (i128, i128) {
    let oracle = storage::get_oracle(e);
    let vault = storage::get_vault(e);
    let price_scalar = get_price_scalar(e, &oracle);
    let market_count = storage::get_market_count(e);

    let mut net_pnl: i128 = 0;
    for i in 0..market_count {
        let data = storage::get_market_data(e, i);
        let mkt_config = storage::get_market_config(e, i);
        let price = load_price(e, &oracle, &mkt_config.asset);

        let long_pnl = price.fixed_mul_floor(e, &data.long_entry_weighted, &price_scalar)
            - data.long_notional_size;
        let short_pnl = data.short_notional_size
            - price.fixed_mul_floor(e, &data.short_entry_weighted, &price_scalar);

        net_pnl += long_pnl + short_pnl;
    }

    let vault_balance = VaultClient::new(e, &vault).total_assets();
    (net_pnl, vault_balance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::SCALAR_7;
    use crate::testutils::{
        create_oracle, create_token, create_trading, create_vault, default_config, default_market,
        setup_env,
    };

    #[test]
    fn test_initialize() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            assert_eq!(storage::get_status(&e), ContractStatus::Setup as u32);
            assert_eq!(storage::get_vault(&e), vault);
            assert_eq!(storage::get_token(&e), token);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #700)")]
    fn test_initialize_twice() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            let config = default_config();
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &config);
            execute_initialize(&e, &String::from_str(&e, "Test2"), &vault, &oracle, &config);
        });
    }

    #[test]
    fn test_set_config() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());

            let mut new_config = default_config();
            new_config.caller_take_rate = 0_2000000; // 20%
            execute_set_config(&e, &new_config);

            assert_eq!(storage::get_config(&e).caller_take_rate, 0_2000000);
        });
    }

    #[test]
    fn test_set_market() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());

            let market = default_market(&e);
            execute_set_market(&e, &market);
            assert!(storage::get_market_config(&e, 0).enabled);
        });
    }

    // ==========================================
    // set_status tests
    // ==========================================

    #[test]
    #[should_panic(expected = "Error(Contract, #760)")]
    fn test_set_status_invalid() {
        let e = setup_env();
        execute_set_status(&e, 42);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #760)")]
    fn test_admin_cannot_set_permissionless_on_ice() {
        let e = setup_env();
        execute_set_status(&e, 1); // OnIce (permissionless only)
    }

    // ==========================================
    // require_valid_config validation
    // ==========================================

    #[test]
    #[should_panic(expected = "Error(Contract, #735)")]
    fn test_config_negative_caller_take_rate() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut config = default_config();
            config.caller_take_rate = -1;
            execute_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #702)")]
    fn test_config_caller_take_rate_over_100() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut config = default_config();
            config.caller_take_rate = SCALAR_7 + 1;
            execute_set_config(&e, &config);
        });
    }

    // ==========================================
    // require_valid_market_config validation
    // ==========================================

    #[test]
    #[should_panic(expected = "Error(Contract, #735)")]
    fn test_market_zero_init_margin() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut market = default_market(&e);
            market.init_margin = 0;
            execute_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #735)")]
    fn test_market_negative_init_margin() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut market = default_market(&e);
            market.init_margin = -1;
            execute_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #735)")]
    fn test_config_negative_base_fee() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut config = default_config();
            config.base_fee_dominant = -1;
            execute_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #735)")]
    fn test_market_negative_base_hourly_rate() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut market = default_market(&e);
            market.base_hourly_rate = -1;
            execute_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #735)")]
    fn test_market_zero_price_impact_scalar() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut market = default_market(&e);
            market.price_impact_scalar = 0;
            execute_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #735)")]
    fn test_market_negative_price_impact_scalar() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut market = default_market(&e);
            market.price_impact_scalar = -1;
            execute_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #702)")]
    fn test_config_min_collateral_below_scalar() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut config = default_config();
            config.min_collateral = SCALAR_7 - 1;
            execute_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #702)")]
    fn test_config_max_collateral_below_min() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut config = default_config();
            config.min_collateral = 100 * SCALAR_7;
            config.max_collateral = 50 * SCALAR_7;
            execute_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #702)")]
    fn test_config_max_collateral_equals_min() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut config = default_config();
            config.min_collateral = 100 * SCALAR_7;
            config.max_collateral = 100 * SCALAR_7;
            execute_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #702)")]
    fn test_market_init_margin_below_maintenance() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &oracle, &default_config());
            let mut market = default_market(&e);
            // maintenance_margin = SCALAR_7 / 200 = 0_0050000 (0.5%)
            // init_margin = 0.4% < 0.5% → should panic
            market.init_margin = 0_0040000;
            execute_set_market(&e, &market);
        });
    }

}

