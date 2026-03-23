use soroban_sdk::{contracttype, unwrap::UnwrapOptimized, Address, BytesN, Env, Symbol};

/********** Ledger Thresholds **********/

const ONE_DAY_LEDGERS: u32 = 17280; // assumes 5s a ledger

const LEDGER_THRESHOLD_INSTANCE: u32 = ONE_DAY_LEDGERS * 30; // ~30 days
const LEDGER_BUMP_INSTANCE: u32 = LEDGER_THRESHOLD_INSTANCE + ONE_DAY_LEDGERS; // ~31 days

const LEDGER_THRESHOLD_POOL: u32 = ONE_DAY_LEDGERS * 100; // ~100 days
const LEDGER_BUMP_POOL: u32 = LEDGER_THRESHOLD_POOL + 20 * ONE_DAY_LEDGERS; // ~120 days

#[derive(Clone)]
#[contracttype]
pub enum FactoryDataKey {
    Pools(Address),
}

#[derive(Clone)]
#[contracttype]
pub struct FactoryInitMeta {
    pub trading_hash: BytesN<32>,
    pub vault_hash: BytesN<32>,
    pub treasury: Address,
}

/// Bump the instance rent for the contract
pub fn extend_instance(e: &Env) {
    e.storage()
        .instance()
        .extend_ttl(LEDGER_THRESHOLD_INSTANCE, LEDGER_BUMP_INSTANCE);
}

pub fn get_init_meta(e: &Env) -> FactoryInitMeta {
    e.storage()
        .instance()
        .get::<Symbol, FactoryInitMeta>(&Symbol::new(e, "InitMeta"))
        .unwrap_optimized()
}

pub fn set_init_meta(e: &Env, meta: &FactoryInitMeta) {
    e.storage()
        .instance()
        .set::<Symbol, FactoryInitMeta>(&Symbol::new(e, "InitMeta"), meta);
}

pub fn is_deployed(e: &Env, pool_id: &Address) -> bool {
    let key = FactoryDataKey::Pools(pool_id.clone());
    if let Some(result) = e
        .storage()
        .persistent()
        .get::<FactoryDataKey, bool>(&key)
    {
        e.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD_POOL, LEDGER_BUMP_POOL);
        result
    } else {
        false
    }
}

pub fn set_deployed(e: &Env, pool_id: &Address) {
    let key = FactoryDataKey::Pools(pool_id.clone());
    e.storage()
        .persistent()
        .set::<FactoryDataKey, bool>(&key, &true);
    e.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD_POOL, LEDGER_BUMP_POOL);
}
