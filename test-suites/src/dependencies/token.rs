pub(crate) mod token_contract_wasm {
    soroban_sdk::contractimport!(file = "../wasm/soroban_token_contract.wasm");
}
pub use token_contract_wasm::{WASM as TOKEN_WASM};