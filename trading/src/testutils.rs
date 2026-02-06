#![cfg(any(test, feature = "testutils"))]

use crate::constants::{SCALAR_7, SCALAR_18};
use crate::contract::TradingContract;
use crate::storage;
use crate::types::{ContractStatus, MarketConfig, MarketData, TradingConfig};
use sep_40_oracle::testutils::{MockPriceOracleClient, MockPriceOracleWASM};
use sep_40_oracle::Asset as StellarAsset;
use sep_41_token::testutils::{MockTokenClient, MockTokenWASM};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{contract, contractimpl, Address, Env, IntoVal, String, Symbol};

//************************************************
//           Test Constants
//************************************************

/// Default BTC price in SCALAR_7
pub const BTC_PRICE: i128 = 100_000_0000000; // 100,000 USD

//************************************************
//           Mock Vault
//************************************************

#[contract]
pub struct MockVault;

#[contractimpl]
impl MockVault {
    pub fn __constructor(e: Env, token: Address) {
        e.storage().instance().set(&Symbol::new(&e, "token"), &token);
    }

    pub fn query_asset(e: Env) -> Address {
        e.storage().instance().get(&Symbol::new(&e, "token")).unwrap()
    }

    pub fn total_assets(e: Env) -> i128 {
        let token: Address = e.storage().instance().get(&Symbol::new(&e, "token")).unwrap();
        soroban_sdk::token::TokenClient::new(&e, &token).balance(&e.current_contract_address())
    }

    pub fn strategy_withdraw(e: Env, strategy: Address, amount: i128) {
        let token: Address = e.storage().instance().get(&Symbol::new(&e, "token")).unwrap();
        soroban_sdk::token::TokenClient::new(&e, &token)
            .transfer(&e.current_contract_address(), &strategy, &amount);
    }
}

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

/// Create a mock vault and fund it with tokens.
/// Returns the vault address.
pub fn create_vault(e: &Env, token: &Address, initial_assets: i128) -> Address {
    let address = e.register(MockVault, (token.clone(),));
    if initial_assets > 0 {
        MockTokenClient::new(e, token).mint(&address, &initial_assets);
    }
    address
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
        min_open_time: 0,               // disabled
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
        long_interest_index: 0,
        short_interest_index: 0,
        last_update: 0,
    }
}

//************************************************
//           Environment Setup
//************************************************

/// Create a default test environment with mock auth and ledger info.
pub fn setup_env() -> Env {
    let e = Env::default();
    e.mock_all_auths();
    jump(&e, 1000);
    e
}

/// Advance the ledger to the given timestamp (sequence_number derived as timestamp / 10).
pub fn jump(e: &Env, timestamp: u64) {
    e.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp,
        protocol_version: 25,
        sequence_number: (timestamp / 10) as u32,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 3110400,
    });
}

/// Fully initialize a trading contract with oracle, vault, token, and BTC market.
///
/// Returns (contract_address, token_client).
pub fn setup_contract(e: &Env) -> (Address, MockTokenClient<'_>) {
    let (contract, owner) = create_trading(e);
    let (oracle, _) = create_oracle(e);
    let (token, token_client) = create_token(e, &owner);
    let vault = create_vault(e, &token, 100_000_000 * SCALAR_7);

    e.as_contract(&contract, || {
        crate::trading::execute_initialize(
            e,
            &String::from_str(e, "Test"),
            &vault,
            &default_config(&oracle),
        );
        storage::set_status(e, ContractStatus::Active as u32);

        storage::set_market_config(e, 0, &default_market(e));
        let mut market_data = default_market_data();
        market_data.last_update = e.ledger().timestamp();
        market_data.long_interest_index = SCALAR_18;
        market_data.short_interest_index = SCALAR_18;
        storage::set_market_data(e, 0, &market_data);
        storage::next_market_index(e);
    });

    // Fund contract for token transfers
    token_client.mint(&contract, &(10_000_000 * SCALAR_7));

    (contract, token_client)
}
