use soroban_sdk::{contracttype, BytesN, Env};
use soroban_sdk::unwrap::UnwrapOptimized;

#[contracttype]
pub enum DataKey {
    Signer,
    MaxConfidenceBps,
    MaxStaleness,
}

pub fn get_signer(e: &Env) -> BytesN<32> {
    // SAFETY: set in __constructor; always present for initialized contract
    e.storage().instance().get(&DataKey::Signer).unwrap_optimized()
}

pub fn set_signer(e: &Env, signer: &BytesN<32>) {
    e.storage().instance().set(&DataKey::Signer, signer);
}

pub fn get_max_confidence_bps(e: &Env) -> u32 {
    // SAFETY: set in __constructor; always present for initialized contract
    e.storage().instance().get(&DataKey::MaxConfidenceBps).unwrap_optimized()
}

pub fn set_max_confidence_bps(e: &Env, bps: u32) {
    e.storage().instance().set(&DataKey::MaxConfidenceBps, &bps);
}

pub fn get_max_staleness(e: &Env) -> u64 {
    // SAFETY: set in __constructor; always present for initialized contract
    e.storage().instance().get(&DataKey::MaxStaleness).unwrap_optimized()
}

pub fn set_max_staleness(e: &Env, seconds: u64) {
    e.storage().instance().set(&DataKey::MaxStaleness, &seconds);
}
