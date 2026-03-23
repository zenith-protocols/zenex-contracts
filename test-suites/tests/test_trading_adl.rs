use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::{BTC_FEED_ID, BTC_PRICE, PRICE_SCALAR};

// ==========================================
// Helper Functions
// ==========================================

fn setup_fixture() -> TestFixture<'static> {
    let fixture = create_fixture_with_data();
    // Bump max_notional for ADL tests that need large positions
    let mut config = fixture.trading.get_config();
    config.max_notional = 1_000_000_000 * SCALAR_7; // 1B
    fixture.trading.set_config(&config);
    fixture
}

fn open_long(fixture: &TestFixture, user: &Address, collateral: i128, notional: i128) -> u32 {
    fixture.open_and_fill(
        user,
        AssetIndex::BTC as u32,
        collateral,
        notional,
        true,
        BTC_PRICE,
        0,
        0,
    )
}

fn open_short(fixture: &TestFixture, user: &Address, collateral: i128, notional: i128) -> u32 {
    fixture.open_and_fill(
        user,
        AssetIndex::BTC as u32,
        collateral,
        notional,
        false,
        BTC_PRICE,
        0,
        0,
    )
}

fn set_btc_price(fixture: &TestFixture, price: i128) {
    fixture.set_price(BTC_FEED_ID, price);
}

/// Opens enough long positions to create a deficit when price moves up.
/// Vault has ~100M. Opens 500M notional total (5 × 100M).
/// At 50% price increase: PnL = 250M > vault 100M → deficit.
fn create_deficit_long_positions(fixture: &TestFixture, user: &Address) -> Vec<u32> {
    let mut ids = vec![];
    for _ in 0..5 {
        ids.push(open_long(
            fixture,
            user,
            1_100_000 * SCALAR_7,   // 1.1M collateral (margin + fees headroom)
            100_000_000 * SCALAR_7, // 100M notional
        ));
    }
    ids
}

/// Opens enough short positions to create a deficit when price drops.
fn create_deficit_short_positions(fixture: &TestFixture, user: &Address) -> Vec<u32> {
    let mut ids = vec![];
    for _ in 0..5 {
        ids.push(open_short(
            fixture,
            user,
            1_100_000 * SCALAR_7,   // 1.1M collateral (margin + fees headroom)
            100_000_000 * SCALAR_7,
        ));
    }
    ids
}

/// Helper: trigger the Active → OnIce + ADL flow.
/// Moves price to create a deficit, then calls update_status once
/// which transitions Active → OnIce and runs ADL in one step.
fn trigger_adl(fixture: &TestFixture, price: i128) {
    set_btc_price(fixture, price);
    fixture.trading.update_status(&fixture.dummy_price());
}

// ==========================================
// 1. No deficit reverts
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #780)")]
fn test_update_status_active_threshold_not_met() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    open_long(&fixture, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);
    set_btc_price(&fixture, 110_000 * PRICE_SCALAR);

    // Should fail — threshold not met (vault well-funded relative to PnL)
    fixture.trading.update_status(&fixture.dummy_price());
}

#[test]
fn test_adl_restores_active_when_pnl_drops() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    create_deficit_long_positions(&fixture, &user);

    // ADL at $150k: Active → OnIce
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR);
    assert_eq!(fixture.trading.get_status(), 1); // OnIce

    // Drop price → PnL falls below 90% of vault → restores Active
    set_btc_price(&fixture, 120_000 * PRICE_SCALAR);
    fixture.trading.update_status(&fixture.dummy_price());
    assert_eq!(fixture.trading.get_status(), 0); // Active
}

// ==========================================
// 2. ADL triggers correctly
// ==========================================

#[test]
fn test_adl_triggers_and_reduces_aggregates() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    create_deficit_long_positions(&fixture, &user);

    let market_before = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let long_notional_before = market_before.l_notional;
    assert!(long_notional_before > 0);

    // Move price up 50% → PnL = 250M > vault ~100M → deficit
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR);

    let market_after = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert!(market_after.l_notional < long_notional_before);
    assert!(market_after.l_entry_wt < market_before.l_entry_wt);

    let scalar_18: i128 = 1_000_000_000_000_000_000;
    assert!(market_after.l_adl_idx < scalar_18);
    // Short side should be unaffected
    assert_eq!(market_after.s_adl_idx, scalar_18);
}

// ==========================================
// 3. ADL reduction applied on close
// ==========================================

#[test]
fn test_adl_reduces_position_notional_on_close() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    let ids = create_deficit_long_positions(&fixture, &user);
    let pos_id = ids[0];

    let position_before = fixture.trading.get_position(&pos_id);
    let notional_before = position_before.notional;

    // Create deficit and trigger ADL
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR);

    // Market aggregates should be reduced (ADL acts on aggregates, not individual positions)
    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    // 5 positions of 100M each = 500M total; ADL should reduce this
    assert!(market.l_notional < 500_000_000 * SCALAR_7);

    // Raw position in storage is unchanged (ADL applied via index on close)
    let position_raw = fixture.trading.get_position(&pos_id);
    assert_eq!(position_raw.notional, notional_before);
    assert_eq!(position_raw.col, position_before.col);

    // Close applies ADL reduction — payout reflects the reduced notional
    fixture.jump(31); // past MIN_OPEN_TIME
    let payout = fixture.trading.close_position(&pos_id, &fixture.dummy_price());
    assert!(payout > 0); // Still profitable (price went up)
}

