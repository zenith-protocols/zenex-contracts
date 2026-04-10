#![no_std]
#![allow(clippy::too_many_arguments)]

mod events;
mod storage;

#[cfg(test)]
mod test;

use events::Deploy;
pub use storage::FactoryInitMeta;

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype,
    Address, BytesN, Env, String,
};

/// Mirrors trading::TradingConfig. Same XDR encoding on-chain.
#[contracttype]
#[derive(Clone, Debug)]
pub struct TradingConfig {
    pub caller_rate:  i128, // keeper's share of trading fees (SCALAR_7)
    pub min_notional: i128, // minimum notional per position (token_decimals)
    pub max_notional: i128, // maximum notional per position (token_decimals)
    pub fee_dom:      i128, // dominant-side trading fee rate (SCALAR_7)
    pub fee_non_dom:  i128, // non-dominant-side trading fee rate (SCALAR_7)
    pub max_util:     i128, // global utilization cap (SCALAR_7)
    pub r_funding:    i128, // base hourly funding rate (SCALAR_18)
    pub r_base:       i128, // base hourly borrowing rate (SCALAR_18)
    pub r_var:        i128, // vault-level variable borrowing rate (SCALAR_18)
}

/// Factory contract for atomic deployment of trading pools (trading + vault).
#[contract]
pub struct FactoryContract;

#[contractclient(name = "FactoryClient")]
pub trait Factory {
    /// Deploy a new trading pool: creates a strategy-vault and a trading contract atomically.
    ///
    /// # Parameters
    /// - `admin` - Owner of the new trading contract (must `require_auth`)
    /// - `salt` - User-provided salt for deterministic address derivation
    /// - `token` - Collateral token address
    /// - `price_verifier` - Pyth price verifier contract address
    /// - `config` - Global trading parameters
    /// - `vault_name` / `vault_symbol` - Vault share token metadata
    /// - `vault_decimals_offset` - Inflation attack protection offset (0-10)
    /// - `vault_lock_time` - Deposit lock duration in seconds
    ///
    /// # Returns
    /// Address of the newly deployed trading contract.
    fn deploy(
        e: Env,
        admin: Address,
        salt: BytesN<32>,
        token: Address,
        price_verifier: Address,
        config: TradingConfig,
        vault_name: String,
        vault_symbol: String,
        vault_decimals_offset: u32,
        vault_lock_time: u64,
    ) -> Address;

    /// Returns `true` if the given trading address was deployed by this factory.
    fn is_deployed(e: Env, trading: Address) -> bool;
}

#[contractimpl]
impl FactoryContract {
    /// Initialize the factory with compiled WASM hashes and the treasury address.
    ///
    /// # Parameters
    /// - `init_meta` - [`FactoryInitMeta`] containing `trading_hash`, `vault_hash`, and `treasury` address
    pub fn __constructor(e: Env, init_meta: FactoryInitMeta) {
        storage::set_init_meta(&e, &init_meta);
    }
}

#[contractimpl]
impl Factory for FactoryContract {
    fn deploy(
        e: Env,
        admin: Address,
        salt: BytesN<32>,
        token: Address,
        price_verifier: Address,
        config: TradingConfig,
        vault_name: String,
        vault_symbol: String,
        vault_decimals_offset: u32,
        vault_lock_time: u64,
    ) -> Address {
        admin.require_auth();
        storage::extend_instance(&e);
        let init_meta = storage::get_init_meta(&e);

        let mut vault_salt_bytes = salt.to_array();
        vault_salt_bytes[31] ^= 1;
        let vault_salt: BytesN<32> = BytesN::from_array(&e, &vault_salt_bytes);

        let trading_deployer = e.deployer().with_current_contract(salt);
        let vault_deployer = e.deployer().with_current_contract(vault_salt);
        let trading_address = trading_deployer.deployed_address();
        let vault_address = vault_deployer.deployed_address();

        // Deploy vault first (its constructor doesn't call trading)
        vault_deployer.deploy_v2(
            init_meta.vault_hash,
            (vault_name, vault_symbol, token.clone(), vault_decimals_offset, trading_address.clone(), vault_lock_time),
        );

        // Deploy trading (vault is already live so cross-contract calls work)
        trading_deployer.deploy_v2(
            init_meta.trading_hash,
            (admin.clone(), token, vault_address.clone(), price_verifier, init_meta.treasury, config),
        );

        storage::set_deployed(&e, &trading_address);

        Deploy {
            trading: trading_address.clone(),
            vault: vault_address.clone(),
        }.publish(&e);
        trading_address
    }

    fn is_deployed(e: Env, trading: Address) -> bool {
        storage::extend_instance(&e);
        storage::is_deployed(&e, &trading)
    }
}
