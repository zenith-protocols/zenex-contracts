use crate::constants::{SCALAR_18, SCALAR_7};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::types::{MarketConfig, QueuedMarketInit, TradingConfig};
use crate::{constants::SECONDS_PER_WEEK, storage, MarketData};
use sep_40_oracle::Asset;
use soroban_sdk::{panic_with_error, vec, Address, Env, String};

pub fn execute_initialize(
    e: &Env,
    name: &String,
    vault: &Address,
    config: &TradingConfig,
) {
    storage::set_name(e, name);
    let vault_client = VaultClient::new(e, vault);
    let token = vault_client.token();
    storage::set_vault(e, vault);
    storage::set_token(e, &token);
    require_valid_config(e, config);
    storage::set_config(e, config);
    storage::set_market_list(e, &vec![e]);
    storage::set_status(e, 3u32) //TODO: Define constants for statuses
}

pub fn execute_set_config(
    e: &Env,
    config: &TradingConfig,
) {

    require_valid_config(e, &config);
    storage::set_config(e, &config);
    TradingEvents::set_config(
        e,
        config.oracle.clone(),
        config.caller_take_rate.clone(),
        config.max_positions.clone(),
    );
}

pub fn execute_queue_set_market(e: &Env, asset: &Asset, config: &MarketConfig) {
    require_valid_market_config(e, config);

    let mut unlock_time = e.ledger().timestamp();
    if storage::get_status(e) != 3 { //TODO: Constants for statuses
        unlock_time += SECONDS_PER_WEEK
    }

    storage::set_queued_market(
        e,
        asset,
        &QueuedMarketInit {
            config: config.clone(),
            unlock_time,
        },
    );
    TradingEvents::queue_set_market(e, asset.clone(), config.clone());
}

pub fn execute_cancel_queued_market(e: &Env, asset: &Asset) {
    storage::del_queued_market(e, asset);
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
        long_notional_size: 0,
        long_count: 0,
        short_collateral: 0,
        short_notional_size: 0,
        short_count: 0,
        long_interest_index: SCALAR_18,
        short_interest_index: SCALAR_18,
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
    if config.max_positions <= 0 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}
