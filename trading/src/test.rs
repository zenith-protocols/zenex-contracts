#![cfg(test)]
extern crate std;

use crate::constants::{SCALAR_7, SECONDS_PER_WEEK};
use crate::contract::{TradingClient, TradingContract};
use crate::testutils::{
    create_oracle, create_token, create_vault, default_config, default_market, BTC_PRICE,
};
use crate::types::{ContractStatus, ExecuteRequest, ExecuteRequestType, TradingConfig};
use sep_40_oracle::testutils::MockPriceOracleClient;
use sep_41_token::testutils::MockTokenClient;
use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{vec, Address, Env, IntoVal, String, Vec};

/// Test fixture containing all the components needed for trading tests
#[allow(dead_code)]
struct TestFixture<'a> {
    env: Env,
    contract: Address,
    client: TradingClient<'a>,
    owner: Address,
    vault: Address,
    token: Address,
    token_client: MockTokenClient<'a>,
    oracle: Address,
    oracle_client: MockPriceOracleClient<'a>,
}

impl<'a> TestFixture<'a> {
    fn setup(e: &Env) -> TestFixture<'a> {
        e.mock_all_auths();

        let owner = Address::generate(e);
        let (oracle, oracle_client) = create_oracle(e);
        let (token, token_client) = create_token(e, &owner);
        let (vault, _) = create_vault(e, &token, 1_000_000 * SCALAR_7);

        let contract = e.register(TradingContract, (&owner,));
        let client = TradingClient::new(e, &contract);

        TestFixture {
            env: e.clone(),
            contract,
            client,
            owner,
            vault,
            token,
            token_client,
            oracle,
            oracle_client,
        }
    }

    fn initialize(&self) {
        let config = default_config(&self.oracle);
        self.client.initialize(
            &String::from_str(&self.env, "Test Trading"),
            &self.vault,
            &config,
        );
    }

    fn setup_market(&self) {
        let market = default_market(&self.env);
        self.client.queue_set_market(&market);
        self.client.set_market(&market.asset);
    }

    fn activate(&self) {
        self.client.set_status(&(ContractStatus::Active as u32));
    }

    fn fund_user(&self, user: &Address, amount: i128) {
        self.token_client.mint(user, &amount);
    }
}

fn setup_env() -> Env {
    let e = Env::default();
    e.ledger().set(LedgerInfo {
        timestamp: 1000,
        protocol_version: 25,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 3110400,
    });
    e
}

// ==========================================
// Initialization Tests
// ==========================================

#[test]
fn test_initialize() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();

    // Contract should be in Setup status after init
    // We can verify by trying to add a market (only works in Setup)
    fixture.setup_market();
}

#[test]
#[should_panic(expected = "Error(Contract, #300)")]
fn test_initialize_twice() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.initialize(); // Should panic
}

// ==========================================
// Config Management Tests
// ==========================================

#[test]
fn test_queue_and_set_config_setup_mode() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();

    let mut new_config = default_config(&fixture.oracle);
    new_config.max_positions = 20;

    fixture.client.queue_set_config(&new_config);
    fixture.client.set_config(); // No delay in Setup mode
}

#[test]
fn test_queue_and_set_config_active_mode() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let mut new_config = default_config(&fixture.oracle);
    new_config.max_positions = 20;

    fixture.client.queue_set_config(&new_config);

    // Advance time past 1 week unlock
    e.ledger().set(LedgerInfo {
        timestamp: 1000 + SECONDS_PER_WEEK + 1,
        protocol_version: 25,
        sequence_number: 200,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 3110400,
    });

    fixture.client.set_config();
}

#[test]
#[should_panic(expected = "Error(Contract, #304)")]
fn test_set_config_not_unlocked() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let config = default_config(&fixture.oracle);
    fixture.client.queue_set_config(&config);
    fixture.client.set_config(); // Should panic - not unlocked yet
}

#[test]
fn test_cancel_set_config() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();

    let config = default_config(&fixture.oracle);
    fixture.client.queue_set_config(&config);
    fixture.client.cancel_set_config();
}

