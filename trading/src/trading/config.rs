use crate::constants::{SCALAR_18, SCALAR_7, STATUS_SETUP};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::types::{ConfigUpdate, MarketConfig, QueuedMarketInit, TradingConfig};
use crate::{constants::SECONDS_PER_WEEK, storage, MarketData};
use sep_40_oracle::Asset;
use soroban_sdk::{panic_with_error, vec, Address, Env, String};

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
    storage::set_market_list(e, &vec![e]);
    storage::set_status(e, STATUS_SETUP)
}

pub fn execute_queue_set_config(e: &Env, config: &TradingConfig) {
    require_valid_config(e, config);
    let mut unlock_time = e.ledger().timestamp();
    // 1-week delay unless in Setup mode (allows immediate config during initial setup)
    if storage::get_status(e) != STATUS_SETUP {
        unlock_time += SECONDS_PER_WEEK;
    }

    let update = ConfigUpdate {
        config: config.clone(),
        unlock_time,
    };
    storage::set_config_update(e, &update);
    // Emit queue event for clients/indexers
    TradingEvents::queue_set_config(
        e,
        config.oracle.clone(),
        config.caller_take_rate,
        config.max_positions,
        unlock_time,
    );
}

pub fn execute_cancel_set_config(e: &Env) {
    if !storage::has_config_update(e) {
        panic_with_error!(e, TradingError::UpdateNotQueued);
    }
    storage::del_config_update(e);
    TradingEvents::cancel_set_config(e);
}

pub fn execute_set_config(e: &Env) {
    // Apply only if there is a queued config and it's unlocked
    if !storage::has_config_update(e) {
        panic_with_error!(e, TradingError::UpdateNotQueued);
    }

    let queued = storage::get_config_update(e);
    if queued.unlock_time > e.ledger().timestamp() {
        panic_with_error!(e, TradingError::UpdateNotUnlocked);
    }

    // Validate then apply
    require_valid_config(e, &queued.config);
    storage::set_config(e, &queued.config);
    storage::del_config_update(e);

    TradingEvents::set_config(
        e,
        queued.config.oracle.clone(),
        queued.config.caller_take_rate,
        queued.config.max_positions,
    );
}



pub fn execute_queue_set_market(e: &Env, asset: &Asset, config: &MarketConfig) {
    require_valid_market_config(e, config);

    let mut unlock_time = e.ledger().timestamp();
    // 1-week delay unless in Setup mode (allows immediate market activation during initial setup)
    if storage::get_status(e) != STATUS_SETUP {
        unlock_time += SECONDS_PER_WEEK;
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
        panic_with_error!(e, TradingError::UpdateNotUnlocked);
    }

    storage::set_market_config(e, asset, &queued_market.config);

    // Initialize MarketData with default values
    let initial_market_data = MarketData {
        long_collateral: 0,
        long_notional_size: 0,

        short_collateral: 0,
        short_notional_size: 0,

        long_interest_index: SCALAR_18,
        short_interest_index: SCALAR_18,
        last_update: e.ledger().timestamp(),
    };
    storage::set_market_data(e, asset, &initial_market_data);
    storage::push_market_list(e, asset);
    TradingEvents::set_market(e, asset.clone());
}

fn require_valid_market_config(e: &Env, config: &MarketConfig) {
    // Collateral bounds
    if config.min_collateral < SCALAR_7 || config.max_collateral <= config.min_collateral {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // Margin requirements
    if config.maintenance_margin <= 0
        || config.init_margin <= 0
        || config.init_margin < config.maintenance_margin
    {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // Fee configuration
    if config.base_fee < 0 || config.base_hourly_rate < 0 || config.price_impact_scalar <= 0 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}

fn require_valid_config(e: &Env, config: &TradingConfig) {
    if config.caller_take_rate < 0 || config.caller_take_rate > SCALAR_7 {
        panic_with_error!(e, TradingError::InvalidConfig);
    }

    // max_utilization must be 0 (disabled) or between 1x and 100x (SCALAR_7 to 100 * SCALAR_7)
    // Values above 100x could cause overflow in fixed-point multiplication
    const MAX_UTILIZATION_CAP: i128 = 100 * SCALAR_7; // 100x = 1_000_000_000
    if config.max_utilization != 0
        && (config.max_utilization < SCALAR_7 || config.max_utilization > MAX_UTILIZATION_CAP)
    {
        panic_with_error!(e, TradingError::InvalidConfig);
    }
}
