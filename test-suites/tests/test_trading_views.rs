use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::BTC_PRICE;

// ==========================================
// Helper Functions
// ==========================================

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data(false)
}

// ==========================================
// get_config
// ==========================================

#[test]
fn test_get_config() {
    let fixture = setup_fixture();

    let config = fixture.trading.get_config();

    assert_eq!(config.max_positions, 10);
    assert_eq!(config.max_utilization, 10 * SCALAR_7);
    assert_eq!(config.max_price_age, 900);
    assert_eq!(config.caller_take_rate, 0);
}

// ==========================================
// get_status
// ==========================================

#[test]
fn test_get_status_active() {
    let fixture = setup_fixture();

    let status = fixture.trading.get_status();
    assert_eq!(status, 0); // Active
}

#[test]
fn test_get_status_after_change() {
    let fixture = setup_fixture();

    fixture.trading.set_status(&1u32); // OnIce
    assert_eq!(fixture.trading.get_status(), 1);

    fixture.trading.set_status(&2u32); // Frozen
    assert_eq!(fixture.trading.get_status(), 2);
}

// ==========================================
// get_vault
// ==========================================

#[test]
fn test_get_vault() {
    let fixture = setup_fixture();

    let vault = fixture.trading.get_vault();
    assert_eq!(vault, fixture.vault.address);
}

// ==========================================
// get_token
// ==========================================

#[test]
fn test_get_token() {
    let fixture = setup_fixture();

    let token = fixture.trading.get_token();
    assert_eq!(token, fixture.token.address);
}

// ==========================================
// get_market
// ==========================================

#[test]
fn test_get_market_btc() {
    let fixture = setup_fixture();

    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));

    assert_eq!(market.asset_index, AssetIndex::BTC as u32);
    assert!(market.config.enabled);
    assert_eq!(market.config.min_collateral, SCALAR_7);
    assert_eq!(market.config.max_collateral, 1_000_000 * SCALAR_7);
    assert_eq!(market.config.init_margin, 0_0100000);
    assert_eq!(market.config.maintenance_margin, 0_0050000);
    assert_eq!(market.config.base_fee, 0_0005000);

    // Fresh market should have zero open interest
    assert_eq!(market.data.long_notional_size, 0);
    assert_eq!(market.data.short_notional_size, 0);
}

#[test]
fn test_get_market_eth() {
    let fixture = setup_fixture();

    let market = fixture.trading.get_market(&(AssetIndex::ETH as u32));

    assert_eq!(market.asset_index, AssetIndex::ETH as u32);
    assert!(market.config.enabled);
}

#[test]
fn test_get_market_reflects_open_interest() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open a long position
    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    assert_eq!(market.data.long_notional_size, 10_000 * SCALAR_7);
    assert_eq!(market.data.short_notional_size, 0);

    // Open a short position
    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(500 * SCALAR_7),
        &(5_000 * SCALAR_7),
        &false,
        &0,
        &0,
        &0,
    );

    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    assert_eq!(market.data.long_notional_size, 10_000 * SCALAR_7);
    assert_eq!(market.data.short_notional_size, 5_000 * SCALAR_7);
}

#[test]
#[should_panic(expected = "Error(Contract, #310)")]
fn test_get_market_not_found() {
    let fixture = setup_fixture();

    // Index 99 doesn't exist
    fixture.trading.get_market(&99u32);
}

// ==========================================
// get_position
// ==========================================

#[test]
fn test_get_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let (position_id, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    let position = fixture.trading.get_position(&position_id);

    assert_eq!(position.id, position_id);
    assert_eq!(position.user, user);
    assert!(position.filled);
    assert_eq!(position.asset_index, AssetIndex::BTC as u32);
    assert!(position.is_long);
    assert_eq!(position.collateral, 1_000 * SCALAR_7);
    assert_eq!(position.notional_size, 10_000 * SCALAR_7);
    assert_eq!(position.entry_price, BTC_PRICE);
    assert_eq!(position.take_profit, 0);
    assert_eq!(position.stop_loss, 0);
}

#[test]
fn test_get_position_limit_order() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = BTC_PRICE + 1000 * SCALAR_7;
    let (position_id, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &entry_price,
        &0,
        &0,
    );

    let position = fixture.trading.get_position(&position_id);

    assert_eq!(position.id, position_id);
    assert!(!position.filled); // Limit order is pending
    assert_eq!(position.entry_price, entry_price);
}

#[test]
#[should_panic(expected = "Error(Contract, #325)")]
fn test_get_position_not_found() {
    let fixture = setup_fixture();

    fixture.trading.get_position(&999u32);
}

// ==========================================
// get_user_positions
// ==========================================

#[test]
fn test_get_user_positions_empty() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    let positions = fixture.trading.get_user_positions(&user);
    assert_eq!(positions.len(), 0);
}

#[test]
fn test_get_user_positions_multiple() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let (id1, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    let (id2, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::ETH as u32),
        &(500 * SCALAR_7),
        &(5_000 * SCALAR_7),
        &false,
        &0,
        &0,
        &0,
    );

    let positions = fixture.trading.get_user_positions(&user);
    assert_eq!(positions.len(), 2);
    assert_eq!(positions.get(0).unwrap(), id1);
    assert_eq!(positions.get(1).unwrap(), id2);
}

#[test]
fn test_get_user_positions_after_close() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let (id1, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    let (id2, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::ETH as u32),
        &(500 * SCALAR_7),
        &(5_000 * SCALAR_7),
        &false,
        &0,
        &0,
        &0,
    );

    // Close first position
    fixture.trading.close_position(&id1);

    let positions = fixture.trading.get_user_positions(&user);
    assert_eq!(positions.len(), 1);
    assert_eq!(positions.get(0).unwrap(), id2);
}

#[test]
fn test_get_user_positions_different_users() {
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);
    fixture.token.mint(&user1, &(100_000 * SCALAR_7));
    fixture.token.mint(&user2, &(100_000 * SCALAR_7));

    fixture.trading.open_position(
        &user1,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    fixture.trading.open_position(
        &user2,
        &(AssetIndex::BTC as u32),
        &(500 * SCALAR_7),
        &(5_000 * SCALAR_7),
        &false,
        &0,
        &0,
        &0,
    );

    assert_eq!(fixture.trading.get_user_positions(&user1).len(), 1);
    assert_eq!(fixture.trading.get_user_positions(&user2).len(), 1);
}
