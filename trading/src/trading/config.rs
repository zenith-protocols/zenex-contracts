use sep_40_oracle::Asset;
use soroban_sdk::{panic_with_error, vec, Address, Env, String};
use crate::{storage, constants::SECONDS_PER_WEEK, MarketData};
use crate::constants::{SCALAR_18, SCALAR_7};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::types::{MarketConfig, QueuedMarketInit, TradingConfig};

pub fn execute_initialize(e: &Env, name: &String, admin: &Address, oracle: &Address, caller_take_rate: i128, max_positions: u32) {
    storage::set_name(e, name);
    let config = TradingConfig {
        status: 3,
        oracle: oracle.clone(),
        caller_take_rate,
        max_positions,
    };
    require_valid_config(e, &config);
    storage::set_config(e, &config);
    storage::set_market_list(e, &vec![e]);
    storage::set_admin(e, admin);
}

pub fn execute_update_config(e: &Env, admin: &Address, oracle: &Address, caller_take_rate: i128, max_positions: u32) {
    let mut config = storage::get_config(e);
    config.oracle = oracle.clone();
    config.caller_take_rate = caller_take_rate;
    config.max_positions = max_positions;
    require_valid_config(e, &config);
    storage::set_config(e, &config);
    TradingEvents::update_config(e, admin.clone(), oracle.clone(), caller_take_rate, max_positions);
}

pub fn execute_queue_set_market(e: &Env, admin: &Address, asset: &Asset, config: &MarketConfig) {
    require_valid_market_config(e, config);

    let mut unlock_time = e.ledger().timestamp();
    if storage::get_config(e).status != 3 {
        unlock_time += SECONDS_PER_WEEK
    }

    storage::set_queued_market(e, asset, &QueuedMarketInit {
        config: config.clone(),
        unlock_time,
    });
    TradingEvents::queue_set_market(e, admin.clone(), asset.clone(), config.clone());
}

pub fn execute_set_vault(e: &Env, admin: &Address, vault: &Address) {
    if storage::get_config(e).status != 3 {
        panic_with_error!(e, TradingError::BadRequest);
    }

    let vault_client = VaultClient::new(e, vault);
    let token = vault_client.token();
    storage::set_vault(e, &vault);
    storage::set_token(e, &token);
    TradingEvents::set_vault(e, admin.clone(), vault.clone(), token);
}

pub fn execute_set_market(e: &Env, asset: &Asset) {
    let queued_market = storage::get_queued_market(e, asset);
    if queued_market.unlock_time > e.ledger().timestamp() {
        panic_with_error!(e, TradingError::NotUnlocked);
    }

    storage::set_market_config(e, &asset, &queued_market.config);

    // Initialize MarketData with default values
    let initial_market_data = MarketData {
        long_collateral: 0,
        long_borrowed: 0,
        long_count: 0,
        short_collateral: 0,
        short_borrowed: 0,
        short_count: 0,
        long_interest_index: SCALAR_18, // Start with 1.0 in 18-decimal precision
        short_interest_index: SCALAR_18, // Start with 1.0 in 18-decimal precision
        last_update: e.ledger().timestamp(),
    };
    storage::set_market_data(e, asset, &initial_market_data);
    storage::push_market_list(e, asset);
    TradingEvents::set_market(e, asset.clone());
}

fn require_valid_market_config(e: &Env, config: &MarketConfig) {
    //TODO: Validate the market config
}

fn require_valid_config(e: &Env, config: &TradingConfig) {
    if config.caller_take_rate < 0 || config.caller_take_rate > SCALAR_7 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
    if config.max_positions < 1 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}