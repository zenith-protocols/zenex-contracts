//! Timelock (governance) integration tests with real trading contract.
//!
//! Exercises the governance contract's queue/execute/cancel flows end-to-end.
//! Each test explicitly deploys a real trading contract and governance contract,
//! sets governance as the trading owner, then verifies the timelock mechanism.
//!
//! Ownership transfer follows stellar-access's 2-step pattern:
//! 1. Current owner calls `transfer_ownership(new_owner, live_until_ledger)` on trading
//! 2. New owner calls `accept_ownership()` on trading
//! Because these come from the `Ownable` trait (not the `Trading` trait), we
//! invoke them via `env.invoke_contract`.

use soroban_sdk::testutils::{Address as _, BytesN as _, Ledger, LedgerInfo};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{vec as svec, Address, BytesN, Env, IntoVal, String, Symbol, Val, Vec};
use test_suites::constants::SCALAR_7;
use trading::testutils::{
    default_config, default_market, MockPriceVerifier, MockPriceVerifierClient, MockTreasury,
    FEED_BTC, BTC_PRICE,
};
use trading::TradingClient;

const WASM_TRADING: &[u8] = include_bytes!("../../target/wasm32v1-none/release/trading.wasm");
const WASM_VAULT: &[u8] =
    include_bytes!("../../target/wasm32v1-none/release/strategy_vault.wasm");

/// Timelock delay used across tests (1 day = 86400 seconds).
const DELAY: u64 = 86400;

// ================================================================
// Helpers
// ================================================================

/// A self-contained test environment with trading, governance, and support contracts.
#[allow(dead_code)]
struct GovernanceFixture<'a> {
    env: Env,
    owner: Address,
    trading: TradingClient<'a>,
    gov: governance::GovernanceClient<'a>,
}

fn create_token<'a>(e: &Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
    let addr = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let client = StellarAssetClient::new(e, &addr);
    (addr, client)
}

/// Deploy the full stack with governance as trading owner.
///
/// Steps:
/// 1. Deploy mock price verifier + mock treasury
/// 2. Deploy vault + trading via factory (owner = `admin`)
/// 3. Deploy governance with `admin` as owner
/// 4. Transfer trading ownership from `admin` to governance (2-step)
fn setup<'a>() -> GovernanceFixture<'a> {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();
    e.mock_all_auths_allowing_non_root_auth();

    // Set a reasonable starting timestamp
    e.ledger().set(LedgerInfo {
        timestamp: 1_000_000,
        protocol_version: 25,
        sequence_number: 100_000,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 999_999,
        min_persistent_entry_ttl: 999_999,
        max_entry_ttl: 9_999_999,
    });

    let admin = Address::generate(&e);
    let (token_id, token_client) = create_token(&e, &admin);

    // Mock price verifier
    let pv_id = e.register(MockPriceVerifier, ());
    let pv_client = MockPriceVerifierClient::new(&e, &pv_id);
    pv_client.set_price(&(FEED_BTC), &BTC_PRICE);

    // Mock treasury
    let treasury_id = e.register(MockTreasury, ());

    // Deploy via factory for realistic deployment
    let trading_hash = e.deployer().upload_contract_wasm(WASM_TRADING);
    let vault_hash = e.deployer().upload_contract_wasm(WASM_VAULT);
    let init_meta = factory::FactoryInitMeta {
        trading_hash,
        vault_hash,
        treasury: treasury_id,
    };
    let factory_id = e.register(factory::FactoryContract {}, (init_meta,));
    let factory_client = factory::FactoryClient::new(&e, &factory_id);

    let config = test_suites::to_factory_config(&default_config());
    let salt = BytesN::<32>::random(&e);
    let trading_id = factory_client.deploy(
        &admin,
        &salt,
        &token_id,
        &pv_id,
        &config,
        &String::from_str(&e, "Zenex LP"),
        &String::from_str(&e, "zLP"),
        &0u32,
        &300u64,
    );
    let trading_client = TradingClient::new(&e, &trading_id);

    // Fund the vault so markets can be configured
    token_client.mint(&admin, &(100_000_000 * SCALAR_7));
    let vault_id = trading_client.get_vault();
    // Deposit into vault using the vault client interface
    let vault_client =
        test_suites::dependencies::vault::VaultClient::new(&e, &vault_id);
    vault_client.deposit(
        &(100_000_000 * SCALAR_7),
        &admin,
        &admin,
        &admin,
    );

    // Create a BTC market on trading
    let market_config = default_market(&e);
    trading_client.set_market(&(FEED_BTC), &market_config);

    // Deploy governance with admin as owner (generic timelock, no trading address at construction)
    let gov_id = e.register(
        governance::GovernanceContract,
        (admin.clone(), DELAY),
    );
    let gov_client = governance::GovernanceClient::new(&e, &gov_id);

    // Transfer trading ownership from admin to governance (2-step)
    // Step 1: admin initiates transfer
    let far_future_ledger = e.ledger().sequence() + 1_000_000;
    let _: () = e.invoke_contract(
        &trading_id,
        &Symbol::new(&e, "transfer_ownership"),
        svec![
            &e,
            gov_id.clone().into_val(&e),
            far_future_ledger.into_val(&e),
        ],
    );

    // Step 2: governance accepts
    let _: () = e.invoke_contract(
        &trading_id,
        &Symbol::new(&e, "accept_ownership"),
        svec![&e],
    );

    // Verify governance is now the owner
    let owner_opt: Option<Address> = e.invoke_contract(
        &trading_id,
        &Symbol::new(&e, "get_owner"),
        svec![&e],
    );
    assert_eq!(owner_opt, Some(gov_id.clone()), "governance should be trading owner");

    GovernanceFixture {
        env: e,
        owner: admin,
        trading: trading_client,
        gov: gov_client,
    }
}