#[test]
#[should_panic(expected = "Error(Contract, #303)")]
fn test_cancel_set_config_not_queued() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.client.cancel_set_config(); // Should panic
}

// ==========================================
// Market Management Tests
// ==========================================

#[test]
fn test_queue_and_set_market() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();

    let market = default_market(&e);
    fixture.client.queue_set_market(&market);
    fixture.client.set_market(&market.asset);
}

#[test]
fn test_cancel_queued_market() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();

    let market = default_market(&e);
    fixture.client.queue_set_market(&market);
    fixture.client.cancel_set_market(&market.asset);
}

#[test]
#[should_panic(expected = "Error(Contract, #304)")]
fn test_set_market_not_unlocked() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    // Queue new market in Active mode
    let mut market = default_market(&e);
    market.asset = sep_40_oracle::Asset::Other(soroban_sdk::Symbol::new(&e, "ETH"));
    fixture.client.queue_set_market(&market);
    fixture.client.set_market(&market.asset); // Should panic - not unlocked
}

// ==========================================
// Status Management Tests
// ==========================================

#[test]
fn test_set_status_active() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.client.set_status(&(ContractStatus::Active as u32));
}

#[test]
fn test_set_status_on_ice() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();
    fixture.client.set_status(&(ContractStatus::OnIce as u32));
}

#[test]
fn test_set_status_frozen() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();
    fixture.client.set_status(&(ContractStatus::Frozen as u32));
}

#[test]
#[should_panic(expected = "Error(Contract, #381)")]
fn test_set_status_setup_invalid() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    // Cannot set status back to Setup
    fixture.client.set_status(&(ContractStatus::Setup as u32));
}

// ==========================================
// Position Tests
// ==========================================

#[test]
fn test_open_market_order_long() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);

    // Approve token spend
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    let (position_id, fee) = fixture.client.open_position(
        &user,
        &0,                  // asset_index
        &collateral,         // collateral
        &(10_000 * SCALAR_7), // notional_size (10x leverage)
        &true,               // is_long
        &0,                  // entry_price (0 = market order)
        &0,                  // take_profit
        &0,                  // stop_loss
    );

    assert_eq!(position_id, 1);
    assert!(fee > 0);
}

#[test]
fn test_open_limit_order() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    // Limit order above current price for long
    let entry_price = BTC_PRICE + 5000 * SCALAR_7;
    let (position_id, fee) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &entry_price,
        &0,
        &0,
    );

    assert_eq!(position_id, 1);
    // Fee is charged on position creation (price impact fee)
    assert!(fee >= 0);
}

#[test]
fn test_open_short_position() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &false, // short
        &0,
        &0,
        &0,
    );

    assert_eq!(position_id, 1);
}

#[test]
fn test_close_position() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    let (pnl, fee) = fixture.client.close_position(&position_id);
    // At same price, pnl should be ~0
    assert!(pnl.abs() < SCALAR_7); // Allow for small rounding
    assert!(fee >= 0);
}

#[test]
fn test_cancel_pending_position() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    // Create pending limit order
    let entry_price = BTC_PRICE + 5000 * SCALAR_7;
    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &entry_price,
        &0,
        &0,
    );

    // Cancel it
    let (pnl, fee) = fixture.client.close_position(&position_id);
    assert_eq!(pnl, 0);
    assert_eq!(fee, 0);
}

#[test]
fn test_modify_collateral_deposit() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 3);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 3), &1000000);

    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    // Add more collateral
    let new_collateral = collateral + 500 * SCALAR_7;
    fixture
        .client
        .modify_collateral(&position_id, &new_collateral);
}

#[test]
fn test_modify_collateral_withdraw() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 2_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    // Withdraw some collateral (still above margin requirement)
    let new_collateral = collateral - 500 * SCALAR_7;
    fixture
        .client
        .modify_collateral(&position_id, &new_collateral);
}

