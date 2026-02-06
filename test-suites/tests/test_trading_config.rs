use soroban_sdk::Symbol;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::default_market;

const SCALAR_18: i128 = 1_000_000_000_000_000_000;
const SECONDS_PER_WEEK: u64 = 604800;

// ==========================================
// Initialize Tests
// ==========================================

#[test]
fn test_initialize_success() {
    // create_fixture_with_data already initializes the contract
    let fixture = create_fixture_with_data(false);
    let config = fixture.trading.get_config();
    assert_eq!(config.max_positions, 10);
    assert_eq!(config.max_price_age, 900);
}

#[test]
#[should_panic(expected = "Error(Contract, #300)")]
fn test_initialize_already_initialized() {
    let fixture = TestFixture::create(false);

    // Try to initialize again - should panic
    fixture
        .trading
        .initialize(&soroban_sdk::String::from_str(&fixture.env, "Test"), &fixture.vault.address, &fixture.trading.get_config());
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_initialize_invalid_config_caller_take_rate_over_100() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: SCALAR_7 + 1, // Over 100%
        max_positions: 10,
        max_utilization: 10 * SCALAR_7, // 10x
        max_price_age: 900,
        min_open_time: 0,
    };

    // queue_set_config validates the config
    fixture.trading.queue_set_config(&config);
}

#[test]
#[should_panic(expected = "Error(Contract, #330)")]
fn test_initialize_invalid_config_negative_caller_take_rate() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: -1,
        max_positions: 10,
        max_utilization: 10 * SCALAR_7, // 10x
        max_price_age: 900,
        min_open_time: 0,
    };

    fixture.trading.queue_set_config(&config);
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_initialize_invalid_config_max_utilization_below_1x() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 10,
        max_utilization: SCALAR_7 - 1, // Below 1x (but not 0)
        max_price_age: 900,
        min_open_time: 0,
    };

    fixture.trading.queue_set_config(&config);
}

#[test]
#[should_panic(expected = "Error(Contract, #330)")]
fn test_initialize_max_utilization_zero_not_allowed() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 10,
        max_utilization: 0, // Zero not allowed
        max_price_age: 900,
        min_open_time: 0,
    };

    fixture.trading.queue_set_config(&config);
}

#[test]
fn test_initialize_max_utilization_100x() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 10,
        max_utilization: 100 * SCALAR_7, // Max allowed
        max_price_age: 900,
        min_open_time: 0,
    };

    fixture.trading.queue_set_config(&config);
    fixture.trading.set_config();

    let stored_config = fixture.trading.get_config();
    assert_eq!(stored_config.max_utilization, 100 * SCALAR_7);
}

// ==========================================
// Queue/Set Config Tests
// ==========================================

#[test]
fn test_queue_set_config_in_setup_mode() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 20, // Different value
        max_utilization: 10 * SCALAR_7, // 10x
        max_price_age: 900,
        min_open_time: 0,
    };

    // In Setup mode, should be immediately applyable
    fixture.trading.queue_set_config(&config);
    fixture.trading.set_config();

    let stored_config = fixture.trading.get_config();
    assert_eq!(stored_config.max_positions, 20);
}

#[test]
fn test_queue_set_config_with_timelock() {
    let fixture = TestFixture::create(false);
    fixture.trading.set_status(&0u32); // Active

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 20,
        max_utilization: 10 * SCALAR_7, // 10x
        max_price_age: 900,
        min_open_time: 0,
    };

    fixture.trading.queue_set_config(&config);

    // Config should still be old (10)
    let stored_config = fixture.trading.get_config();
    assert_eq!(stored_config.max_positions, 10);
}

#[test]
#[should_panic(expected = "Error(Contract, #304)")]
fn test_set_config_before_unlock() {
    let fixture = TestFixture::create(false);
    fixture.trading.set_status(&0u32); // Active

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 20,
        max_utilization: 10 * SCALAR_7, // 10x
        max_price_age: 900,
        min_open_time: 0,
    };

    fixture.trading.queue_set_config(&config);

    // Try to apply before unlock time
    fixture.trading.set_config();
}

#[test]
fn test_set_config_after_unlock() {
    let fixture = TestFixture::create(false);
    fixture.trading.set_status(&0u32); // Active

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 20,
        max_utilization: 10 * SCALAR_7, // 10x
        max_price_age: 900,
        min_open_time: 0,
    };

    fixture.trading.queue_set_config(&config);

    // Jump forward 1 week + 1 second
    fixture.jump(SECONDS_PER_WEEK + 1);

    fixture.trading.set_config();

    let stored_config = fixture.trading.get_config();
    assert_eq!(stored_config.max_positions, 20);
}

#[test]
fn test_cancel_set_config() {
    let fixture = TestFixture::create(false);
    fixture.trading.set_status(&0u32); // Active

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 20,
        max_utilization: 10 * SCALAR_7, // 10x
        max_price_age: 900,
        min_open_time: 0,
    };

    fixture.trading.queue_set_config(&config);
    fixture.trading.cancel_set_config();

    // Original config should remain
    let stored_config = fixture.trading.get_config();
    assert_eq!(stored_config.max_positions, 10);
}

#[test]
#[should_panic(expected = "Error(Contract, #303)")]
fn test_cancel_set_config_when_not_queued() {
    let fixture = TestFixture::create(false);

    fixture.trading.cancel_set_config();
}

