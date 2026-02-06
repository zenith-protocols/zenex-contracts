use crate::constants::SECONDS_PER_WEEK;
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{
    CancelSetConfig, CancelSetMarket, QueueSetConfig, QueueSetMarket, SetConfig, SetMarket,
    SetStatus,
};
use crate::types::{ConfigUpdate, ContractStatus, MarketConfig, QueuedMarketInit, TradingConfig};
use crate::validation::{require_valid_config, require_valid_market_config};
use crate::{storage, MarketData};
use sep_40_oracle::{Asset, PriceFeedClient};
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, Address, Env, String};

pub fn execute_initialize(e: &Env, name: &String, vault: &Address, config: &TradingConfig) {
    if storage::has_name(e) {
        panic_with_error!(e, TradingError::AlreadyInitialized);
    }
    storage::set_name(e, name);
    let vault_client = VaultClient::new(e, vault);
    let token = vault_client.query_asset();
    storage::set_vault(e, vault);
    storage::set_token(e, &token);

    // Store decimals before validation (validation depends on token_scalar)
    let price_decimals = PriceFeedClient::new(e, &config.oracle).decimals();
    storage::set_price_decimals(e, price_decimals);
    let token_decimals = TokenClient::new(e, &token).decimals();
    storage::set_token_decimals(e, token_decimals);

    require_valid_config(e, config);
    storage::set_config(e, config);

    storage::set_status(e, ContractStatus::Setup as u32);
}

pub fn execute_queue_set_config(e: &Env, config: &TradingConfig) {
    require_valid_config(e, config);
    let mut unlock_time = e.ledger().timestamp();
    // 1-week delay unless in Setup mode (allows immediate config during initial setup)
    if storage::get_status(e) != ContractStatus::Setup as u32 {
        unlock_time += SECONDS_PER_WEEK;
    }

    let update = ConfigUpdate {
        config: config.clone(),
        unlock_time,
    };
    storage::set_config_update(e, &update);
    QueueSetConfig {
        config: config.clone(),
    }
    .publish(e);
}

pub fn execute_cancel_set_config(e: &Env) {
    let queued = storage::get_config_update(e); // panics if not queued
    storage::del_config_update(e);
    CancelSetConfig {
        config: queued.config,
    }
    .publish(e);
}

pub fn execute_set_config(e: &Env) {
    let queued = storage::get_config_update(e); // panics if not queued

    if queued.unlock_time > e.ledger().timestamp() {
        panic_with_error!(e, TradingError::UpdateNotUnlocked);
    }

    // Validate then apply
    require_valid_config(e, &queued.config);
    storage::set_config(e, &queued.config);
    storage::del_config_update(e);

    SetConfig {
        config: queued.config,
    }
    .publish(e);
}



pub fn execute_queue_set_market(e: &Env, config: &MarketConfig) {
    require_valid_market_config(e, config);

    let mut unlock_time = e.ledger().timestamp();
    // 1-week delay unless in Setup mode (allows immediate market activation during initial setup)
    if storage::get_status(e) != ContractStatus::Setup as u32 {
        unlock_time += SECONDS_PER_WEEK;
    }

    storage::set_queued_market(
        e,
        &config.asset,
        &QueuedMarketInit {
            config: config.clone(),
            unlock_time,
        },
    );
    QueueSetMarket {
        asset: config.asset.clone(),
    }
    .publish(e);
}

pub fn execute_cancel_queued_market(e: &Env, asset: &Asset) {
    storage::del_queued_market(e, asset);
    CancelSetMarket {
        asset: asset.clone(),
    }
    .publish(e);
}

pub fn execute_set_market(e: &Env, asset: &Asset) {
    let queued_market = storage::get_queued_market(e, asset);
    if queued_market.unlock_time > e.ledger().timestamp() {
        panic_with_error!(e, TradingError::UpdateNotUnlocked);
    }

    // Get next market index from counter
    let asset_index = storage::next_market_index(e);

    // Store the queued market config (asset already set during queue)
    storage::set_market_config(e, asset_index, &queued_market.config);

    // Initialize MarketData with default values
    let initial_market_data = MarketData {
        long_notional_size: 0,
        short_notional_size: 0,
        long_interest_index: 0,
        short_interest_index: 0,
        last_update: e.ledger().timestamp(),
    };
    storage::set_market_data(e, asset_index, &initial_market_data);

    // Clean up queued market
    storage::del_queued_market(e, asset);

    SetMarket {
        asset: asset.clone(),
        asset_index,
    }
    .publish(e);
}

