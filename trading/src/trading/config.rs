use crate::constants::SECONDS_PER_WEEK;
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{
    CancelSetConfig, CancelSetMarket, QueueSetConfig, QueueSetMarket, SetConfig, SetMarket,
};
use crate::types::{ConfigUpdate, ContractStatus, MarketConfig, QueuedMarketInit, TradingConfig};
use crate::validation::{require_valid_config, require_valid_market_config};
use crate::{storage, MarketData};
use sep_40_oracle::{Asset, PriceFeedClient};
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
    require_valid_config(e, config);
    storage::set_config(e, config);
    let decimals = PriceFeedClient::new(e, &config.oracle).decimals();
    storage::set_decimals(e, decimals);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::SCALAR_7;
    use crate::testutils::{
        create_oracle, create_token, create_trading, create_vault, default_config, default_market,
    };
    use soroban_sdk::testutils::{Ledger, LedgerInfo};

    fn setup_env() -> soroban_sdk::Env {
        let e = soroban_sdk::Env::default();
        e.mock_all_auths();
        e.ledger().set(LedgerInfo {
            timestamp: 1000,
            protocol_version: 25,
            sequence_number: 100,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3110400,
        });
        e
    }

    #[test]
    fn test_initialize() {
        let e = setup_env();
        let (address, owner) = create_trading(&e);
        let (oracle, _) = create_oracle(&e);
        let (token, _) = create_token(&e, &owner);
        let (vault, _) = create_vault(&e, &token, 1_000_000 * SCALAR_7);

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
        let (vault, _) = create_vault(&e, &token, 1_000_000 * SCALAR_7);

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
        let (vault, _) = create_vault(&e, &token, 1_000_000 * SCALAR_7);

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
        let (vault, _) = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            storage::set_status(&e, ContractStatus::Active as u32);

            let mut new_config = default_config(&oracle);
            new_config.max_positions = 20;
            execute_queue_set_config(&e, &new_config);
        });

        // Advance time past unlock
        e.ledger().set(LedgerInfo {
            timestamp: 1000 + SECONDS_PER_WEEK + 1,
            protocol_version: 25,
            sequence_number: 200,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3110400,
        });

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
        let (vault, _) = create_vault(&e, &token, 1_000_000 * SCALAR_7);

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
        let (vault, _) = create_vault(&e, &token, 1_000_000 * SCALAR_7);

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
        let (vault, _) = create_vault(&e, &token, 1_000_000 * SCALAR_7);

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
        let (vault, _) = create_vault(&e, &token, 1_000_000 * SCALAR_7);

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
        let (vault, _) = create_vault(&e, &token, 1_000_000 * SCALAR_7);

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
        let (vault, _) = create_vault(&e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(&e, &String::from_str(&e, "Test"), &vault, &default_config(&oracle));
            storage::set_status(&e, ContractStatus::Active as u32);

            let market = default_market(&e);
            execute_queue_set_market(&e, &market);
            execute_set_market(&e, &market.asset); // Should panic - not unlocked
        });
    }
}

