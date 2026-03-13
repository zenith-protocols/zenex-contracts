use crate::storage::ZenexInitMeta;
use crate::{ZenexFactoryClient, ZenexFactoryContract};

use sep_41_token::testutils::{MockTokenClient, MockTokenWASM};
use soroban_sdk::{
    testutils::{Address as _, BytesN as _},
    Address, BytesN, Env, IntoVal, String,
};
use trading::testutils::default_config;

const TRADING_WASM: &[u8] =
    include_bytes!("../../target/wasm32v1-none/release/trading.wasm");
const VAULT_WASM: &[u8] =
    include_bytes!("../../target/wasm32v1-none/release/strategy_vault.wasm");

fn setup_factory(e: &Env) -> (Address, ZenexFactoryClient<'_>) {
    let trading_hash = e.deployer().upload_contract_wasm(TRADING_WASM);
    let vault_hash = e.deployer().upload_contract_wasm(VAULT_WASM);
    let treasury = Address::generate(e);
    let init_meta = ZenexInitMeta {
        trading_hash,
        vault_hash,
        treasury,
    };
    let address = e.register(ZenexFactoryContract {}, (init_meta,));
    let client = ZenexFactoryClient::new(e, &address);
    (address, client)
}

fn create_token(e: &Env, admin: &Address) -> Address {
    let address = Address::generate(e);
    e.register_at(&address, MockTokenWASM, ());
    let client = MockTokenClient::new(e, &address);
    client.initialize(admin, &7, &"USDC".into_val(e), &"USDC".into_val(e));
    address
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

    let (trading_address, vault_address) = factory.deploy(
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
    assert!(factory.is_pool(&trading_address));
    assert!(!factory.is_pool(&Address::generate(&e)));

    // Verify second deploy with different salt produces different addresses
    let salt2 = BytesN::<32>::random(&e);
    let (trading_2, vault_2) = factory.deploy(
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
    assert_ne!(vault_address, vault_2);
    assert!(factory.is_pool(&trading_2));
}

#[test]
fn test_factory_frontrun_protection() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();
    e.mock_all_auths_allowing_non_root_auth();

    let (_factory_address, factory) = setup_factory(&e);
    let admin1 = Address::generate(&e);
    let admin2 = Address::generate(&e);
    let token = create_token(&e, &admin1);
    let price_verifier = Address::generate(&e);
    let salt = BytesN::<32>::random(&e);

    let (trading_1, _) = factory.deploy(
        &admin1,
        &salt,
        &token,
        &price_verifier,
        &default_config(),
        &String::from_str(&e, "LP 1"),
        &String::from_str(&e, "zLP1"),
        &0u32,
        &300u64,
    );

    // Same salt, different admin → different address
    let (trading_2, _) = factory.deploy(
        &admin2,
        &salt,
        &token,
        &price_verifier,
        &default_config(),
        &String::from_str(&e, "LP 2"),
        &String::from_str(&e, "zLP2"),
        &0u32,
        &300u64,
    );

    assert_ne!(trading_1, trading_2);
}
