use sep_41_token::testutils::{MockTokenClient, MockTokenWASM};
use soroban_sdk::{Address, Env, IntoVal};

pub fn create_stellar_token<'a>(e: &Env, admin: &Address) -> (Address, MockTokenClient<'a>) {
    let contract_id = e.register_stellar_asset_contract_v2(admin.clone()).address();
    let client = MockTokenClient::new(e, &contract_id);
    // set admin to bump instance
    client.set_admin(admin);
    (contract_id, client)
}

#[allow(dead_code)]
pub fn create_token<'a>(
    e: &Env,
    admin: &Address,
    decimals: u32,
    symbol: &str,
) -> (Address, MockTokenClient<'a>) {
    let contract_id = e.register(MockTokenWASM, ());
    let client = MockTokenClient::new(e, &contract_id);
    client.initialize(
        admin,
        &decimals,
        &"test token".into_val(e),
        &symbol.into_val(e),
    );
    // set admin to bump instance
    client.set_admin(admin);
    (contract_id, client)
}