#![no_std]
#![allow(clippy::too_many_arguments)]

mod events;
mod storage;

#[cfg(test)]
mod test;

use events::Deploy;
pub use storage::ZenexInitMeta;

use soroban_sdk::{
    contract, contractclient, contractimpl,
    Address, Bytes, BytesN, Env, IntoVal, String,
};
use trading::TradingConfig;

#[contract]
pub struct ZenexFactoryContract;

#[contractclient(name = "ZenexFactoryClient")]
pub trait ZenexFactory {
    /// Deploy a new trading pool (trading contract + strategy vault)
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
    ) -> (Address, Address);

    /// Check if a trading contract was deployed by this factory
    fn is_pool(e: Env, pool_id: Address) -> bool;
}

#[contractimpl]
impl ZenexFactoryContract {
    pub fn __constructor(e: Env, init_meta: ZenexInitMeta) {
        storage::set_init_meta(&e, &init_meta);
    }
}

#[contractimpl]
impl ZenexFactory for ZenexFactoryContract {
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
    ) -> (Address, Address) {
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
        (trading_address, vault_address)
    }

    fn is_pool(e: Env, pool_id: Address) -> bool {
        storage::extend_instance(&e);
        storage::is_deployed(&e, &pool_id)
    }
}

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
