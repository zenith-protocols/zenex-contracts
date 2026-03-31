use crate::{
    errors::TradingError,
    types::{MarketConfig, MarketData, Position, TradingConfig},
};
use soroban_sdk::{
    contracttype, panic_with_error, unwrap::UnwrapOptimized, Address, Env,
    IntoVal, TryFromVal, Val, Vec,
};


// Three TTL tiers based on access frequency and expected lifetime:
// - Instance (30/31d): Core state bumped every single transaction. Short threshold
//   is fine because it's extended on every call (hourly at least for funding updates).
// - Market (45/52d): Config and data are touched on every position action but not
//   every tx. Longer threshold provides buffer for idle markets.
// - Position (14/21d): Perp positions are short-lived (most close within days).
//   Shorter TTL avoids paying rent for abandoned/expired positions.

const ONE_DAY_LEDGERS: u32 = 17280; // assumes ~5s per ledger

const LEDGER_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * 30;      // ~30 days
const LEDGER_BUMP_INSTANCE: u32 = LEDGER_THRESHOLD_INSTANCE + ONE_DAY_LEDGERS; // ~31 days

const LEDGER_THRESHOLD_MARKET: u32 = ONE_DAY_LEDGERS * 45;        // ~45 days
const LEDGER_BUMP_MARKET: u32 = LEDGER_THRESHOLD_MARKET + 7 * ONE_DAY_LEDGERS; // ~52 days

const LEDGER_THRESHOLD_POSITION: u32 = ONE_DAY_LEDGERS * 14;      // ~14 days
const LEDGER_BUMP_POSITION: u32 = LEDGER_THRESHOLD_POSITION + 7 * ONE_DAY_LEDGERS; // ~21 days

#[derive(Clone)]
#[contracttype]
pub enum TradingStorageKey {
    // Instance storage
    Status,
    Vault,
    Token,
    PriceVerifier,
    Config,
    Treasury,
    PositionCounter,
    TotalNotional,
    LastFundingUpdate,
    // Persistent storage (per-entity)
    Markets, // Rarely accessed only during ADL and adding markets.
    MarketConfig(u32),
    MarketData(u32),
    UserPositions(Address),
    Position(u32),
}

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

pub fn get_price_verifier(e: &Env) -> Address {
    e.storage()
        .instance()
        .get(&TradingStorageKey::PriceVerifier)
        .unwrap_optimized()
}

pub fn set_price_verifier(e: &Env, price_verifier: &Address) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::PriceVerifier, price_verifier);
}

pub fn get_treasury(e: &Env) -> Address {
    e.storage()
        .instance()
        .get(&TradingStorageKey::Treasury)
        .unwrap_optimized()
}

pub fn set_treasury(e: &Env, treasury: &Address) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::Treasury, treasury);
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

pub fn next_position_id(e: &Env) -> u32 {
    let key = TradingStorageKey::PositionCounter;
    let current: u32 = e.storage().instance().get(&key).unwrap_or(0);
    e.storage().instance().set(&key, &(current + 1));
    current
}

pub fn get_total_notional(e: &Env) -> i128 {
    e.storage()
        .instance()
        .get(&TradingStorageKey::TotalNotional)
        .unwrap_or(0)
}

pub fn set_total_notional(e: &Env, total: i128) {
    e.storage()
        .instance()
        .set(&TradingStorageKey::TotalNotional, &total);
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

pub fn get_markets(e: &Env) -> Vec<u32> {
    let key = TradingStorageKey::Markets;
    let result = e
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(e));
    if !result.is_empty() {
        e.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD_MARKET, LEDGER_BUMP_MARKET);
    }
    result
}

pub fn set_markets(e: &Env, markets: &Vec<u32>) {
    let key = TradingStorageKey::Markets;
    e.storage().persistent().set(&key, markets);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_MARKET, LEDGER_BUMP_MARKET);
}

pub fn get_market_config(e: &Env, feed_id: u32) -> MarketConfig {
    let key = TradingStorageKey::MarketConfig(feed_id);
    let config: MarketConfig = e
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(e, TradingError::MarketNotFound));
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_MARKET, LEDGER_BUMP_MARKET);
    config
}

pub fn set_market_config(e: &Env, feed_id: u32, config: &MarketConfig) {
    let key = TradingStorageKey::MarketConfig(feed_id);
    e.storage().persistent().set(&key, config);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_MARKET, LEDGER_BUMP_MARKET);
}

pub fn get_market_data(e: &Env, feed_id: u32) -> MarketData {
    let key = TradingStorageKey::MarketData(feed_id);
    let result = e
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(e, TradingError::MarketNotFound));
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_MARKET, LEDGER_BUMP_MARKET);
    result
}

pub fn set_market_data(e: &Env, feed_id: u32, data: &MarketData) {
    let key = TradingStorageKey::MarketData(feed_id);
    e.storage().persistent().set(&key, data);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_MARKET, LEDGER_BUMP_MARKET);
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
        .extend_ttl(&key, LEDGER_THRESHOLD_POSITION, LEDGER_BUMP_POSITION);
    result
}

pub fn set_position(e: &Env, position_id: u32, position: &Position) {
    let key = TradingStorageKey::Position(position_id);
    e.storage().persistent().set(&key, position);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_POSITION, LEDGER_BUMP_POSITION);
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
        LEDGER_THRESHOLD_POSITION,
        LEDGER_BUMP_POSITION,
    )
}

pub fn set_user_positions(e: &Env, user: &Address, positions: &Vec<u32>) {
    let key = TradingStorageKey::UserPositions(user.clone());
    e.storage().persistent().set(&key, positions);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_POSITION, LEDGER_BUMP_POSITION);
}

pub fn add_user_position(e: &Env, user: &Address, position_id: u32) {
    let mut positions = get_user_positions(e, user);
    if positions.len() >= crate::constants::MAX_ENTRIES {
        panic_with_error!(e, TradingError::MaxPositionsReached);
    }
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
