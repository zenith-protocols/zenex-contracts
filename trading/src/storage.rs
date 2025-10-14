use crate::{
    types::{MarketConfig, MarketData, Position, QueuedMarketInit, TradingConfig},
    ConfigUpdate,
};
use sep_40_oracle::Asset;
use soroban_sdk::{
    contracttype, unwrap::UnwrapOptimized, Address, Env, IntoVal, String, Symbol, TryFromVal, Val,
    Vec,
};

/********** Ledger Thresholds **********/

const ONE_DAY_LEDGERS: u32 = 17280; // assumes 5s a ledger
const LEDGER_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * 30; // ~ 30 days
const LEDGER_BUMP_INSTANCE: u32 = LEDGER_THRESHOLD_INSTANCE + ONE_DAY_LEDGERS; // ~ 31 days
const LEDGER_THRESHOLD_SHARED: u32 = ONE_DAY_LEDGERS * 45; // ~ 45 days
const LEDGER_BUMP_SHARED: u32 = LEDGER_THRESHOLD_SHARED + ONE_DAY_LEDGERS; // ~ 46 days
const LEDGER_THRESHOLD_USER: u32 = ONE_DAY_LEDGERS * 100; // ~ 100 days
const LEDGER_BUMP_USER: u32 = LEDGER_THRESHOLD_USER + 20 * ONE_DAY_LEDGERS; // ~ 120 days

/********** Storage Types **********/

const NAME_KEY: &str = "Name";
const STATUS_KEY: &str = "Status";
const VAULT_KEY: &str = "Vault";
const TOKEN_KEY: &str = "Token";
const CONFIG_KEY: &str = "Config";
const MARKET_LIST_KEY: &str = "MarketList";
const POSITION_COUNTER_KEY: &str = "PosCtr";
const CONFIG_UPDATE_KEY: &str = "ConfigUpdate";

#[derive(Clone)]
#[contracttype]
pub enum TradingDataKey {
    // A map of asset to market config
    MarketConfig(Asset),
    // A map of asset to queued market init
    MarketInit(Asset),
    // A map of asset to market data
    MarketData(Asset),
    // Map of positions in the trading platform for a user
    UserPositions(Address),
    // The position data
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

/// Fetch the user's positions or return an empty Vec
///
/// ### Arguments
/// * `user` - The address of the user
pub fn get_user_positions(e: &Env, user: &Address) -> Vec<u32> {
    let key = TradingDataKey::UserPositions(user.clone());
    get_persistent_default(
        e,
        &key,
        || Vec::new(e),
        LEDGER_THRESHOLD_USER,
        LEDGER_BUMP_USER,
    )
}

/// Set the user's positions
///
/// ### Arguments
/// * `user` - The address of the user
/// * `positions` - The new positions for the user
pub fn set_user_positions(e: &Env, user: &Address, positions: &Vec<u32>) {
    let key = TradingDataKey::UserPositions(user.clone());
    e.storage()
        .persistent()
        .set::<TradingDataKey, Vec<u32>>(&key, positions);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_USER, LEDGER_BUMP_USER);
}

/// Add a position to a user's positions
///
/// ### Arguments
/// * `user` - The address of the user
/// * `position_id` - The position ID to add
pub fn add_user_position(e: &Env, user: &Address, position_id: u32) {
    let mut positions = get_user_positions(e, user);
    positions.push_back(position_id);
    set_user_positions(e, user, &positions);
}

/// Remove a position from a user's positions
///
/// ### Arguments
/// * `user` - The address of the user
/// * `position_id` - The position ID to remove
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

/// Set a trading platform name
///
/// ### Arguments
/// * `name` - The Name of the trading platform
pub fn set_name(e: &Env, name: &String) {
    e.storage()
        .instance()
        .set::<Symbol, String>(&Symbol::new(e, NAME_KEY), name);
}

pub fn has_name(e: &Env) -> bool {
    e.storage().instance().has(&Symbol::new(e, NAME_KEY))
}

/********** Trading Config **********/

pub fn get_config(e: &Env) -> TradingConfig {
    e.storage()
        .instance()
        .get(&Symbol::new(e, CONFIG_KEY))
        .unwrap_optimized()
}

pub fn set_config(e: &Env, config: &TradingConfig) {
    e.storage()
        .instance()
        .set::<Symbol, TradingConfig>(&Symbol::new(e, CONFIG_KEY), config);
}

/********** Config Update **********/

pub fn get_config_update(e: &Env) -> ConfigUpdate {
    e.storage()
        .temporary()
        .get(&Symbol::new(e, CONFIG_UPDATE_KEY))
        .unwrap_optimized()
}

pub fn set_config_update(e: &Env, update: &ConfigUpdate) {
    let key = Symbol::new(e, CONFIG_UPDATE_KEY);
    e.storage()
        .temporary()
        .set::<Symbol, ConfigUpdate>(&key, update);
    e.storage()
        .temporary()
        .extend_ttl(&key, LEDGER_THRESHOLD_USER, LEDGER_BUMP_USER);
}

pub fn has_config_update(e: &Env) -> bool {
    e.storage()
        .temporary()
        .has(&Symbol::new(e, CONFIG_UPDATE_KEY))
}

pub fn del_config_update(e: &Env) {
    e.storage()
        .temporary()
        .remove(&Symbol::new(e, CONFIG_UPDATE_KEY));
}

/********** Market Config **********/

