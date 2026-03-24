#![cfg(any(test, feature = "testutils"))]

use crate::constants::{SCALAR_7, SCALAR_18};
use crate::contract::TradingContract;
use crate::storage;
use crate::types::{MarketConfig, MarketData, TradingConfig};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, Env, Map, Vec};

//************************************************
//           Test Constants
//************************************************

/// Default BTC price raw (exponent -8), used in mock price verifier
pub const BTC_PRICE_RAW: i128 = 10_000_000_000_000; // $100,000 with exponent -8

/// Default BTC price (raw Pyth value with exponent -8)
pub const BTC_PRICE: i128 = BTC_PRICE_RAW; // $100,000 = 10_000_000_000_000

/// BTC feed ID for Pyth Lazer
pub const BTC_FEED_ID: u32 = 1;

/// Price scalar matching mock exponent -8 (10^8)
pub const PRICE_SCALAR: i128 = 100_000_000;

//************************************************
//           Mock Price Verifier
//************************************************

/// Mock price-verifier that simulates verify functions.
/// Stores a map of feed_id → normalized price (i128) in instance storage.
#[contract]
pub struct MockPriceVerifier;

/// PriceData type matching price-verifier's return type.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockPriceData {
    pub feed_id: u32,
    pub price: i128,
    pub exponent: i32,
    pub publish_time: u64,
}

/// Storage key for mock price-verifier prices.
#[contracttype]
#[derive(Clone)]
pub enum MockPVKey {
    Prices,
}

#[contractimpl]
impl MockPriceVerifier {
    /// Set the normalized price for a feed_id.
    pub fn set_price(e: Env, feed_id: u32, price: i128) {
        let mut prices: Map<u32, i128> = e
            .storage()
            .instance()
            .get(&MockPVKey::Prices)
            .unwrap_or(Map::new(&e));
        prices.set(feed_id, price);
        e.storage()
            .instance()
            .set(&MockPVKey::Prices, &prices);
    }

    /// Verify price feeds (mock: ignores price bytes, returns all stored prices).
    pub fn verify_prices(e: Env, _price: Bytes) -> Vec<MockPriceData> {
        let prices: Map<u32, i128> = e
            .storage()
            .instance()
            .get(&MockPVKey::Prices)
            .expect("no prices configured");
        let mut results: Vec<MockPriceData> = Vec::new(&e);
        let keys = prices.keys();
        for i in 0..keys.len() {
            let feed_id = keys.get(i).unwrap();
            let price = prices.get(feed_id).unwrap();
            results.push_back(MockPriceData {
                feed_id,
                price,
                exponent: -8,
                publish_time: e.ledger().timestamp(),
            });
        }
        results
    }
}

//************************************************
//           Mock Vault
//************************************************

#[contract]
pub struct MockVault;

#[contractimpl]
impl MockVault {
    pub fn __constructor(e: Env, token: Address) {
        e.storage().instance().set(&soroban_sdk::Symbol::new(&e, "token"), &token);
    }

    pub fn query_asset(e: Env) -> Address {
        e.storage().instance().get(&soroban_sdk::Symbol::new(&e, "token")).unwrap()
    }

    pub fn total_assets(e: Env) -> i128 {
        let token: Address = e.storage().instance().get(&soroban_sdk::Symbol::new(&e, "token")).unwrap();
        soroban_sdk::token::TokenClient::new(&e, &token).balance(&e.current_contract_address())
    }

    pub fn strategy_withdraw(e: Env, strategy: Address, amount: i128) {
        let token: Address = e.storage().instance().get(&soroban_sdk::Symbol::new(&e, "token")).unwrap();
        soroban_sdk::token::TokenClient::new(&e, &token)
            .transfer(&e.current_contract_address(), &strategy, &amount);
    }
}

//************************************************
//           Mock Treasury
//************************************************

#[contract]
pub struct MockTreasury;

#[contractimpl]
impl MockTreasury {
    pub fn get_rate(_e: Env) -> i128 {
        500_000 // 5% protocol fee
    }

    pub fn get_fee(e: Env, total_fee: i128) -> i128 {
        use soroban_fixed_point_math::SorobanFixedPoint;
        let rate = 500_000_i128;
        if total_fee > 0 {
            total_fee.fixed_mul_floor(&e, &rate, &SCALAR_7)
        } else {
            0
        }
    }
}

//************************************************
//           Contract Setup Helpers
//************************************************

pub fn create_treasury(e: &Env) -> Address {
    e.register(MockTreasury, ())
}

pub fn create_trading(e: &Env) -> (Address, Address) {
    create_trading_with_vault(e, 100_000 * SCALAR_7)
}

