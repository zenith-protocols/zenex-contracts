use soroban_sdk::{contracttype, unwrap::UnwrapOptimized, Address, Env};
use stellar_tokens::fungible::{
    BALANCE_EXTEND_AMOUNT, BALANCE_TTL_THRESHOLD, INSTANCE_EXTEND_AMOUNT, INSTANCE_TTL_THRESHOLD,
};

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct DepositLock {
    /// Timestamp of the most recent deposit (seconds).
    pub timestamp: u64,
    /// Number of shares deposited within the current lock window.
    pub shares: i128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum StrategyStorageKey {
    LockTime,
    Strategy,
    DepositLock(Address),
}

pub fn extend_instance(e: &Env) {
    e.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_EXTEND_AMOUNT);
}

pub fn get_lock_time(e: &Env) -> u64 {
    e.storage()
        .instance()
        .get::<StrategyStorageKey, u64>(&StrategyStorageKey::LockTime)
        .unwrap_optimized()
}

pub fn set_lock_time(e: &Env, lock_time: &u64) {
    e.storage()
        .instance()
        .set::<StrategyStorageKey, u64>(&StrategyStorageKey::LockTime, lock_time);
}

pub fn get_strategy(e: &Env) -> Address {
    e.storage()
        .instance()
        .get::<StrategyStorageKey, Address>(&StrategyStorageKey::Strategy)
        .unwrap_optimized()
}

pub fn set_strategy(e: &Env, strategy: &Address) {
    e.storage()
        .instance()
        .set::<StrategyStorageKey, Address>(&StrategyStorageKey::Strategy, strategy);
}

pub fn get_deposit_lock(e: &Env, user: &Address) -> Option<DepositLock> {
    let key = StrategyStorageKey::DepositLock(user.clone());
    let result = e
        .storage()
        .persistent()
        .get::<StrategyStorageKey, DepositLock>(&key);
    if result.is_some() {
        e.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_THRESHOLD, BALANCE_EXTEND_AMOUNT);
    }
    result
}

pub fn set_deposit_lock(e: &Env, user: &Address, lock: &DepositLock) {
    let key = StrategyStorageKey::DepositLock(user.clone());
    e.storage()
        .persistent()
        .set::<StrategyStorageKey, DepositLock>(&key, lock);
    e.storage()
        .persistent()
        .extend_ttl(&key, BALANCE_TTL_THRESHOLD, BALANCE_EXTEND_AMOUNT);
}