/// Advance the ledger timestamp by `secs` seconds.
fn jump(e: &Env, secs: u64) {
    e.ledger().set(LedgerInfo {
        timestamp: e.ledger().timestamp().saturating_add(secs),
        protocol_version: 25,
        sequence_number: e.ledger().sequence().saturating_add((secs / 5) as u32),
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 999_999,
        min_persistent_entry_ttl: 999_999,
        max_entry_ttl: 9_999_999,
    });
}

// ================================================================
// Tests
// ================================================================

/// Queue a set_config via the generic timelock, wait past delay, execute.
/// Verify trading config changed.
#[test]
fn test_timelock_queue_and_execute_set_config() {
    let f = setup();

    // Prepare a modified config with a different r_base
    let mut new_config = default_config();
    new_config.r_base = 20_000_000_000_000; // doubled from default

    // Queue the config change via generic queue(target, fn_name, args)
    let args: Vec<Val> = Vec::from_array(&f.env, [new_config.clone().into_val(&f.env)]);
    let nonce = f.gov.queue(
        &f.trading.address,
        &Symbol::new(&f.env, "set_config"),
        &args,
    );

    // Verify it's queued
    let queued = f.gov.get_queued(&nonce);
    assert_eq!(queued.target, f.trading.address, "queued target should be trading");

    // Jump past the delay
    jump(&f.env, DELAY + 1);

    // Execute the queued config change (permissionless)
    f.gov.execute(&nonce);

    // Verify the trading contract now has the new config
    let live_config = f.trading.get_config();
    assert_eq!(
        live_config.r_base, new_config.r_base,
        "trading config r_base should be updated via timelock"
    );
}

/// Queue a set_market via the generic timelock, wait past delay, execute.
/// Verify market config changed.
#[test]
fn test_timelock_queue_and_execute_set_market() {
    let f = setup();

    // Prepare a modified market config with changed max_util
    let mut new_market = default_market(&f.env);
    new_market.max_util = 8 * SCALAR_7; // 8x (up from 5x default)

    // Queue the market change via generic queue
    let feed_id: u32 = FEED_BTC;
    let args: Vec<Val> = Vec::from_array(
        &f.env,
        [feed_id.into_val(&f.env), new_market.clone().into_val(&f.env)],
    );
    let nonce = f.gov.queue(
        &f.trading.address,
        &Symbol::new(&f.env, "set_market"),
        &args,
    );

    // Verify it's queued
    let queued = f.gov.get_queued(&nonce);
    assert_eq!(queued.fn_name, Symbol::new(&f.env, "set_market"));

    // Jump past the delay
    jump(&f.env, DELAY + 1);

    // Execute
    f.gov.execute(&nonce);

    // Verify
    let live_market = f.trading.get_market_config(&(FEED_BTC));
    assert_eq!(
        live_market.max_util, new_market.max_util,
        "market max_util should be updated via timelock"
    );
}

/// Execute BEFORE the delay passes should revert with NotUnlocked.
#[test]
fn test_timelock_execute_before_delay_reverts() {
    let f = setup();

    let mut new_config = default_config();
    new_config.r_base = 20_000_000_000_000;

    let args: Vec<Val> = Vec::from_array(&f.env, [new_config.clone().into_val(&f.env)]);
    let nonce = f.gov.queue(
        &f.trading.address,
        &Symbol::new(&f.env, "set_config"),
        &args,
    );

    // Jump only half the delay
    jump(&f.env, DELAY / 2);

    // Try to execute too early -- should fail with NotUnlocked
    let result = f.gov.try_execute(&nonce);
    assert!(result.is_err(), "execute should revert before delay passes");
}

