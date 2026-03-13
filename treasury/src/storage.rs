use soroban_sdk::{Env, Symbol};

const ONE_DAY_LEDGERS: u32 = 17280;

const LEDGER_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * 30;
const LEDGER_BUMP_INSTANCE: u32 = LEDGER_THRESHOLD_INSTANCE + ONE_DAY_LEDGERS;

pub fn extend_instance(e: &Env) {
    e.storage()
        .instance()
        .extend_ttl(LEDGER_THRESHOLD_INSTANCE, LEDGER_BUMP_INSTANCE);
}

pub fn get_rate(e: &Env) -> i128 {
    e.storage()
        .instance()
        .get::<Symbol, i128>(&Symbol::new(e, "Rate"))
        .unwrap_or(0)
}

pub fn set_rate(e: &Env, rate: i128) {
    e.storage()
        .instance()
        .set::<Symbol, i128>(&Symbol::new(e, "Rate"), &rate);
}
