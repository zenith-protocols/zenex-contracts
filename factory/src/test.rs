use crate::storage::FactoryInitMeta;
use crate::{FactoryClient, FactoryContract};

use soroban_sdk::{
    testutils::{Address as _, BytesN as _},
    token::StellarAssetClient,
    Address, BytesN, Env, String,
};
use trading::testutils::default_config;

const TRADING_WASM: &[u8] =
    include_bytes!("../../target/wasm32v1-none/release/trading.wasm");
const VAULT_WASM: &[u8] =
    include_bytes!("../../target/wasm32v1-none/release/strategy_vault.wasm");

fn setup_factory(e: &Env) -> (Address, FactoryClient<'_>) {
    let trading_hash = e.deployer().upload_contract_wasm(TRADING_WASM);
    let vault_hash = e.deployer().upload_contract_wasm(VAULT_WASM);
    let treasury = Address::generate(e);
    let init_meta = FactoryInitMeta {
        trading_hash,
        vault_hash,
        treasury,
    };
    let address = e.register(FactoryContract {}, (init_meta,));
    let client = FactoryClient::new(e, &address);
    (address, client)
}

fn create_token(e: &Env, admin: &Address) -> Address {
    e.register_stellar_asset_contract_v2(admin.clone()).address()
}

#[test]
fn test_factory_deploy() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();
    e.mock_all_auths_allowing_non_root_auth();

    let (_factory_address, factory) = setup_factory(&e);
    let admin = Address::generate(&e);
    let token = create_token(&e, &admin);
    let price_verifier = Address::generate(&e); // mock, won't be called during deploy
    let salt = BytesN::<32>::random(&e);

    let trading_address = factory.deploy(
        &admin,
        &salt,
        &token,
        &price_verifier,
        &default_config(),
        &String::from_str(&e, "Zenex LP"),
        &String::from_str(&e, "zLP"),
        &0u32,
        &300u64,
    );

    // Verify the deployed addresses are valid and tracked
    assert!(factory.is_deployed(&trading_address));
    assert!(!factory.is_deployed(&Address::generate(&e)));

    // Verify second deploy with different salt produces different addresses
    let salt2 = BytesN::<32>::random(&e);
    let trading_2 = factory.deploy(
        &admin,
        &salt2,
        &token,
        &price_verifier,
        &default_config(),
        &String::from_str(&e, "Zenex LP 2"),
        &String::from_str(&e, "zLP2"),
        &0u32,
        &300u64,
    );
    assert_ne!(trading_address, trading_2);
    assert!(factory.is_deployed(&trading_2));
}
