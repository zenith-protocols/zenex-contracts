//! Integration tests for factory deployment and price verifier.
//!
//! These tests deploy the full contract stack (trading, vault, price-verifier,
//! treasury) using real contracts (no mocks). Price data is constructed with
//! real Ed25519 signatures via `pyth_helper`.

use ed25519_dalek::SigningKey;
use price_verifier::{PriceVerifier, PriceVerifierClient};
use soroban_sdk::testutils::{Address as _, BytesN as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, BytesN, Env, String};
use test_suites::pyth_helper;
use test_suites::test_fixture::TestFixture;
use trading::testutils::default_config;
use trading::TradingClient;
use treasury::TreasuryContract;
use factory::{FactoryClient, FactoryContract, FactoryInitMeta};

const TRADING_WASM: &[u8] =
    include_bytes!("../../target/wasm32v1-none/release/trading.wasm");
const VAULT_WASM: &[u8] =
    include_bytes!("../../target/wasm32v1-none/release/strategy_vault.wasm");

/// Helper: deploy a real PriceVerifier via native struct registration.
fn deploy_price_verifier(
    e: &Env,
    owner: &Address,
    pubkey_bytes: &[u8; 32],
    max_confidence_bps: u32,
    max_staleness: u64,
) -> (Address, PriceVerifierClient<'static>) {
    let pv_id = e.register(
        PriceVerifier,
        (
            owner,
            BytesN::<32>::from_array(e, pubkey_bytes),
            max_confidence_bps,
            max_staleness,
        ),
    );
    let client = PriceVerifierClient::new(e, &pv_id);
    (pv_id, client)
}

/// Helper: deploy a real Treasury via native struct registration.
fn deploy_treasury(e: &Env, owner: &Address, rate: i128) -> Address {
    e.register(TreasuryContract, (owner, rate))
}

// =========================================================================
//  Factory Deployment Tests (TEST-06)
// =========================================================================

#[test]
fn test_factory_deploy_v2_creates_trading_and_vault() {
    let fixture = TestFixture::create();

    // Verify trading address is valid and has a vault
    let vault_addr = fixture.trading.get_vault();
    assert_ne!(fixture.trading.address, vault_addr);

    // Verify vault holds the correct underlying asset (the token)
    let vault_asset = fixture.vault.query_asset();
    assert_eq!(vault_asset, fixture.token.address);

    // Verify trading knows about the vault
    assert_eq!(fixture.trading.get_vault(), fixture.vault.address);

    // Verify trading has the correct price verifier
    assert_eq!(
        fixture.trading.get_price_verifier(),
        fixture.price_verifier.address
    );

    // Verify factory tracks the deployment
    assert!(fixture.factory.is_deployed(&fixture.trading.address));
}

#[test]
fn test_factory_deploy_deterministic_address_prediction() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();
    e.mock_all_auths_allowing_non_root_auth();

    let owner = Address::generate(&e);
    let token_addr = e
        .register_stellar_asset_contract_v2(owner.clone())
        .address();

    // Deploy real PriceVerifier and Treasury
    let (_signing_key, pubkey_bytes) = pyth_helper::test_keypair();
    let (pv_id, _pv_client) = deploy_price_verifier(&e, &owner, &pubkey_bytes, 200, 60);
    let treasury_id = deploy_treasury(&e, &owner, 500_000);

    // Set up factory
    let trading_hash = e.deployer().upload_contract_wasm(TRADING_WASM);
    let vault_hash = e.deployer().upload_contract_wasm(VAULT_WASM);
    let init_meta = FactoryInitMeta {
        trading_hash,
        vault_hash,
        treasury: treasury_id,
    };
    let factory_id = e.register(FactoryContract {}, (init_meta,));
    let factory_client = FactoryClient::new(&e, &factory_id);

    let config = default_config();

    // Deploy with salt1
    let salt1 = BytesN::<32>::random(&e);
    let trading_1 = factory_client.deploy(
        &owner,
        &salt1,
        &token_addr,
        &pv_id,
        &config,
        &String::from_str(&e, "Zenex LP"),
        &String::from_str(&e, "zLP"),
        &0u32,
        &300u64,
    );

    // Deploy with salt2 (different salt = different address)
    let salt2 = BytesN::<32>::random(&e);
    let trading_2 = factory_client.deploy(
        &owner,
        &salt2,
        &token_addr,
        &pv_id,
        &config,
        &String::from_str(&e, "Zenex LP 2"),
        &String::from_str(&e, "zLP2"),
        &0u32,
        &300u64,
    );

    // Different salts produce different addresses
    assert_ne!(trading_1, trading_2);

    // Both are tracked by factory
    assert!(factory_client.is_deployed(&trading_1));
    assert!(factory_client.is_deployed(&trading_2));

    // Unrelated address is not tracked
    assert!(!factory_client.is_deployed(&Address::generate(&e)));
}

