use crate::{
    errors::TradingError,
    types::{MarketConfig, MarketData, Position, QueuedMarketInit, TradingConfig},
    ConfigUpdate,
};
use sep_40_oracle::Asset;
use soroban_sdk::{
    contracttype, panic_with_error, unwrap::UnwrapOptimized, Address, Env, IntoVal, String,
    TryFromVal, Val, Vec,
};

/********** Ledger Thresholds **********/

const ONE_DAY_LEDGERS: u32 = 17280; // assumes 5s a ledger

// Instance storage: bump every ~30 days
const LEDGER_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * 30; // 30 days - bump when below this
const LEDGER_BUMP_INSTANCE: u32 = ONE_DAY_LEDGERS * 60;      // 60 days - bump to this

// Shared storage (market config/data): bump every ~30 days
const LEDGER_THRESHOLD_SHARED: u32 = ONE_DAY_LEDGERS * 30;   // 30 days - bump when below this
const LEDGER_BUMP_SHARED: u32 = ONE_DAY_LEDGERS * 60;        // 60 days - bump to this

// User storage (positions): bump every ~30 days
const LEDGER_THRESHOLD_USER: u32 = ONE_DAY_LEDGERS * 30;     // 30 days - bump when below this
const LEDGER_BUMP_USER: u32 = ONE_DAY_LEDGERS * 60;          // 60 days - bump to this

/********** Storage Keys **********/

#[derive(Clone)]
#[contracttype]
pub enum TradingStorageKey {
    // Instance storage
    Name,
    Status,
    Vault,
    Token,
    Config,
    MarketCounter,
    PositionCounter,
    // Temporary storage
    ConfigUpdate,
    MarketInit(Asset), // Uses Asset since index doesn't exist until market is activated
    // Persistent storage
    MarketConfig(u32),
    MarketData(u32),
    UserPositions(Address),
    Position(u32),
}

/********** Storage **********/

/// Bump the instance rent for the contract
pub fn extend_instance(e: &Env) {
    e.storage()
        .instance()
        .extend_ttl(LEDGER_THRESHOLD_INSTANCE, LEDGER_BUMP_INSTANCE);
}

/// Fetch an entry in persistent storage that has a default value if it doesn't exist
fn get_persistent_default<K: IntoVal<Env, Val>, V: TryFromVal<Env, Val>, F: FnOnce() -> V>(
    e: &Env,
    key: &K,
    default: F,
    bump_threshold: u32,
    bump_amount: u32,
) -> V {
    if let Some(result) = e.storage().persistent().get::<K, V>(key) {
        e.storage()
            .persistent()
            .extend_ttl(key, bump_threshold, bump_amount);
        result
    } else {
        default()
    }
}

/********** User Positions **********/

pub fn get_user_positions(e: &Env, user: &Address) -> Vec<u32> {
    let key = TradingStorageKey::UserPositions(user.clone());
    get_persistent_default(
        e,
        &key,
        || Vec::new(e),
        LEDGER_THRESHOLD_USER,
        LEDGER_BUMP_USER,
    )
}

pub fn set_user_positions(e: &Env, user: &Address, positions: &Vec<u32>) {
    let key = TradingStorageKey::UserPositions(user.clone());
    e.storage().persistent().set(&key, positions);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_USER, LEDGER_BUMP_USER);
}

pub fn add_user_position(e: &Env, user: &Address, position_id: u32) {
    let mut positions = get_user_positions(e, user);
    positions.push_back(position_id);
    set_user_positions(e, user, &positions);
}

pub fn remove_user_position(e: &Env, user: &Address, position_id: u32) {
    let mut positions = get_user_positions(e, user);
    for i in 0..positions.len() {
        if positions.get(i) == Some(position_id) {
            positions.remove(i);
            break;
        }
    }
    set_user_positions(e, user, &positions);
}

/********** Metadata **********/

pub fn set_name(e: &Env, name: &String) {
    e.storage().instance().set(&TradingStorageKey::Name, name);
}

pub fn has_name(e: &Env) -> bool {
    e.storage().instance().has(&TradingStorageKey::Name)
}

/********** Trading Config **********/

pub fn get_config(e: &Env) -> TradingConfig {
    e.storage()
        .instance()
        .get(&TradingStorageKey::Config)
        .unwrap_optimized()
}

pub fn set_config(e: &Env, config: &TradingConfig) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::Config, config);
}

/********** Config Update **********/