pub fn execute_set_status(e: &Env, status: u32) {
    ContractStatus::from_u32(e, status);
    storage::set_status(e, status);
    SetStatus { status }.publish(e);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::SCALAR_7;
    use crate::testutils::{
        create_oracle, create_token, create_trading, create_vault, default_config, default_market,
        setup_env,
    };
    use crate::testutils::jump;

    #[test]
    fn test_initialize() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            assert_eq!(storage::get_status(&e), ContractStatus::Setup as u32);
            assert_eq!(storage::get_vault(&e), vault);
            assert_eq!(storage::get_token(&e), token);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #300)")]
    fn test_initialize_twice() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            let config = default_config(&oracle);
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &config);
            execute_initialize(&e, &String::from_str(&e, "Test2"), &vault, &config);
        });
    }

    #[test]
    fn test_queue_set_config_setup_mode() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));

            let mut new_config = default_config(&oracle);
            new_config.max_positions = 20;
            execute_queue_set_config(&e, &new_config);
            execute_set_config(&e); // No delay in Setup mode

            assert_eq!(storage::get_config(&e).max_positions, 20);
        });
    }

    #[test]
    fn test_queue_set_config_active_mode() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            storage::set_status(&e, ContractStatus::Active as u32);

            let mut new_config = default_config(&oracle);
            new_config.max_positions = 20;
            execute_queue_set_config(&e, &new_config);
        });

        // Advance time past unlock
        jump(&e, 1000 + SECONDS_PER_WEEK + 1);

        e.as_contract(&address, || {
            execute_set_config(&e);
            assert_eq!(storage::get_config(&e).max_positions, 20);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #304)")]
    fn test_set_config_not_unlocked() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            storage::set_status(&e, ContractStatus::Active as u32);
            execute_queue_set_config(&e, &default_config(&oracle));
            execute_set_config(&e); // Should panic - not unlocked
        });
    }

    #[test]
    fn test_cancel_set_config() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            execute_queue_set_config(&e, &default_config(&oracle));
            execute_cancel_set_config(&e);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #303)")]
    fn test_cancel_set_config_not_queued() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            execute_cancel_set_config(&e); // Should panic - nothing queued
        });
    }

    #[test]
    fn test_queue_set_market() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));

            let market = default_market(&e);
            execute_queue_set_market(&e, &market);
            execute_set_market(&e, &market.asset); // No delay in Setup mode
            assert!(storage::get_market_config(&e, 0).enabled);
        });
    }

    #[test]
    fn test_cancel_queued_market() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));

            let market = default_market(&e);
            execute_queue_set_market(&e, &market);
            execute_cancel_queued_market(&e, &market.asset);
            // Market was never created - counter stays at 0
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #304)")]
    fn test_set_market_not_unlocked() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            storage::set_status(&e, ContractStatus::Active as u32);

            let market = default_market(&e);
            execute_queue_set_market(&e, &market);
            execute_set_market(&e, &market.asset); // Should panic - not unlocked
        });
    }

    // ==========================================
    // set_status tests
    // ==========================================

    #[test]
    #[should_panic(expected = "Error(Contract, #381)")]
    fn test_set_status_invalid() {
        let e = setup_env();
        execute_set_status(&e, 42);
    }

    // ==========================================
    // require_valid_config validation
    // ==========================================

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_config_negative_caller_take_rate() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut config = default_config(&oracle);
            config.caller_take_rate = -1;
            execute_queue_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_config_zero_max_utilization() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut config = default_config(&oracle);
            config.max_utilization = 0;
            execute_queue_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_config_caller_take_rate_over_100() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut config = default_config(&oracle);
            config.caller_take_rate = SCALAR_7 + 1;
            execute_queue_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_config_max_utilization_below_1x() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut config = default_config(&oracle);
            config.max_utilization = SCALAR_7 - 1;
            execute_queue_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_config_max_utilization_above_100x() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut config = default_config(&oracle);
            config.max_utilization = 101 * SCALAR_7;
            execute_queue_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_config_max_price_age_equal_oracle_resolution() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut config = default_config(&oracle);
            config.max_price_age = 300; // Equal to oracle resolution
            execute_queue_set_config(&e, &config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_config_max_price_age_below_oracle_resolution() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut config = default_config(&oracle);
            config.max_price_age = 100;
            execute_queue_set_config(&e, &config);
        });
    }

    // ==========================================
    // require_valid_market_config validation
    // ==========================================

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_market_zero_maintenance_margin() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.maintenance_margin = 0;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_market_negative_maintenance_margin() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.maintenance_margin = -1;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_market_zero_init_margin() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.init_margin = 0;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_market_negative_init_margin() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.init_margin = -1;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_market_negative_base_fee() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.base_fee = -1;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_market_negative_base_hourly_rate() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.base_hourly_rate = -1;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_market_zero_price_impact_scalar() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.price_impact_scalar = 0;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #330)")]
    fn test_market_negative_price_impact_scalar() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.price_impact_scalar = -1;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_market_min_collateral_below_scalar() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.min_collateral = SCALAR_7 - 1;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_market_max_collateral_below_min() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.min_collateral = 100 * SCALAR_7;
            market.max_collateral = 50 * SCALAR_7;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_market_max_collateral_equals_min() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.min_collateral = 100 * SCALAR_7;
            market.max_collateral = 100 * SCALAR_7;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_market_init_margin_below_maintenance() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.init_margin = 0_0040000;        // 0.4%
            market.maintenance_margin = 0_0050000;  // 0.5%
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_market_ratio_cap_below_1x() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.ratio_cap = crate::constants::SCALAR_18 - 1;
            execute_queue_set_market(&e, &market);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #302)")]
    fn test_market_ratio_cap_above_5x() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let vault = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            let mut market = default_market(&e);
            market.ratio_cap = 6 * crate::constants::SCALAR_18;
            execute_queue_set_market(&e, &market);
        });
    }
}

