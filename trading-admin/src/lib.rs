#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, contracterror, panic_with_error,
    Address, Env,
};
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_contract_utils::upgradeable::UpgradeableInternal;
use stellar_macros::{only_owner, Upgradeable};
use trading::{MarketConfig, TradingClient, TradingConfig};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AdminError {
    Unauthorized = 1,
    UpdateNotQueued = 2,
    UpdateNotUnlocked = 3,
}

#[derive(Clone)]
#[contracttype]
pub enum AdminStorageKey {
    Trading,
    Delay,
    ConfigUpdate,
    MarketUpdate(u32),    // keyed by a nonce
    MarketNonce,
}

#[contracttype]
#[derive(Clone)]
pub struct QueuedConfig {
    pub config: TradingConfig,
    pub unlock_time: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct QueuedMarket {
    pub config: MarketConfig,
    pub unlock_time: u64,
    pub nonce: u32,
}

/********** Ledger Thresholds **********/

const ONE_DAY_LEDGERS: u32 = 17280;
const LEDGER_THRESHOLD_TEMP: u32 = ONE_DAY_LEDGERS * 100;
const LEDGER_BUMP_TEMP: u32 = LEDGER_THRESHOLD_TEMP + 20 * ONE_DAY_LEDGERS;

#[derive(Upgradeable)]
#[contract]
pub struct TradingAdminContract;

#[contractclient(name = "TradingAdminClient")]
pub trait TradingAdmin {
    /// (Owner only) Queue a config update for the trading contract
    fn queue_set_config(e: Env, config: TradingConfig);

    /// (Owner only) Cancel a queued config update
    fn cancel_set_config(e: Env);

    /// Apply a queued config update after the delay has passed
    fn set_config(e: Env);

    /// (Owner only) Queue a new market for the trading contract
    fn queue_set_market(e: Env, config: MarketConfig) -> u32;

    /// (Owner only) Cancel a queued market
    fn cancel_set_market(e: Env, nonce: u32);

    /// Apply a queued market after the delay has passed
    fn set_market(e: Env, nonce: u32);

    /// (Owner only) Set the status on the trading contract (immediate, no delay)
    fn set_status(e: Env, status: u32);

    /// Get the trading contract address
    fn get_trading(e: Env) -> Address;

    /// Get the configured delay in seconds
    fn get_delay(e: Env) -> u64;

    /// Get a queued config update (if any)
    fn get_queued_config(e: Env) -> QueuedConfig;

    /// Get a queued market by nonce
    fn get_queued_market(e: Env, nonce: u32) -> QueuedMarket;
}

#[contractimpl]
impl TradingAdminContract {
    pub fn __constructor(e: Env, owner: Address, trading: Address, delay: u64) {
        ownable::set_owner(&e, &owner);
        e.storage().instance().set(&AdminStorageKey::Trading, &trading);
        e.storage().instance().set(&AdminStorageKey::Delay, &delay);
    }
}

#[contractimpl]
impl TradingAdmin for TradingAdminContract {
    #[only_owner]
    fn queue_set_config(e: Env, config: TradingConfig) {
        let delay: u64 = e.storage().instance().get(&AdminStorageKey::Delay).unwrap();
        let unlock_time = e.ledger().timestamp() + delay;
        let queued = QueuedConfig {
            config,
            unlock_time,
        };
        let key = AdminStorageKey::ConfigUpdate;
        e.storage().temporary().set(&key, &queued);
        e.storage().temporary().extend_ttl(&key, LEDGER_THRESHOLD_TEMP, LEDGER_BUMP_TEMP);
    }

    #[only_owner]
    fn cancel_set_config(e: Env) {
        let key = AdminStorageKey::ConfigUpdate;
        if !e.storage().temporary().has(&key) {
            panic_with_error!(&e, AdminError::UpdateNotQueued);
        }
        e.storage().temporary().remove(&key);
    }

