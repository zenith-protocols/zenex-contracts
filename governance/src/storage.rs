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
pub const MIN_TTL_LEDGERS: u32 = ONE_DAY_LEDGERS; // 1 day floor

/// Compute TTL for a queued entry: 2x delay with a minimum of 1 day.
/// Returns (threshold, bump) in ledger units.
pub fn ttl_for_delay(delay_seconds: u64) -> (u32, u32) {
    // 5 seconds per ledger
    let delay_ledgers = (delay_seconds / 5) as u32;
    let threshold = (delay_ledgers * 2).max(MIN_TTL_LEDGERS);
    let bump = threshold + ONE_DAY_LEDGERS;
    (threshold, bump)
}

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

pub fn set_queued(e: &Env, nonce: u32, queued: &QueuedCall, delay: u64) {
    let key = GovKey::Queued(nonce);
    e.storage().temporary().set(&key, queued);
    let (threshold, bump) = ttl_for_delay(delay);
    e.storage()
        .temporary()
        .extend_ttl(&key, threshold, bump);
}

pub fn set_pending_delay(e: &Env, pending: &PendingDelay, delay: u64) {
    let key = GovKey::PendingDelay;
    e.storage().temporary().set(&key, pending);
    let (threshold, bump) = ttl_for_delay(delay);
    e.storage()
        .temporary()
        .extend_ttl(&key, threshold, bump);
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