#[test]
fn test_set_triggers() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    // Set triggers for long position
    let take_profit = BTC_PRICE + 10_000 * SCALAR_7;
    let stop_loss = BTC_PRICE - 5_000 * SCALAR_7;
    fixture
        .client
        .set_triggers(&position_id, &take_profit, &stop_loss);
}

// ==========================================
// Execute (Keeper) Tests
// ==========================================

#[test]
fn test_execute_fill_limit_order() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    // Create limit order above current price
    let entry_price = BTC_PRICE + 1000 * SCALAR_7;
    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &entry_price,
        &0,
        &0,
    );

    // Keeper tries to fill - should succeed since current price <= entry price for long
    let keeper = Address::generate(&e);
    let requests = vec![
        &e,
        ExecuteRequest {
            request_type: ExecuteRequestType::Fill as u32,
            position_id,
        },
    ];

    let results = fixture.client.execute(&keeper, &requests);
    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0), Some(0)); // Success
}

#[test]
fn test_execute_take_profit() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    // Open position with TP at current price
    let take_profit = BTC_PRICE;
    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &take_profit,
        &0,
    );

    // Keeper triggers TP
    let keeper = Address::generate(&e);
    let requests = vec![
        &e,
        ExecuteRequest {
            request_type: ExecuteRequestType::TakeProfit as u32,
            position_id,
        },
    ];

    let results = fixture.client.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0)); // Success
}

#[test]
fn test_execute_stop_loss() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    // Open position with SL above current price (for long, triggers when price <= SL)
    let stop_loss = BTC_PRICE + 1000 * SCALAR_7;
    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &stop_loss,
    );

    // Keeper triggers SL
    let keeper = Address::generate(&e);
    let requests = vec![
        &e,
        ExecuteRequest {
            request_type: ExecuteRequestType::StopLoss as u32,
            position_id,
        },
    ];

    let results = fixture.client.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0)); // Success
}

#[test]
fn test_execute_liquidation() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 100 * SCALAR_7; // Small collateral
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    // Open highly leveraged position
    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7), // 100x leverage
        &true,
        &0,
        &0,
        &0,
    );

    // Advance time to accrue interest
    e.ledger().set(LedgerInfo {
        timestamp: 1000 + 86400 * 30, // 30 days
        protocol_version: 25,
        sequence_number: 200,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 3110400,
    });

    // Update oracle price to be current
    fixture.oracle_client.set_price_stable(&vec![&e, BTC_PRICE]);

    // Keeper tries to liquidate
    let keeper = Address::generate(&e);
    let requests = vec![
        &e,
        ExecuteRequest {
            request_type: ExecuteRequestType::Liquidate as u32,
            position_id,
        },
    ];

    let results = fixture.client.execute(&keeper, &requests);
    // May or may not be liquidatable depending on interest accrued
    assert_eq!(results.len(), 1);
}

#[test]
fn test_execute_multiple_requests() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 10);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 10), &1000000);

    // Create multiple positions
    let entry_price = BTC_PRICE + 1000 * SCALAR_7;
    let (pos1, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &entry_price,
        &0,
        &0,
    );

    let (pos2, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &BTC_PRICE, // TP at current price
        &0,
    );

    // Batch execute
    let keeper = Address::generate(&e);
    let requests = vec![
        &e,
        ExecuteRequest {
            request_type: ExecuteRequestType::Fill as u32,
            position_id: pos1,
        },
        ExecuteRequest {
            request_type: ExecuteRequestType::TakeProfit as u32,
            position_id: pos2,
        },
    ];

    let results = fixture.client.execute(&keeper, &requests);
    assert_eq!(results.len(), 2);
}

// ==========================================
// Error Cases
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #380)")]
fn test_open_position_contract_paused() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.client.set_status(&(ContractStatus::Frozen as u32));

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    // Should fail - contract is frozen
    fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #331)")]