#[test]
fn test_factory_deploy_vault_decimals_offset() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();
    e.mock_all_auths_allowing_non_root_auth();

    let owner = Address::generate(&e);
    let token_addr = e
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let token_client = StellarAssetClient::new(&e, &token_addr);

    // Deploy PriceVerifier and Treasury
    let (_signing_key, pubkey_bytes) = pyth_helper::test_keypair();
    let (pv_id, _pv_client) = deploy_price_verifier(&e, &owner, &pubkey_bytes, 200, 60);
    let treasury_id = deploy_treasury(&e, &owner, 500_000);

    // Set up factory
    let trading_hash = e.deployer().upload_contract_wasm(TRADING_WASM);
    let vault_hash = e.deployer().upload_contract_wasm(VAULT_WASM);
    let init_meta = FactoryInitMeta {
        trading_hash,
        vault_hash,
        treasury: treasury_id,
    };
    let factory_id = e.register(FactoryContract {}, (init_meta,));
    let factory_client = FactoryClient::new(&e, &factory_id);

    let config = default_config();

    // Deploy with decimals_offset=2 (inflation attack protection)
    let salt = BytesN::<32>::random(&e);
    let trading_id = factory_client.deploy(
        &owner,
        &salt,
        &token_addr,
        &pv_id,
        &config,
        &String::from_str(&e, "Zenex LP"),
        &String::from_str(&e, "zLP"),
        &2u32, // decimals_offset=2
        &300u64,
    );

    let trading_client = TradingClient::new(&e, &trading_id);
    let vault_id = trading_client.get_vault();

    // Mint tokens and deposit into vault
    token_client.mint(&owner, &1_000_0000000);
    let vault_client = test_suites::dependencies::vault::VaultClient::new(&e, &vault_id);
    let shares = vault_client.deposit(&1_000_0000000, &owner, &owner, &owner);

    // With decimals_offset=2, the vault works correctly (shares returned)
    assert!(shares > 0);
    assert_eq!(vault_client.total_assets(), 1_000_0000000);
}

// =========================================================================
//  Price Verifier Integration Tests (TEST-07)
// =========================================================================

#[test]
fn test_price_verifier_real_signature_verification() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();
    e.mock_all_auths_allowing_non_root_auth();

    let owner = Address::generate(&e);
    let (signing_key, pubkey_bytes) = pyth_helper::test_keypair();
    let (_pv_id, pv_client) = deploy_price_verifier(&e, &owner, &pubkey_bytes, 200, 60);

    // Set ledger timestamp to match our signed payload
    let timestamp = 1000u64;
    e.ledger().with_mut(|li| li.timestamp = timestamp);

    // Build a signed BTC price update
    let price_bytes =
        pyth_helper::btc_price_update(&e, &signing_key, 10_000_000_000_000, timestamp);

    // Verify the price using the real PriceVerifier contract
    let price_data = pv_client.verify_price(&price_bytes);

    assert_eq!(price_data.feed_id, 1);
    assert_eq!(price_data.price, 10_000_000_000_000_i128);
    assert_eq!(price_data.exponent, -8);
    assert_eq!(price_data.publish_time, timestamp);
}

#[test]
#[should_panic(expected = "Error(Contract, #800)")]
fn test_price_verifier_wrong_signer_rejected() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let (_signing_key, pubkey_bytes) = pyth_helper::test_keypair();

    // Register PriceVerifier with the CORRECT pubkey
    let (_pv_id, pv_client) = deploy_price_verifier(&e, &owner, &pubkey_bytes, 200, 60);

    let timestamp = 1000u64;
    e.ledger().with_mut(|li| li.timestamp = timestamp);

    // Sign with a DIFFERENT key (wrong signer)
    let wrong_seed: [u8; 32] = [
        99, 98, 97, 96, 95, 94, 93, 92, 91, 90, 89, 88, 87, 86, 85, 84, 83, 82, 81, 80, 79, 78,
        77, 76, 75, 74, 73, 72, 71, 70, 69, 68,
    ];
    let wrong_key = SigningKey::from_bytes(&wrong_seed);

    let price_bytes =
        pyth_helper::btc_price_update(&e, &wrong_key, 10_000_000_000_000, timestamp);

    // Should panic: the pubkey in the blob won't match trusted_signer
    pv_client.verify_price(&price_bytes);
}

#[test]
#[should_panic(expected = "Error(Contract, #820)")]
fn test_price_verifier_stale_price_rejected() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let (signing_key, pubkey_bytes) = pyth_helper::test_keypair();

    let max_staleness = 60u64;
    let (_pv_id, pv_client) =
        deploy_price_verifier(&e, &owner, &pubkey_bytes, 200, max_staleness);

    // Build price with current timestamp
    let timestamp = 1000u64;
    let price_bytes =
        pyth_helper::btc_price_update(&e, &signing_key, 10_000_000_000_000, timestamp);

    // Jump ledger forward past max_staleness
    e.ledger()
        .with_mut(|li| li.timestamp = timestamp + max_staleness + 10);

    // Should panic with PriceStale error (#820)
    pv_client.verify_price(&price_bytes);
}

