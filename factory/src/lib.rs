#![no_std]
#![allow(clippy::too_many_arguments)]

mod events;
mod storage;

#[cfg(test)]
mod test;

use events::Deploy;
pub use storage::FactoryInitMeta;

use soroban_sdk::{
    contract, contractclient, contractimpl,
    Address, Bytes, BytesN, Env, IntoVal, String,
};
use trading::TradingConfig;

/// Factory contract for atomic deployment of trading pools (trading + vault).
#[contract]
pub struct FactoryContract;

#[contractclient(name = "FactoryClient")]
pub trait Factory {
    /// Deploy a new trading pool: creates a strategy-vault and a trading contract atomically.
    ///
    /// WHY: Deterministic address precomputation resolves the circular dependency:
    /// the vault's `strategy` must point to the trading contract, and the trading
    /// contract's `vault` must point to the vault. By using `deployed_address()`,
    /// both addresses are known before either contract is deployed.
    ///
    /// # Deployment order
    /// 1. Compute deterministic salts from (admin, salt) for front-run protection
    /// 2. Precompute both addresses via `deployer.deployed_address()`
    /// 3. Deploy vault first (its constructor does NOT call trading)
    /// 4. Deploy trading second (it can call vault if needed during construction)
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

        // Compute deterministic salts with front-run protection
        let (trading_salt, vault_salt) = compute_salts(&e, &admin, &salt);

        // Precompute both addresses before deploying either contract
        let trading_deployer = e.deployer().with_current_contract(trading_salt);
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

        // Record deployments
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

/// Compute deterministic, front-run-resistant salts for vault and trading deployment.
///
/// WHY: The salt is derived from `keccak256(user_salt || admin_address || discriminator)`.
/// Including the admin address prevents front-running: an attacker cannot use the same
/// salt to deploy to the same address because their admin address differs.
/// The trailing byte (0 for trading, 1 for vault) ensures the two contracts get
/// different addresses even with the same admin and user salt.
fn compute_salts(e: &Env, admin: &Address, salt: &BytesN<32>) -> (BytesN<32>, BytesN<32>) {
    let mut admin_bytes: [u8; 56] = [0; 56];
    admin.to_string().copy_into_slice(&mut admin_bytes);

    let mut trading_salt_bytes: Bytes = salt.into_val(e);
    trading_salt_bytes.extend_from_array(&admin_bytes);
    trading_salt_bytes.push_back(0u8);
    let trading_salt = e.crypto().keccak256(&trading_salt_bytes);

    let mut vault_salt_bytes: Bytes = salt.into_val(e);
    vault_salt_bytes.extend_from_array(&admin_bytes);
    vault_salt_bytes.push_back(1u8);
    let vault_salt = e.crypto().keccak256(&vault_salt_bytes);

    (trading_salt.into(), vault_salt.into())
}
