use soroban_sdk::{contracttype, Address, Env, Vec as SorobanVec, Symbol, unwrap::UnwrapOptimized};

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct WithdrawalRequest {
    pub shares: i128,              // Shares to be withdrawn
    pub unlock_time: u64,          // Timestamp when withdrawal can be executed
}

// Persistent storage keys
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum DataKey {
    Strategy(Address),             // Stores net_impact as i128
    WithdrawalRequest(Address),    // Stores WithdrawalRequest
}

const ONE_DAY_LEDGERS: u32 = 17280; // assumes 5s a ledger
const LEDGER_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * 30; // ~ 30 days
const LEDGER_BUMP_INSTANCE: u32 = LEDGER_THRESHOLD_INSTANCE + ONE_DAY_LEDGERS; // ~ 31 days
const LEDGER_THRESHOLD_SHARED: u32 = ONE_DAY_LEDGERS * 45; // ~ 45 days
const LEDGER_BUMP_SHARED: u32 = LEDGER_THRESHOLD_SHARED + ONE_DAY_LEDGERS; // ~ 46 days
const LEDGER_THRESHOLD_USER: u32 = ONE_DAY_LEDGERS * 100; // ~ 100 days
const LEDGER_BUMP_USER: u32 = LEDGER_THRESHOLD_USER + 20 * ONE_DAY_LEDGERS; // ~ 120 days

// Instance storage key strings
const TOKEN: &str = "Token";
const SHARE_TOKEN: &str = "ShareToken";
const TOTAL_SHARES: &str = "TotalShares";
const LOCK_TIME: &str = "LockTime";
const PENALTY_RATE: &str = "PenaltyRate";
const STRATEGIES: &str = "Strategies";

pub fn extend_instance(e: &Env) {
    e.storage()
        .instance()
        .extend_ttl(LEDGER_THRESHOLD_INSTANCE, LEDGER_BUMP_INSTANCE);
}

pub fn get_token(e: &Env) -> Address {
    e.storage().instance().get(&Symbol::new(e, TOKEN)).unwrap_optimized()
}

pub fn set_token(e: &Env, token: &Address) {
    e.storage().instance().set(&Symbol::new(e, TOKEN), token);
}

pub fn get_share_token(e: &Env) -> Address {
    e.storage().instance().get(&Symbol::new(e, SHARE_TOKEN)).unwrap_optimized()
}

pub fn set_share_token(e: &Env, share_token: &Address) {
    e.storage().instance().set(&Symbol::new(e, SHARE_TOKEN), share_token);
}

pub fn get_total_shares(e: &Env) -> i128 {
    e.storage()
        .instance()
        .get(&Symbol::new(e, TOTAL_SHARES))
        .unwrap_optimized()
}

pub fn set_total_shares(e: &Env, total_shares: &i128) {
    e.storage()
        .instance()
        .set(&Symbol::new(e, TOTAL_SHARES), total_shares);
}

pub fn get_lock_time(e: &Env) -> u64 {
    e.storage()
        .instance()
        .get(&Symbol::new(e, LOCK_TIME))
        .unwrap_optimized()
}

pub fn set_lock_time(e: &Env, lock_time: &u64) {
    e.storage()
        .instance()
        .set(&Symbol::new(e, LOCK_TIME), lock_time);
}

pub fn get_penalty_rate(e: &Env) -> i128 {
    e.storage()
        .instance()
        .get(&Symbol::new(e, PENALTY_RATE))
        .unwrap_optimized()
}

pub fn set_penalty_rate(e: &Env, rate: &i128) {
    e.storage()
        .instance()
        .set(&Symbol::new(e, PENALTY_RATE), rate);
}

pub fn get_strategies(e: &Env) -> SorobanVec<Address> {
    e.storage()
        .instance()
        .get(&Symbol::new(e, STRATEGIES))
        .unwrap_optimized()
}

pub fn set_strategies(e: &Env, strategies: &SorobanVec<Address>) {
    e.storage()
        .instance()
        .set(&Symbol::new(e, STRATEGIES), strategies);
}

pub fn get_strategy_net_impact(e: &Env, strategy_addr: &Address) -> i128 {
    let key = DataKey::Strategy(strategy_addr.clone());
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
    e.storage()
        .persistent()
        .get::<DataKey, i128>(&key)
        .unwrap_optimized()
}

pub fn set_strategy_net_impact(e: &Env, strategy_addr: &Address, net_impact: &i128) {
    let key = DataKey::Strategy(strategy_addr.clone());
    e.storage().persistent().set(&key, net_impact);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_SHARED, LEDGER_BUMP_SHARED);
}

pub fn get_withdrawal_request(e: &Env, user: &Address) -> WithdrawalRequest {
    let key = DataKey::WithdrawalRequest(user.clone());
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_USER, LEDGER_BUMP_USER);
    e.storage()
        .persistent()
        .get::<DataKey, WithdrawalRequest>(&key)
        .unwrap_optimized()
}

pub fn set_withdrawal_request(e: &Env, user: &Address, request: &WithdrawalRequest) {
    let key = DataKey::WithdrawalRequest(user.clone());
    e.storage().persistent().set(&key, request);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_USER, LEDGER_BUMP_USER);
}

pub fn remove_withdrawal_request(e: &Env, user: &Address) {
    let key = DataKey::WithdrawalRequest(user.clone());
    e.storage().persistent().remove(&key);
}

pub fn has_withdrawal_request(e: &Env, user: &Address) -> bool {
    let key = DataKey::WithdrawalRequest(user.clone());
    e.storage().persistent().has(&key)
}