    fn set_config(e: Env) {
        let key = AdminStorageKey::ConfigUpdate;
        let queued: QueuedConfig = e.storage().temporary().get(&key)
            .unwrap_or_else(|| panic_with_error!(&e, AdminError::UpdateNotQueued));

        if queued.unlock_time > e.ledger().timestamp() {
            panic_with_error!(&e, AdminError::UpdateNotUnlocked);
        }

        let trading: Address = e.storage().instance().get(&AdminStorageKey::Trading).unwrap();
        TradingClient::new(&e, &trading).set_config(&queued.config);
        e.storage().temporary().remove(&key);
    }

    #[only_owner]
    fn queue_set_market(e: Env, config: MarketConfig) -> u32 {
        let delay: u64 = e.storage().instance().get(&AdminStorageKey::Delay).unwrap();
        let unlock_time = e.ledger().timestamp() + delay;
        let nonce = next_market_nonce(&e);
        let queued = QueuedMarket {
            config,
            unlock_time,
            nonce,
        };
        let key = AdminStorageKey::MarketUpdate(nonce);
        e.storage().temporary().set(&key, &queued);
        e.storage().temporary().extend_ttl(&key, LEDGER_THRESHOLD_TEMP, LEDGER_BUMP_TEMP);
        nonce
    }

    #[only_owner]
    fn cancel_set_market(e: Env, nonce: u32) {
        let key = AdminStorageKey::MarketUpdate(nonce);
        if !e.storage().temporary().has(&key) {
            panic_with_error!(&e, AdminError::UpdateNotQueued);
        }
        e.storage().temporary().remove(&key);
    }

    fn set_market(e: Env, nonce: u32) {
        let key = AdminStorageKey::MarketUpdate(nonce);
        let queued: QueuedMarket = e.storage().temporary().get(&key)
            .unwrap_or_else(|| panic_with_error!(&e, AdminError::UpdateNotQueued));

        if queued.unlock_time > e.ledger().timestamp() {
            panic_with_error!(&e, AdminError::UpdateNotUnlocked);
        }

        let trading: Address = e.storage().instance().get(&AdminStorageKey::Trading).unwrap();
        TradingClient::new(&e, &trading).set_market(&queued.config);
        e.storage().temporary().remove(&key);
    }

    #[only_owner]
    fn set_status(e: Env, status: u32) {
        let trading: Address = e.storage().instance().get(&AdminStorageKey::Trading).unwrap();
        TradingClient::new(&e, &trading).set_status(&status);
    }

    fn get_trading(e: Env) -> Address {
        e.storage().instance().get(&AdminStorageKey::Trading).unwrap()
    }

    fn get_delay(e: Env) -> u64 {
        e.storage().instance().get(&AdminStorageKey::Delay).unwrap()
    }

    fn get_queued_config(e: Env) -> QueuedConfig {
        e.storage().temporary().get(&AdminStorageKey::ConfigUpdate)
            .unwrap_or_else(|| panic_with_error!(&e, AdminError::UpdateNotQueued))
    }

    fn get_queued_market(e: Env, nonce: u32) -> QueuedMarket {
        e.storage().temporary().get(&AdminStorageKey::MarketUpdate(nonce))
            .unwrap_or_else(|| panic_with_error!(&e, AdminError::UpdateNotQueued))
    }
}

fn next_market_nonce(e: &Env) -> u32 {
    let key = AdminStorageKey::MarketNonce;
    let current: u32 = e.storage().instance().get(&key).unwrap_or(0);
    e.storage().instance().set(&key, &(current + 1));
    current
}

#[contractimpl(contracttrait)]
impl Ownable for TradingAdminContract {}

impl UpgradeableInternal for TradingAdminContract {
    fn _require_auth(e: &Env, operator: &Address) {
        operator.require_auth();
        let owner = ownable::get_owner(e).expect("owner not set");
        if *operator != owner {
            panic_with_error!(e, AdminError::Unauthorized)
        }
    }
}
