use soroban_sdk::{contracttype, BytesN, Env};

#[contracttype]
pub enum DataKey {
    Signer,
    MaxConfidenceBps,
}

pub fn get_signer(e: &Env) -> BytesN<32> {
    e.storage().instance().get(&DataKey::Signer).expect("not initialized")
}

pub fn set_signer(e: &Env, signer: &BytesN<32>) {
    e.storage().instance().set(&DataKey::Signer, signer);
}

pub fn get_max_confidence_bps(e: &Env) -> u32 {
    e.storage().instance().get(&DataKey::MaxConfidenceBps).expect("not initialized")
}

pub fn set_max_confidence_bps(e: &Env, bps: u32) {
    e.storage().instance().set(&DataKey::MaxConfidenceBps, &bps);
}
