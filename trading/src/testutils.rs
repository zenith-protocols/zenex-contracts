#![cfg(any(test, feature = "testutils"))]

use crate::constants::{SCALAR_7, SCALAR_18};
use crate::contract::TradingContract;
use crate::storage;
use crate::types::{ContractStatus, MarketConfig, MarketData, TradingConfig};
use sep_40_oracle::testutils::{MockPriceOracleClient, MockPriceOracleWASM};
use sep_40_oracle::Asset as StellarAsset;
use sep_41_token::testutils::{MockTokenClient, MockTokenWASM};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address, Env, IntoVal, String, Symbol};

//************************************************
//           Mock Vault Contract
//************************************************

#[contract]
pub struct MockVault;

#[contractimpl]
impl MockVault {
    pub fn __constructor(e: Env, token: Address, total_assets: i128) {
        e.storage().instance().set(&Symbol::new(&e, "token"), &token);
        e.storage().instance().set(&Symbol::new(&e, "assets"), &total_assets);
    }

    pub fn query_asset(e: Env) -> Address {
        e.storage().instance().get(&Symbol::new(&e, "token")).unwrap()
    }

    pub fn total_assets(e: Env) -> i128 {
        e.storage().instance().get(&Symbol::new(&e, "assets")).unwrap()
    }

    pub fn strategy_withdraw(e: Env, _strategy: Address, amount: i128) {
        let current: i128 = e.storage().instance().get(&Symbol::new(&e, "assets")).unwrap();
        e.storage().instance().set(&Symbol::new(&e, "assets"), &(current - amount));
    }

    // Test helper to set total_assets
    pub fn set_total_assets(e: Env, amount: i128) {
        e.storage().instance().set(&Symbol::new(&e, "assets"), &amount);
    }
}

//************************************************
//           Test Constants
//************************************************

/// Default BTC price in SCALAR_7
pub const BTC_PRICE: i128 = 100_000_0000000; // 100,000 USD

/// Default ETH price in SCALAR_7
pub const ETH_PRICE: i128 = 2_000_0000000; // 2,000 USD

/// Seconds in an hour
pub const SECONDS_IN_HOUR: u64 = 3600;

/// Seconds in a day
pub const SECONDS_IN_DAY: u64 = 86400;

/// Seconds in a week
pub const SECONDS_IN_WEEK: u64 = 604800;

//************************************************
//           Contract Setup Helpers
//************************************************

/// Create a trading contract for unit testing.
/// Returns (contract_address, owner_address)
pub fn create_trading(e: &Env) -> (Address, Address) {
    let owner = Address::generate(e);
    let address = e.register(TradingContract {}, (owner.clone(),));
    (address, owner)
}

/// Create a mock oracle with default setup and return (address, client)
pub fn create_oracle<'a>(e: &Env) -> (Address, MockPriceOracleClient<'a>) {
    use sep_40_oracle::testutils::Asset;
    use soroban_sdk::vec as svec;

    let address = e.register(MockPriceOracleWASM, ());
    let client = MockPriceOracleClient::new(e, &address);
    let admin = Address::generate(e);

    // Set up oracle with BTC asset, 7 decimals, 300s resolution
    client.set_data(
        &admin,
        &Asset::Other(Symbol::new(e, "USD")),
        &svec![e, Asset::Other(Symbol::new(e, "BTC"))],
        &7,
        &300,
    );
    client.set_price_stable(&svec![e, BTC_PRICE]);

    (address, client)
}

/// Create a mock token and return (address, client)
pub fn create_token<'a>(e: &Env, admin: &Address) -> (Address, MockTokenClient<'a>) {
    let address = Address::generate(e);
    e.register_at(&address, MockTokenWASM, ());
    let client = MockTokenClient::new(e, &address);
    client.initialize(admin, &7, &"Test".into_val(e), &"TST".into_val(e));
    (address, client)
}

/// Create a mock vault and return (address, client)
pub fn create_vault<'a>(e: &Env, token: &Address, total_assets: i128) -> (Address, MockVaultClient<'a>) {
    let address = e.register(MockVault, (token.clone(), total_assets));
    let client = MockVaultClient::new(e, &address);
    (address, client)
}

/// Set up basic contract state for unit testing.
/// Call this inside `e.as_contract(&address, || { ... })`
pub fn setup_test_state(e: &Env, oracle: &Address) {
    storage::set_name(e, &String::from_str(e, "Test"));
    storage::set_status(e, ContractStatus::Active as u32);
    storage::set_config(e, &default_config(oracle));
}

//************************************************
//           Default Configs
//************************************************

/// Create a default trading config for testing
pub fn default_config(oracle: &Address) -> TradingConfig {
    TradingConfig {
        oracle: oracle.clone(),
        caller_take_rate: 0_1000000, // 10%
        max_positions: 10,
        max_utilization: 50 * SCALAR_7, // 50x
        max_price_age: 3600,            // 1 hour
    }
}

/// Create a default market config for testing
pub fn default_market(e: &Env) -> MarketConfig {
    MarketConfig {
        asset: StellarAsset::Other(Symbol::new(e, "BTC")),
        enabled: true,
        max_payout: 10 * SCALAR_7,            // 1000% max payout
        min_collateral: SCALAR_7,             // 1 token minimum
        max_collateral: 1_000_000 * SCALAR_7, // 1M tokens maximum

        init_margin: 0_0100000,        // 1%
        maintenance_margin: 0_0050000, // 0.5%

        base_fee: 0_0005000, // 0.05%
        price_impact_scalar: 8_000_000_000 * SCALAR_7,
        base_hourly_rate: 10_000_000_000_000, // 0.001% per hour in SCALAR_18
        ratio_cap: 5 * SCALAR_18,             // 5x cap
    }
}

/// Create default market data (empty market)
pub fn default_market_data() -> MarketData {
    MarketData {
        long_notional_size: 0,
        short_notional_size: 0,
        long_interest_index: SCALAR_18,
        short_interest_index: SCALAR_18,
        last_update: 0,
    }
}

//************************************************
//           Assertion Helpers
//************************************************

/// Assert that two i128 values are approximately equal within tolerance
pub fn assert_approx_eq(actual: i128, expected: i128, tolerance: i128, message: &str) {
    let diff = (actual - expected).abs();
    assert!(
        diff <= tolerance,
        "{}: expected {} (± {}), got {} (diff: {})",
        message,
        expected,
        tolerance,
        actual,
        diff
    );
}

/// Assert that a value is within a percentage of expected
pub fn assert_within_percent(actual: i128, expected: i128, percent: i128, message: &str) {
    let tolerance = expected.abs() * percent / 100;
    assert_approx_eq(actual, expected, tolerance, message);
}
