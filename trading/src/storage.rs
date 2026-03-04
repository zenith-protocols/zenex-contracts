use crate::{
    errors::TradingError,
    types::{MarketConfig, MarketData, Position, TradingConfig},
};
use soroban_sdk::{
    contracttype, panic_with_error, token::TokenClient, unwrap::UnwrapOptimized, Address, Env,
    IntoVal, String, TryFromVal, Val, Vec,
};

/********** Ledger Thresholds **********/

const ONE_DAY_LEDGERS: u32 = 17280; // assumes 5s a ledger

// Instance storage: core contract state
const LEDGER_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * 30;      // ~30 days
const LEDGER_BUMP_INSTANCE: u32 = LEDGER_THRESHOLD_INSTANCE + ONE_DAY_LEDGERS; // ~31 days

// Shared storage (market config/data): accessed frequently
const LEDGER_THRESHOLD_SHARED: u32 = ONE_DAY_LEDGERS * 45;        // ~45 days
const LEDGER_BUMP_SHARED: u32 = LEDGER_THRESHOLD_SHARED + ONE_DAY_LEDGERS; // ~46 days

// User storage (positions): may be inactive for longer periods
const LEDGER_THRESHOLD_USER: u32 = ONE_DAY_LEDGERS * 100;         // ~100 days
const LEDGER_BUMP_USER: u32 = LEDGER_THRESHOLD_USER + 20 * ONE_DAY_LEDGERS; // ~120 days

/********** Storage Keys **********/

#[derive(Clone)]
#[contracttype]
pub enum TradingStorageKey {
    // Instance storage
    Name,
    Status,
    Vault,
    Token,
    Oracle,
    Config,
    MarketCounter,
    PositionCounter,
    LastFundingUpdate,
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

/********** Config **********/

pub fn set_name(e: &Env, name: &String) {
    e.storage().instance().set(&TradingStorageKey::Name, name);
}

pub fn has_name(e: &Env) -> bool {
    e.storage().instance().has(&TradingStorageKey::Name)
}

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

pub fn get_token_scalar(e: &Env, token: &Address) -> i128 {
    let decimals = TokenClient::new(e, token).decimals();
    10i128.pow(decimals)
}

pub fn get_last_funding_update(e: &Env) -> u64 {
    e.storage()
        .instance()
        .get(&TradingStorageKey::LastFundingUpdate)
        .unwrap_or(0)
}

pub fn set_last_funding_update(e: &Env, timestamp: u64) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::LastFundingUpdate, &timestamp);
}

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

pub fn get_oracle(e: &Env) -> Address {
    e.storage()
        .instance()
        .get(&TradingStorageKey::Oracle)
        .unwrap_optimized()
}

pub fn set_oracle(e: &Env, oracle: &Address) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::Oracle, oracle);
}

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

/********** Market **********/

pub fn get_market_count(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&TradingStorageKey::MarketCounter)
        .unwrap_or(0)
}

pub fn next_market_index(e: &Env) -> u32 {
    let key = TradingStorageKey::MarketCounter;
    let current: u32 = e.storage().instance().get(&key).unwrap_or(0);
    e.storage().instance().set(&key, &(current + 1));
    current
}

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

pub fn get_market_data(e: &Env, asset_index: u32) -> MarketData {
    let key = TradingStorageKey::MarketData(asset_index);
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

pub fn set_market_data(e: &Env, asset_index: u32, data: &MarketData) {
    let key = TradingStorageKey::MarketData(asset_index);
    e.storage().persistent().set(&key, data);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
}

/********** Position **********/

pub fn next_position_id(e: &Env) -> u32 {
    let key = TradingStorageKey::PositionCounter;
    let current: u32 = e.storage().instance().get(&key).unwrap_or(0);
    e.storage().instance().set(&key, &(current + 1));
    current
}

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

pub fn remove_position(e: &Env, position_id: u32) {
    let key = TradingStorageKey::Position(position_id);
    e.storage().persistent().remove(&key);
}

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