#[test]
fn test_price_verifier_multi_feed_verification() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let (signing_key, pubkey_bytes) = pyth_helper::test_keypair();
    let (_pv_id, pv_client) = deploy_price_verifier(&e, &owner, &pubkey_bytes, 200, 60);

    let timestamp = 1000u64;
    e.ledger().with_mut(|li| li.timestamp = timestamp);

    // Build BTC + ETH + XLM multi-feed update
    let btc = 10_000_000_000_000i64; // $100k
    let eth = 200_000_000_000i64; // $2k
    let xlm = 10_000_000i64; // $0.10
    let price_bytes = pyth_helper::multi_price_update(&e, &signing_key, btc, eth, xlm, timestamp);

    let feeds = pv_client.verify_prices(&price_bytes);

    assert_eq!(feeds.len(), 3);

    let btc_data = feeds.get(0).unwrap();
    assert_eq!(btc_data.feed_id, 1);
    assert_eq!(btc_data.price, btc as i128);
    assert_eq!(btc_data.exponent, -8);

    let eth_data = feeds.get(1).unwrap();
    assert_eq!(eth_data.feed_id, 2);
    assert_eq!(eth_data.price, eth as i128);
    assert_eq!(eth_data.exponent, -8);

    let xlm_data = feeds.get(2).unwrap();
    assert_eq!(xlm_data.feed_id, 3);
    assert_eq!(xlm_data.price, xlm as i128);
    assert_eq!(xlm_data.exponent, -8);
}

#[test]
#[should_panic(expected = "Error(Contract, #810)")]
fn test_price_verifier_confidence_too_wide_rejected() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let (signing_key, pubkey_bytes) = pyth_helper::test_keypair();

    // max_confidence_bps = 200 means max 2% confidence interval
    let (_pv_id, pv_client) = deploy_price_verifier(&e, &owner, &pubkey_bytes, 200, 60);

    let timestamp = 1000u64;
    e.ledger().with_mut(|li| li.timestamp = timestamp);

    // Confidence check: raw_conf * 10_000 > raw_price.abs() * max_confidence_bps
    // With price=100_000, max_confidence_bps=200: threshold = 100_000 * 200 = 20_000_000
    // So confidence of 3000: 3000 * 10_000 = 30_000_000 > 20_000_000 --> rejected
    let price_bytes = pyth_helper::price_with_confidence(
        &e,
        &signing_key,
        1,       // feed_id
        100_000, // price
        -8,      // exponent
        3000,    // confidence (3% > 2% max)
        timestamp,
    );

    // Should panic with InvalidPrice error (#810)
    pv_client.verify_price(&price_bytes);
}

#[test]
fn test_price_verifier_confidence_within_bounds_accepted() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let (signing_key, pubkey_bytes) = pyth_helper::test_keypair();
    let (_pv_id, pv_client) = deploy_price_verifier(&e, &owner, &pubkey_bytes, 200, 60);

    let timestamp = 1000u64;
    e.ledger().with_mut(|li| li.timestamp = timestamp);

    // Confidence within bounds: 1000 * 10_000 = 10_000_000 <= 100_000 * 200 = 20_000_000
    let price_bytes = pyth_helper::price_with_confidence(
        &e,
        &signing_key,
        1,       // feed_id
        100_000, // price
        -8,      // exponent
        1000,    // confidence (1% < 2% max) -- accepted
        timestamp,
    );

    let price_data = pv_client.verify_price(&price_bytes);
    assert_eq!(price_data.feed_id, 1);
    assert_eq!(price_data.price, 100_000);
}

#[test]
fn test_fixture_full_stack_price_verification() {
    // End-to-end: TestFixture deploys full stack, price_update() works
    let fixture = TestFixture::create();

    // Mint tokens and seed vault with liquidity
    fixture.token.mint(&fixture.owner, &100_000_000_0000000);
    fixture.vault.deposit(
        &100_000_000_0000000,
        &fixture.owner,
        &fixture.owner,
        &fixture.owner,
    );

    // Create BTC market
    let btc_config = trading::testutils::default_market(&fixture.env);
    fixture.create_market(1, &btc_config);

    // Verify price_for_feed produces a verifiable price
    let price_bytes = fixture.price_for_feed(1, 10_000_000_000_000);
    let price_data = fixture.price_verifier.verify_price(&price_bytes);
    assert_eq!(price_data.feed_id, 1);
    assert_eq!(price_data.price, 10_000_000_000_000_i128);

    // Verify default_prices produces multi-feed verifiable prices
    let multi_bytes = fixture.default_prices();
    let feeds = fixture.price_verifier.verify_prices(&multi_bytes);
    assert_eq!(feeds.len(), 3);
}