pub fn set_config_update(e: &Env, update: &ConfigUpdate) {
    let key = TradingStorageKey::ConfigUpdate;
    e.storage().temporary().set(&key, update);
    e.storage()
        .temporary()
        .extend_ttl(&key, LEDGER_THRESHOLD_USER, LEDGER_BUMP_USER);
}

pub fn get_config_update(e: &Env) -> ConfigUpdate {
    e.storage()
        .temporary()
        .get(&TradingStorageKey::ConfigUpdate)
        .unwrap_or_else(|| panic_with_error!(e, TradingError::UpdateNotQueued))
}

pub fn del_config_update(e: &Env) {
    e.storage()
        .temporary()
        .remove(&TradingStorageKey::ConfigUpdate);
}

/********** Market Config **********/

pub fn get_market_config(e: &Env, asset_index: u32) -> MarketConfig {
    let key = TradingStorageKey::MarketConfig(asset_index);
    let result = e
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(e, TradingError::MarketNotFound));
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
    result
}

pub fn set_market_config(e: &Env, asset_index: u32, config: &MarketConfig) {
    let key = TradingStorageKey::MarketConfig(asset_index);
    e.storage().persistent().set(&key, config);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
}

pub fn get_queued_market(e: &Env, asset: &Asset) -> QueuedMarketInit {
    let key = TradingStorageKey::MarketInit(asset.clone());
    e.storage()
        .temporary()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(e, TradingError::MarketInitNotQueued))
}

pub fn set_queued_market(e: &Env, asset: &Asset, market_init: &QueuedMarketInit) {
    let key = TradingStorageKey::MarketInit(asset.clone());
    e.storage().temporary().set(&key, market_init);
    e.storage()
        .temporary()
        .extend_ttl(&key, LEDGER_THRESHOLD_USER, LEDGER_BUMP_USER);
}

pub fn del_queued_market(e: &Env, asset: &Asset) {
    let key = TradingStorageKey::MarketInit(asset.clone());
    e.storage().temporary().remove(&key);
}

/********** Market Data **********/

pub fn get_market_data(e: &Env, asset_index: u32) -> MarketData {
    let key = TradingStorageKey::MarketData(asset_index);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
    e.storage().persistent().get(&key).unwrap_optimized()
}

pub fn set_market_data(e: &Env, asset_index: u32, data: &MarketData) {
    let key = TradingStorageKey::MarketData(asset_index);
    e.storage().persistent().set(&key, data);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
}

/********** Market Counter **********/

pub fn get_market_counter(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&TradingStorageKey::MarketCounter)
        .unwrap_or(0)
}

pub fn set_market_counter(e: &Env, counter: u32) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::MarketCounter, &counter);
}

pub fn bump_market_index(e: &Env) -> u32 {
    let current_index = get_market_counter(e);
    set_market_counter(e, current_index + 1);
    current_index
}

/********** Position Counter **********/

pub fn get_position_counter(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&TradingStorageKey::PositionCounter)
        .unwrap_or(0)
}

pub fn set_position_counter(e: &Env, counter: u32) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::PositionCounter, &counter);
}

pub fn bump_position_id(e: &Env) -> u32 {
    let current_id = get_position_counter(e);
    let new_id = current_id + 1;
    set_position_counter(e, new_id);
    new_id
}

/********** Position Data **********/

pub fn get_position(e: &Env, position_id: u32) -> Position {
    let key = TradingStorageKey::Position(position_id);
    let result = e
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(e, TradingError::PositionNotFound));
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
    result
}

pub fn set_position(e: &Env, position_id: u32, position: &Position) {
    let key = TradingStorageKey::Position(position_id);
    e.storage().persistent().set(&key, position);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
}

/********** Vault Storage **********/

pub fn get_vault(e: &Env) -> Address {
    e.storage()
        .instance()
        .get(&TradingStorageKey::Vault)
        .unwrap_optimized()
}

pub fn set_vault(e: &Env, vault: &Address) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::Vault, vault);
}

/********** Token Storage **********/

pub fn get_token(e: &Env) -> Address {
    e.storage()
        .instance()
        .get(&TradingStorageKey::Token)
        .unwrap_optimized()
}

pub fn set_token(e: &Env, token: &Address) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::Token, token);
}

pub fn get_status(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&TradingStorageKey::Status)
        .unwrap_optimized()
}

pub fn set_status(e: &Env, status: u32) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::Status, &status);
}