/// Queue, cancel, then try execute after delay. Should revert with NotQueued.
#[test]
fn test_timelock_cancel_prevents_execution() {
    let f = setup();

    let mut new_config = default_config();
    new_config.r_base = 20_000_000_000_000;

    let args: Vec<Val> = Vec::from_array(&f.env, [new_config.clone().into_val(&f.env)]);
    let nonce = f.gov.queue(
        &f.trading.address,
        &Symbol::new(&f.env, "set_config"),
        &args,
    );

    // Cancel the queued update
    f.gov.cancel(&nonce);

    // Jump past delay
    jump(&f.env, DELAY + 1);

    // Try to execute -- should fail because it was cancelled (NotQueued)
    let result = f.gov.try_execute(&nonce);
    assert!(result.is_err(), "execute should revert after cancellation");
}

/// set_status bypasses the delay -- immediate execution.
#[test]
fn test_timelock_set_status_immediate() {
    let f = setup();

    // Verify trading is currently Active (0)
    let status_before = f.trading.get_status();
    assert_eq!(status_before, 0, "trading should start Active");

    // Set status to AdminOnIce (2) immediately through governance
    // This does NOT queue -- it executes right away
    f.gov.set_status(&f.trading.address, &2u32);

    // Verify status changed immediately (no delay)
    let status_after = f.trading.get_status();
    assert_eq!(
        status_after, 2,
        "set_status should bypass delay and change status immediately to AdminOnIce"
    );
}

/// Queue a set_market, cancel it, jump past delay, try execute.
/// Specifically addresses T-ELEV-12 governance front-run concern.
#[test]
fn test_timelock_execute_after_cancel_reverts() {
    let f = setup();

    let mut new_market = default_market(&f.env);
    new_market.max_util = 8 * SCALAR_7;

    let feed_id: u32 = FEED_BTC;
    let args: Vec<Val> = Vec::from_array(
        &f.env,
        [feed_id.into_val(&f.env), new_market.clone().into_val(&f.env)],
    );

    // Queue
    let nonce = f.gov.queue(
        &f.trading.address,
        &Symbol::new(&f.env, "set_market"),
        &args,
    );

    // Cancel
    f.gov.cancel(&nonce);

    // Jump well past delay
    jump(&f.env, DELAY * 2);

    // Execute should fail
    let result = f.gov.try_execute(&nonce);
    assert!(result.is_err(), "execute should revert after cancellation");

    // Verify the original market config is unchanged
    let live_market = f.trading.get_market_config(&(FEED_BTC));
    let original = default_market(&f.env);
    assert_eq!(
        live_market.max_util, original.max_util,
        "market config should be unchanged after cancel+execute"
    );
}

/// Test that cancelling one queued nonce does not affect other queued updates.
#[test]
fn test_timelock_cancel_one_does_not_affect_other() {
    let f = setup();

    let mut market_a = default_market(&f.env);
    market_a.max_util = 8 * SCALAR_7;

    let mut market_b = default_market(&f.env);
    market_b.margin = 0_0200000; // 2% margin

    let feed_id: u32 = FEED_BTC;

    // Queue two market updates
    let args_a: Vec<Val> = Vec::from_array(
        &f.env,
        [feed_id.into_val(&f.env), market_a.clone().into_val(&f.env)],
    );
    let nonce_a = f.gov.queue(
        &f.trading.address,
        &Symbol::new(&f.env, "set_market"),
        &args_a,
    );

    let args_b: Vec<Val> = Vec::from_array(
        &f.env,
        [feed_id.into_val(&f.env), market_b.clone().into_val(&f.env)],
    );
    let nonce_b = f.gov.queue(
        &f.trading.address,
        &Symbol::new(&f.env, "set_market"),
        &args_b,
    );

    // Cancel only nonce_a
    f.gov.cancel(&nonce_a);

    // Jump past delay
    jump(&f.env, DELAY + 1);

    // nonce_a should fail
    let result = f.gov.try_execute(&nonce_a);
    assert!(result.is_err(), "cancelled nonce should fail");

    // nonce_b should succeed
    f.gov.execute(&nonce_b);
    let live = f.trading.get_market_config(&(FEED_BTC));
    assert_eq!(live.margin, market_b.margin, "nonce_b should execute successfully");
}
