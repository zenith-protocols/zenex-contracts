use soroban_sdk::{contracttype, Address, Env, Symbol, Val, Vec};
use soroban_sdk::unwrap::UnwrapOptimized;

#[contracttype]
#[derive(Clone)]
pub enum GovKey {
    Delay,
    Nonce,
    Queued(u32),
    PendingDelay,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedCall {
    pub target: Address,
    pub fn_name: Symbol,
    pub args: Vec<Val>,
    pub unlock_time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingDelay {
    pub new_delay: u64,
    pub unlock_time: u64,
}

pub const ONE_DAY_LEDGERS: u32 = 17280;
pub const LEDGER_THRESHOLD_TEMP: u32 = ONE_DAY_LEDGERS * 100;
pub const LEDGER_BUMP_TEMP: u32 = LEDGER_THRESHOLD_TEMP + 20 * ONE_DAY_LEDGERS;

pub fn get_delay(e: &Env) -> u64 {
    e.storage()
        .instance()
        .get(&GovKey::Delay)
        .unwrap_optimized()
}

pub fn get_queued(e: &Env, nonce: u32) -> Option<QueuedCall> {
    e.storage().temporary().get(&GovKey::Queued(nonce))
}

pub fn get_pending_delay(e: &Env) -> Option<PendingDelay> {
    e.storage().temporary().get(&GovKey::PendingDelay)
}

pub fn set_delay(e: &Env, delay: u64) {
    e.storage().instance().set(&GovKey::Delay, &delay);
}

pub fn set_queued(e: &Env, nonce: u32, queued: &QueuedCall) {
    let key = GovKey::Queued(nonce);
    e.storage().temporary().set(&key, queued);
    e.storage()
        .temporary()
        .extend_ttl(&key, LEDGER_THRESHOLD_TEMP, LEDGER_BUMP_TEMP);
}

pub fn set_pending_delay(e: &Env, pending: &PendingDelay) {
    let key = GovKey::PendingDelay;
    e.storage().temporary().set(&key, pending);
    e.storage()
        .temporary()
        .extend_ttl(&key, LEDGER_THRESHOLD_TEMP, LEDGER_BUMP_TEMP);
}

pub fn next_nonce(e: &Env) -> u32 {
    let key = GovKey::Nonce;
    let current: u32 = e.storage().instance().get(&key).unwrap_or(0);
    e.storage().instance().set(&key, &(current + 1));
    current
}

pub fn remove_queued(e: &Env, nonce: u32) {
    e.storage().temporary().remove(&GovKey::Queued(nonce));
}

pub fn remove_pending_delay(e: &Env) {
    e.storage().temporary().remove(&GovKey::PendingDelay);
}