pub fn create_trading_with_vault(e: &Env, vault_amount: i128) -> (Address, Address) {
    e.mock_all_auths();
    let owner = Address::generate(e);
    let (price_verifier, _) = create_price_verifier(e);
    let (token, _) = create_token(e, &owner);
    let vault = create_vault(e, &token, vault_amount);
    let treasury = create_treasury(e);
    let address = e.register(TradingContract {}, (
        owner.clone(),
        token,
        vault,
        price_verifier,
        treasury,
        default_config(),
    ));
    (address, owner)
}

/// Create a mock price-verifier with BTC price set.
/// Returns (price_verifier_address, MockPriceVerifierClient)
pub fn create_price_verifier(e: &Env) -> (Address, MockPriceVerifierClient<'_>) {
    let address = e.register(MockPriceVerifier, ());
    let client = MockPriceVerifierClient::new(e, &address);

    // BTC at $100,000 normalized to 10 decimals
    client.set_price(&BTC_FEED_ID, &BTC_PRICE_RAW);

    (address, client)
}

pub fn create_token<'a>(e: &Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
    let address = e.register_stellar_asset_contract_v2(admin.clone()).address();
    let client = StellarAssetClient::new(e, &address);
    (address, client)
}

pub fn create_vault(e: &Env, token: &Address, initial_assets: i128) -> Address {
    let address = e.register(MockVault, (token.clone(),));
    if initial_assets > 0 {
        StellarAssetClient::new(e, token).mint(&address, &initial_assets);
    }
    address
}

//************************************************
//           Default Configs
//************************************************

pub fn default_config() -> TradingConfig {
    TradingConfig {
        caller_rate: 1_000_000,                    // 10%
        min_notional: 10 * SCALAR_7,              // 10 tokens minimum notional
        max_notional: 10_000_000 * SCALAR_7,      // 10M tokens maximum notional
        fee_dom: 5_000,                            // 0.05%
        fee_non_dom: 1_000,                        // 0.01%
        max_util: 10 * SCALAR_7,                          // 10x vault
        r_funding: 10_000_000_000_000,             // 0.001% per hour in SCALAR_18
        r_base: 10_000_000_000_000,                // 0.001% per hour in SCALAR_18
        r_var: SCALAR_7,                           // 1× multiplier: at full util, rate doubles
    }
}

pub fn default_market(_e: &Env) -> MarketConfig {
    MarketConfig {
        enabled: true,
        max_util: 5 * SCALAR_7,                           // 5x vault per market
        r_borrow: SCALAR_7,                        // 1× (no adjustment)
        margin: 100_000,                           // 1%
        liq_fee: 50_000,                           // 0.5%
        impact: 8_000_000_000 * SCALAR_7,
    }
}

pub fn default_market_data() -> MarketData {
    MarketData::default()
}

//************************************************
//           Environment Setup
//************************************************

pub fn setup_env() -> Env {
    let e = Env::default();
    e.mock_all_auths();
    jump(&e, 1000);
    e
}

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

/// Dummy price bytes for tests (mock price-verifier ignores contents).
pub fn dummy_price(e: &Env) -> Bytes {
    Bytes::from_array(e, &[0u8; 1])
}

/// Fully initialize a trading contract with price-verifier, vault, token, and BTC market.
pub fn setup_contract(e: &Env) -> (Address, StellarAssetClient<'_>) {
    let owner = Address::generate(e);
    let (price_verifier, _) = create_price_verifier(e);
    let (token, token_client) = create_token(e, &owner);
    let vault = create_vault(e, &token, 100_000_000 * SCALAR_7);
    let treasury = create_treasury(e);

    let contract = e.register(TradingContract {}, (
        owner.clone(),
        token.clone(),
        vault,
        price_verifier,
        treasury,
        default_config(),
    ));

    let config = default_config();

    e.as_contract(&contract, || {
        storage::set_market_config(e, BTC_FEED_ID, &default_market(e));
        let mut market_data = default_market_data();
        market_data.last_update = e.ledger().timestamp();
        market_data.l_fund_idx = SCALAR_18;
        market_data.s_fund_idx = SCALAR_18;
        storage::set_market_data(e, BTC_FEED_ID, &market_data);
        let mut markets = storage::get_markets(e);
        markets.push_back(BTC_FEED_ID);
        storage::set_markets(e, &markets);

        storage::set_last_funding_update(e, e.ledger().timestamp());
        let mut data = storage::get_market_data(e, BTC_FEED_ID);
        data.update_funding_rate(e, config.r_funding);
        storage::set_market_data(e, BTC_FEED_ID, &data);
    });

    token_client.mint(&contract, &(10_000_000 * SCALAR_7));

    (contract, token_client)
}

//************************************************
//           Fuzz / Property Test Wrappers
//************************************************

pub fn calc_funding_rate_for_test(
    e: &Env,
    long_notional: i128,
    short_notional: i128,
    base_rate: i128,
) -> i128 {
    crate::trading::rates::calc_funding_rate(e, long_notional, short_notional, base_rate)
}