#[test]
#[should_panic(expected = "Error(Contract, #303)")]
fn test_set_config_when_not_queued() {
    let fixture = TestFixture::create(false);

    fixture.trading.set_config();
}

// ==========================================
// Queue/Set Market Tests
// ==========================================

#[test]
fn test_queue_set_market_in_setup_mode() {
    let fixture = TestFixture::create(false);

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();

    fixture.trading.queue_set_market(&market_config);
    fixture.trading.set_market(&market_config.asset);

    // Market should exist at index 0
    let market = fixture.trading.get_market(&0u32);
    assert!(market.config.enabled);
    assert_eq!(market.data.long_notional_size, 0);
    assert_eq!(market.data.short_notional_size, 0);
}

#[test]
fn test_queue_multiple_markets() {
    let fixture = TestFixture::create(false);

    let mut btc_config = default_market(&fixture.env);
    btc_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();

    let mut eth_config = default_market(&fixture.env);
    eth_config.asset = fixture.assets[AssetIndex::ETH as usize].clone();

    fixture.trading.queue_set_market(&btc_config);
    fixture.trading.set_market(&btc_config.asset);

    fixture.trading.queue_set_market(&eth_config);
    fixture.trading.set_market(&eth_config.asset);

    // Both markets should exist
    let btc_market = fixture.trading.get_market(&0u32);
    let eth_market = fixture.trading.get_market(&1u32);

    assert!(matches!(btc_market.config.asset, sep_40_oracle::Asset::Other(ref s) if s == &Symbol::new(&fixture.env, "BTC")));
    assert!(matches!(eth_market.config.asset, sep_40_oracle::Asset::Other(ref s) if s == &Symbol::new(&fixture.env, "ETH")));
}

#[test]
#[should_panic(expected = "Error(Contract, #304)")]
fn test_set_market_before_unlock_active_mode() {
    let fixture = TestFixture::create(false);
    fixture.trading.set_status(&0u32); // Active

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();

    fixture.trading.queue_set_market(&market_config);
    // Try to set immediately in Active mode
    fixture.trading.set_market(&market_config.asset);
}

#[test]
fn test_set_market_after_unlock() {
    let fixture = TestFixture::create(false);
    fixture.trading.set_status(&0u32); // Active

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();

    fixture.trading.queue_set_market(&market_config);

    // Jump forward 1 week + 1 second
    fixture.jump(SECONDS_PER_WEEK + 1);

    fixture.trading.set_market(&market_config.asset);

    let market = fixture.trading.get_market(&0u32);
    assert!(market.config.enabled);
}

#[test]
fn test_cancel_queued_market() {
    let fixture = TestFixture::create(false);
    fixture.trading.set_status(&0u32); // Active

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();

    fixture.trading.queue_set_market(&market_config);
    fixture.trading.cancel_set_market(&market_config.asset);
}

#[test]
#[should_panic(expected = "Error(Contract, #311)")]
fn test_set_market_when_not_queued() {
    let fixture = TestFixture::create(false);

    let asset = sep_40_oracle::Asset::Other(Symbol::new(&fixture.env, "BTC"));

    fixture.trading.set_market(&asset);
}

// ==========================================
// Market Config Validation Tests
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #330)")]
fn test_market_config_zero_maintenance_margin() {
    let fixture = TestFixture::create(false);

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();
    market_config.maintenance_margin = 0;

    fixture.trading.queue_set_market(&market_config);
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_market_config_init_margin_below_maintenance() {
    let fixture = TestFixture::create(false);

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();
    market_config.init_margin = 0_0040000; // 0.4%
    market_config.maintenance_margin = 0_0050000; // 0.5%

    fixture.trading.queue_set_market(&market_config);
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_market_config_min_collateral_below_scalar() {
    let fixture = TestFixture::create(false);

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();
    market_config.min_collateral = SCALAR_7 - 1;

    fixture.trading.queue_set_market(&market_config);
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_market_config_max_below_min_collateral() {
    let fixture = TestFixture::create(false);

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();
    market_config.min_collateral = 100 * SCALAR_7;
    market_config.max_collateral = 50 * SCALAR_7;

    fixture.trading.queue_set_market(&market_config);
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_market_config_ratio_cap_below_1x() {
    let fixture = TestFixture::create(false);

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();
    market_config.ratio_cap = SCALAR_18 - 1;

    fixture.trading.queue_set_market(&market_config);
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_market_config_ratio_cap_above_5x() {
    let fixture = TestFixture::create(false);

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();
    market_config.ratio_cap = 6 * SCALAR_18;

    fixture.trading.queue_set_market(&market_config);
}

#[test]
#[should_panic(expected = "Error(Contract, #330)")]
fn test_market_config_negative_base_fee() {
    let fixture = TestFixture::create(false);

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();
    market_config.base_fee = -1;

    fixture.trading.queue_set_market(&market_config);
}

#[test]
#[should_panic(expected = "Error(Contract, #330)")]
fn test_market_config_zero_price_impact_scalar() {
    let fixture = TestFixture::create(false);

    let mut market_config = default_market(&fixture.env);
    market_config.asset = fixture.assets[AssetIndex::BTC as usize].clone();
    market_config.price_impact_scalar = 0;

    fixture.trading.queue_set_market(&market_config);
}