pub fn get_market_config(e: &Env, asset: &Asset) -> MarketConfig {
    let key = TradingDataKey::MarketConfig(asset.clone());
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
    e.storage()
        .persistent()
        .get::<TradingDataKey, MarketConfig>(&key)
        .unwrap_optimized()
}

pub fn set_market_config(e: &Env, asset: &Asset, config: &MarketConfig) {
    let key = TradingDataKey::MarketConfig(asset.clone());
    e.storage()
        .persistent()
        .set::<TradingDataKey, MarketConfig>(&key, config);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
}

pub fn has_market(e: &Env, asset: &Asset) -> bool {
    let key = TradingDataKey::MarketConfig(asset.clone());
    e.storage().persistent().has(&key)
}

pub fn get_queued_market(e: &Env, asset: &Asset) -> QueuedMarketInit {
    let key = TradingDataKey::MarketInit(asset.clone());
    e.storage()
        .temporary()
        .get::<TradingDataKey, QueuedMarketInit>(&key)
        .unwrap_optimized()
}

pub fn has_queued_market(e: &Env, asset: &Asset) -> bool {
    let key = TradingDataKey::MarketInit(asset.clone());
    e.storage().temporary().has(&key)
}

pub fn set_queued_market(e: &Env, asset: &Asset, market_init: &QueuedMarketInit) {
    let key = TradingDataKey::MarketInit(asset.clone());
    e.storage()
        .temporary()
        .set::<TradingDataKey, QueuedMarketInit>(&key, market_init);
    e.storage()
        .temporary()
        .extend_ttl(&key, LEDGER_THRESHOLD_USER, LEDGER_BUMP_USER);
}

pub fn del_queued_market(e: &Env, asset: &Asset) {
    let key = TradingDataKey::MarketInit(asset.clone());
    e.storage().temporary().remove(&key);
}

/********** Market Data **********/

pub fn get_market_data(e: &Env, asset: &Asset) -> MarketData {
    let key = TradingDataKey::MarketData(asset.clone());
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
    e.storage()
        .persistent()
        .get::<TradingDataKey, MarketData>(&key)
        .unwrap_optimized()
}

pub fn set_market_data(e: &Env, asset: &Asset, data: &MarketData) {
    let key = TradingDataKey::MarketData(asset.clone());
    e.storage()
        .persistent()
        .set::<TradingDataKey, MarketData>(&key, data);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
}

/********** Market List **********/

pub fn get_market_list(e: &Env) -> Vec<Asset> {
    e.storage()
        .instance()
        .get::<Symbol, Vec<Asset>>(&Symbol::new(e, MARKET_LIST_KEY))
        .unwrap_optimized()
}

pub fn set_market_list(e: &Env, market_list: &Vec<Asset>) {
    e.storage()
        .instance()
        .set::<Symbol, Vec<Asset>>(&Symbol::new(e, MARKET_LIST_KEY), market_list);
}

pub fn push_market_list(e: &Env, asset: &Asset) -> u32 {
    let mut market_list = get_market_list(e);
    market_list.push_back(asset.clone());
    let new_index = market_list.len() - 1;
    e.storage()
        .instance()
        .set::<Symbol, Vec<Asset>>(&Symbol::new(e, MARKET_LIST_KEY), &market_list);
    new_index
}

/********** Position Counter **********/

pub fn get_position_counter(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get::<Symbol, u32>(&Symbol::new(e, POSITION_COUNTER_KEY))
        .unwrap_or(0)
}

pub fn set_position_counter(e: &Env, counter: u32) {
    e.storage()
        .instance()
        .set::<Symbol, u32>(&Symbol::new(e, POSITION_COUNTER_KEY), &counter);
}

pub fn bump_position_id(e: &Env) -> u32 {
    let current_id = get_position_counter(e);
    let new_id = current_id + 1;
    set_position_counter(e, new_id);
    new_id
}

/********** Position Data **********/

pub fn get_position(e: &Env, position_id: u32) -> Position {
    let key = TradingDataKey::Position(position_id);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
    e.storage()
        .persistent()
        .get::<TradingDataKey, Position>(&key)
        .unwrap_optimized()
}

pub fn set_position(e: &Env, position_id: u32, position: &Position) {
    let key = TradingDataKey::Position(position_id);
    e.storage()
        .persistent()
        .set::<TradingDataKey, Position>(&key, position);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
}

/********** Vault Storage **********/

pub fn get_vault(e: &Env) -> Address {
    e.storage()
        .instance()
        .get(&Symbol::new(e, VAULT_KEY))
        .unwrap_optimized()
}

pub fn set_vault(e: &Env, vault: &Address) {
    e.storage()
        .instance()
        .set::<Symbol, Address>(&Symbol::new(e, VAULT_KEY), vault);
}

/********** Token Storage **********/

pub fn get_token(e: &Env) -> Address {
    e.storage()
        .instance()
        .get(&Symbol::new(e, TOKEN_KEY))
        .unwrap_optimized()
}

pub fn set_token(e: &Env, token: &Address) {
    e.storage()
        .instance()
        .set::<Symbol, Address>(&Symbol::new(e, TOKEN_KEY), token);
}

pub fn get_status(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get::<Symbol, u32>(&Symbol::new(e, STATUS_KEY))
        .unwrap_optimized()
}

pub fn set_status(e: &Env, status: u32) {
    e.storage()
        .instance()
        .set::<Symbol, u32>(&Symbol::new(e, STATUS_KEY), &status);
}
