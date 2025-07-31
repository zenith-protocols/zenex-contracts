pub(crate) mod vault_contract_wasm {
    soroban_sdk::contractimport!(file = "../wasm/vault.wasm");
}

pub use vault_contract_wasm::{Client as VaultClient, WASM as VAULT_WASM};