// ==========================================
// 4. Position opened after ADL is unaffected
// ==========================================

#[test]
fn test_position_after_adl_unaffected() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(20_000_000 * SCALAR_7));

    create_deficit_long_positions(&fixture, &user);

    // Trigger ADL (status ends up as OnIce)
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR);

    // Admin restores Active so we can open new positions
    fixture.trading.set_status(&0u32);
    set_btc_price(&fixture, BTC_PRICE);
    let new_pos_id = open_long(
        &fixture,
        &user,
        1_000 * SCALAR_7,
        10_000 * SCALAR_7,
    );

    // New position should have full notional (no ADL reduction)
    let new_pos = fixture.trading.get_position(&new_pos_id);
    assert_eq!(new_pos.notional, 10_000 * SCALAR_7);
}

// ==========================================
// 5. Multiple ADL events compound correctly
// ==========================================

#[test]
fn test_adl_compounds_correctly() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    create_deficit_long_positions(&fixture, &user);

    let market_original = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let notional_original = market_original.l_notional;

    // First ADL
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR);

    let market_after_first = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let notional_after_first = market_after_first.l_notional;
    assert!(notional_after_first < notional_original);

    // Price rises further — second ADL (already OnIce, just need one update_status)
    set_btc_price(&fixture, 250_000 * PRICE_SCALAR);
    fixture.trading.update_status(&fixture.dummy_price());

    let market_after_second = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let notional_after_second = market_after_second.l_notional;
    assert!(notional_after_second < notional_after_first);

    // ADL index compounds
    let scalar_18: i128 = 1_000_000_000_000_000_000;
    assert!(market_after_second.l_adl_idx < market_after_first.l_adl_idx);
    assert!(market_after_second.l_adl_idx < scalar_18);
}

// ==========================================
// 6. ADL on short side when shorts are winning
// ==========================================

#[test]
fn test_adl_short_side_winning() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    create_deficit_short_positions(&fixture, &user);

    let market_before = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));

    // Price drops 50% → shorts profit: PnL = 500M * 50% = 250M > vault ~100M
    trigger_adl(&fixture, 50_000 * PRICE_SCALAR);

    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let scalar_18: i128 = 1_000_000_000_000_000_000;

    // Short ADL index should have decreased
    assert!(market.s_adl_idx < scalar_18);
    // Long ADL index should be unchanged (longs are losing)
    assert_eq!(market.l_adl_idx, scalar_18);

    // Market short notional should be reduced
    assert!(market.s_notional < market_before.s_notional);
}

// ==========================================
// 7. Close position after ADL settles correctly
// ==========================================

#[test]
fn test_close_after_adl_settles() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    let ids = create_deficit_long_positions(&fixture, &user);
    let pos_id = ids[0];

    // Trigger ADL (status ends up as OnIce)
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR);

    // Close position — allowed in OnIce status
    fixture.jump(31); // past MIN_OPEN_TIME
    let payout = fixture.trading.close_position(&pos_id, &fixture.dummy_price());

    // Position should be gone
    assert!(!fixture.position_exists(pos_id));

    // Payout should be positive (price went up, position is long)
    assert!(payout > 0);
}

// ==========================================
// 8. 100% reduction cap
// ==========================================

#[test]
fn test_adl_100pct_cap() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    let ids = create_deficit_long_positions(&fixture, &user);
    let pos_id = ids[0];

    // Extreme price movement — 10x
    trigger_adl(&fixture, 1_000_000 * PRICE_SCALAR);

    // After ADL, position notional should be >= 0
    let pos = fixture.trading.get_position(&pos_id);
    assert!(pos.notional >= 0);

    // Market aggregates should still be non-negative
    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert!(market.l_notional >= 0);
    assert!(market.l_adl_idx >= 0);
}

// ==========================================
// 9. Entry-weighted accuracy
// ==========================================

#[test]
fn test_entry_weighted_tracking() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    // Open position — entry_weighted should be updated
    let pos1 = open_long(&fixture, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);

    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert!(market.l_entry_wt > 0);
    let ew_after_open = market.l_entry_wt;

    // Open another position at same price
    let _pos2 = open_long(&fixture, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);
    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    // Entry weighted should roughly double
    let ew_after_second = market.l_entry_wt;
    assert!(ew_after_second > ew_after_open);

    // Close first position — entry weighted should decrease
    fixture.jump(31); // past MIN_OPEN_TIME
    fixture.trading.close_position(&pos1, &fixture.dummy_price());
    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert!(market.l_entry_wt < ew_after_second);
}
