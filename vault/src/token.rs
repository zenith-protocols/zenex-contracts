use soroban_sdk::{Address, Bytes, BytesN, Env, String};
use soroban_sdk::xdr::ToXdr;

pub fn create_share_token(
    e: &Env,
    token_wasm_hash: BytesN<32>,
    asset: &Address,
    name: &String,
    symbol: &String,
) -> Address {
    let mut salt = Bytes::new(e);
    salt.append(&asset.to_xdr(e));
    salt.append(&name.clone().to_xdr(e));
    salt.append(&symbol.clone().to_xdr(e));
    let salt = e.crypto().sha256(&salt);

    e.deployer().with_current_contract(salt).deploy_v2(
        token_wasm_hash,
        (
            e.current_contract_address(), // admin
            7u32,                         // decimals
            name.clone(),                 // name
            symbol.clone(),               // symbol
        ),
    )
}