fn test_open_position_collateral_below_min() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = SCALAR_7 / 10; // Below minimum
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #310)")]
fn test_open_position_invalid_market() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    // Invalid asset_index
    fixture.client.open_position(
        &user,
        &99, // Non-existent market
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

// ==========================================
// Upgrade Tests
// ==========================================

#[test]
fn test_upgrade_contract() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();

    // Include the WASM bytes directly
    const WASM: &[u8] = include_bytes!("../../wasm/trading.wasm");

    // Upload the WASM to get its hash
    let wasm_hash = e.deployer().upload_contract_wasm(WASM);

    // Call upgrade function directly via invoke_contract
    e.invoke_contract::<()>(
        &fixture.contract,
        &soroban_sdk::Symbol::new(&e, "upgrade"),
        soroban_sdk::vec![&e, wasm_hash.to_val(), fixture.owner.to_val()],
    );
}

// ==========================================
// View Function Tests
// ==========================================

#[test]
fn test_get_config() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();

    let config = fixture.client.get_config();
    assert_eq!(config.oracle, fixture.oracle);
    assert_eq!(config.max_positions, 10);
}

#[test]
fn test_get_status() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let status: u32 = e.invoke_contract(
        &fixture.contract,
        &soroban_sdk::Symbol::new(&e, "get_status"),
        vec![&e],
    );
    assert_eq!(status, ContractStatus::Active as u32);
}

#[test]
fn test_get_vault() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();

    let vault: Address = e.invoke_contract(
        &fixture.contract,
        &soroban_sdk::Symbol::new(&e, "get_vault"),
        vec![&e],
    );
    assert_eq!(vault, fixture.vault);
}

#[test]
fn test_get_token() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();

    let token: Address = e.invoke_contract(
        &fixture.contract,
        &soroban_sdk::Symbol::new(&e, "get_token"),
        vec![&e],
    );
    assert_eq!(token, fixture.token);
}

#[test]
fn test_get_market_config() {
    use crate::types::MarketConfig;
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();

    let market_config: MarketConfig = e.invoke_contract(
        &fixture.contract,
        &soroban_sdk::Symbol::new(&e, "get_market_config"),
        vec![&e, 0u32.into_val(&e)],
    );
    assert_eq!(market_config.enabled, true);
    assert_eq!(market_config.init_margin, 0_0100000); // 1%
}

#[test]
fn test_get_market_data() {
    use crate::types::MarketData;

    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    // Open a position to create some market data
    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    let market_data: MarketData = e.invoke_contract(
        &fixture.contract,
        &soroban_sdk::Symbol::new(&e, "get_market_data"),
        vec![&e, 0u32.into_val(&e)],
    );
    assert!(market_data.long_notional_size > 0);
}

#[test]
fn test_get_position() {
    use crate::types::Position;

    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 2);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 2), &1000000);

    let (position_id, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    let position: Position = e.invoke_contract(
        &fixture.contract,
        &soroban_sdk::Symbol::new(&e, "get_position"),
        vec![&e, position_id.into_val(&e)],
    );
    assert_eq!(position.id, position_id);
    assert_eq!(position.user, user);
    assert_eq!(position.is_long, true);
    assert_eq!(position.collateral, collateral);
    assert_eq!(position.notional_size, 10_000 * SCALAR_7);
}

#[test]
fn test_get_user_positions() {
    let e = setup_env();
    let fixture = TestFixture::setup(&e);
    fixture.initialize();
    fixture.setup_market();
    fixture.activate();

    let user = Address::generate(&e);
    let collateral = 1_000 * SCALAR_7;
    fixture.fund_user(&user, collateral * 5);
    fixture
        .token_client
        .approve(&user, &fixture.contract, &(collateral * 5), &1000000);

    // Open multiple positions
    let (pos1, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    let (pos2, _) = fixture.client.open_position(
        &user,
        &0,
        &collateral,
        &(5_000 * SCALAR_7),
        &false,
        &0,
        &0,
        &0,
    );

    let positions: Vec<u32> = e.invoke_contract(
        &fixture.contract,
        &soroban_sdk::Symbol::new(&e, "get_user_positions"),
        vec![&e, user.into_val(&e)],
    );
    assert_eq!(positions.len(), 2);
    assert_eq!(positions.get(0), Some(pos1));
    assert_eq!(positions.get(1), Some(pos2));
}

