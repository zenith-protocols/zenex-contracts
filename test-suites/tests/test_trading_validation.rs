use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::default_market;

// ==========================================
// Helper Functions
// ==========================================

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data(false)
}

// ==========================================
// require_active Tests (via open_position)
// ==========================================

#[test]
fn test_require_active_when_active() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Status is Active after create_fixture_with_data
    // Opening a position requires Active status
    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7),
        &(1000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #380)")]
fn test_require_active_when_on_ice() {
    let fixture = setup_fixture();
    fixture.trading.set_status(&1u32); // OnIce
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // OnIce blocks new positions
    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7),
        &(1000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #380)")]
fn test_require_active_when_frozen() {
    let fixture = setup_fixture();
    fixture.trading.set_status(&2u32); // Frozen
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7),
        &(1000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

// ==========================================
// require_active_or_on_ice Tests (via close_position)
// ==========================================

#[test]
fn test_require_active_or_on_ice_when_active() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let (position_id, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7),
        &(1000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    // Close position when Active - should work
    fixture.trading.close_position(&position_id);
}

#[test]
fn test_require_active_or_on_ice_when_on_ice() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let (position_id, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7),
        &(1000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    fixture.trading.set_status(&1u32); // OnIce

    // Close position when OnIce - should still work
    fixture.trading.close_position(&position_id);
}

#[test]
#[should_panic(expected = "Error(Contract, #380)")]
fn test_require_active_or_on_ice_when_frozen() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let (position_id, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7),
        &(1000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    fixture.trading.set_status(&2u32); // Frozen

    // Close position when Frozen - should panic
    fixture.trading.close_position(&position_id);
}

// ==========================================
// require_market_enabled Tests
// ==========================================

#[test]
fn test_require_market_enabled_when_enabled() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // BTC market is enabled by default from create_fixture_with_data
    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7),
        &(1000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #312)")]
fn test_require_market_enabled_when_disabled() {
    // Create fixture without default setup
    let mut fixture = TestFixture::create(false);

    fixture.token.mint(&fixture.owner, &(100_000_000 * SCALAR_7));
    fixture.vault.deposit(
        &(100_000_000 * SCALAR_7),
        &fixture.owner,
        &fixture.owner,
        &fixture.owner,
    );

    // Create a disabled market
    let mut config = default_market(&fixture.env);
    config.asset = fixture.assets[AssetIndex::BTC as usize].clone();
    config.enabled = false;
    fixture.create_market(&config);

    fixture.trading.set_status(&0u32); // Active

    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7),
        &(1000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #310)")]
fn test_require_market_enabled_market_not_found() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Market index 99 doesn't exist
    fixture.trading.open_position(
        &user,
        &99u32, // Non-existent market
        &(100 * SCALAR_7),
        &(1000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

// ==========================================
// require_valid_config Tests (via initialize/queue_set_config)
// These tests need oracle for max_price_age validation
// ==========================================

#[test]
fn test_require_valid_config_valid() {
    // create_fixture_with_data already validates a valid config
    let _fixture = setup_fixture();
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_require_valid_config_caller_take_rate_over_100() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: SCALAR_7 + 1, // Over 100%
        max_positions: 10,
        max_utilization: 0,
        max_price_age: 900,
    };

    fixture.trading.queue_set_config(&config);
}

#[test]
#[should_panic(expected = "Error(Contract, #330)")]
fn test_require_valid_config_negative_caller_take_rate() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: -1,
        max_positions: 10,
        max_utilization: 0,
        max_price_age: 900,
    };

    fixture.trading.queue_set_config(&config);
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_require_valid_config_max_utilization_below_1x() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 10,
        max_utilization: SCALAR_7 - 1, // Below 1x (but not 0)
        max_price_age: 900,
    };

    fixture.trading.queue_set_config(&config);
}

#[test]
fn test_require_valid_config_max_utilization_disabled() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 10,
        max_utilization: 0, // Disabled
        max_price_age: 900,
    };

    fixture.trading.queue_set_config(&config);
}

#[test]
fn test_require_valid_config_max_utilization_100x() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 10,
        max_utilization: 100 * SCALAR_7, // Max allowed
        max_price_age: 900,
    };

    fixture.trading.queue_set_config(&config);
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_require_valid_config_max_utilization_over_100x() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 10,
        max_utilization: 101 * SCALAR_7, // Over 100x
        max_price_age: 900,
    };

    fixture.trading.queue_set_config(&config);
}

#[test]
#[should_panic(expected = "Error(Contract, #330)")]
fn test_require_valid_config_negative_max_utilization() {
    let fixture = TestFixture::create(false);

    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 10,
        max_utilization: -1,
        max_price_age: 900,
    };

    fixture.trading.queue_set_config(&config);
}

#[test]
#[should_panic(expected = "Error(Contract, #302)")]
fn test_require_valid_config_max_price_age_below_oracle_resolution() {
    let fixture = TestFixture::create(false);

    // Oracle resolution is 300 seconds
    let config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000,
        max_positions: 10,
        max_utilization: 0,
        max_price_age: 300, // Equal to oracle resolution (must be >)
    };

    fixture.trading.queue_set_config(&config);
}
