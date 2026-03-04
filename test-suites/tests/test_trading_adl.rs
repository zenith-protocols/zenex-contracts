use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
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

fn open_long(fixture: &TestFixture, user: &Address, collateral: i128, notional: i128) -> u32 {
    let (id, _) = fixture.open_and_fill(
        user,
        AssetIndex::BTC as u32,
        collateral,
        notional,
        true,
        BTC_PRICE,
        0,
        0,
    );
    id
}

fn open_short(fixture: &TestFixture, user: &Address, collateral: i128, notional: i128) -> u32 {
    let (id, _) = fixture.open_and_fill(
        user,
        AssetIndex::BTC as u32,
        collateral,
        notional,
        false,
        BTC_PRICE,
        0,
        0,
    );
    id
}

fn set_btc_price(fixture: &TestFixture, price: i128) {
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,    // USD
        price,        // BTC
        2000_0000000, // ETH
        0_1000000,    // XLM
    ]);
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
            1_000_000 * SCALAR_7,   // 1M collateral
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
            1_000_000 * SCALAR_7,
            100_000_000 * SCALAR_7,
        ));
    }
    ids
}

// ==========================================
// 1. No deficit reverts
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #781)")]
fn test_adl_reverts_when_not_on_ice() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    open_long(&fixture, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);
    set_btc_price(&fixture, 110_000 * SCALAR_7);

    // Should fail — contract is Active, not OnIce
    fixture.trading.trigger_adl();
}

#[test]
#[should_panic(expected = "Error(Contract, #780)")]
fn test_adl_no_deficit_reverts() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open a small position, vault is well-funded
    open_long(&fixture, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);

    // Price goes up (longs profit) but vault has 100M — no deficit
    set_btc_price(&fixture, 110_000 * SCALAR_7);

    // Set to AdminOnIce so ADL can be attempted
    fixture.trading.set_status(&2u32);
    fixture.trading.trigger_adl();
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

    let market_before = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    let long_notional_before = market_before.data.long_notional_size;
    assert!(long_notional_before > 0);

    // Move price up 50% → unrealized PnL = 500M * 50% = 250M > vault ~100M
    set_btc_price(&fixture, 150_000 * SCALAR_7);

    fixture.trading.set_status(&2u32); // AdminOnIce
    fixture.trading.trigger_adl();

    let market_after = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    assert!(market_after.data.long_notional_size < long_notional_before);
    assert!(market_after.data.long_entry_weighted < market_before.data.long_entry_weighted);

    let scalar_18: i128 = 1_000_000_000_000_000_000;
    assert!(market_after.data.long_adl_index < scalar_18);
    // Short side should be unaffected
    assert_eq!(market_after.data.short_adl_index, scalar_18);
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
    let notional_before = position_before.notional_size;

    // Create deficit
    set_btc_price(&fixture, 150_000 * SCALAR_7);
    fixture.trading.set_status(&2u32); // AdminOnIce
    fixture.trading.trigger_adl();

    // Market aggregates should be reduced (ADL acts on aggregates, not individual positions)
    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    // 5 positions of 100M each = 500M total; ADL should reduce this
    assert!(market.data.long_notional_size < 500_000_000 * SCALAR_7);

    // Raw position in storage is unchanged (ADL applied via index on close)
    let position_raw = fixture.trading.get_position(&pos_id);
    assert_eq!(position_raw.notional_size, notional_before);
    assert_eq!(position_raw.collateral, position_before.collateral);

    // Close applies ADL reduction — pnl reflects the reduced notional
    let (pnl, _fee) = fixture.trading.close_position(&pos_id);
    assert!(pnl > 0); // Still profitable (price went up)
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

    // Trigger ADL
    set_btc_price(&fixture, 150_000 * SCALAR_7);
    fixture.trading.set_status(&2u32); // AdminOnIce
    fixture.trading.trigger_adl();

    // Restore to Active and open a new position
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
    assert_eq!(new_pos.notional_size, 10_000 * SCALAR_7);
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

    let market_original = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    let notional_original = market_original.data.long_notional_size;

    // First ADL
    set_btc_price(&fixture, 150_000 * SCALAR_7);
    fixture.trading.set_status(&2u32); // AdminOnIce
    fixture.trading.trigger_adl();

    let market_after_first = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    let notional_after_first = market_after_first.data.long_notional_size;
    assert!(notional_after_first < notional_original);

    // Price rises further — second ADL
    set_btc_price(&fixture, 250_000 * SCALAR_7);
    fixture.trading.trigger_adl();

    let market_after_second = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    let notional_after_second = market_after_second.data.long_notional_size;
    assert!(notional_after_second < notional_after_first);

    // ADL index compounds
    let scalar_18: i128 = 1_000_000_000_000_000_000;
    assert!(market_after_second.data.long_adl_index < market_after_first.data.long_adl_index);
    assert!(market_after_second.data.long_adl_index < scalar_18);
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

    let market_before = fixture.trading.get_market(&(AssetIndex::BTC as u32));

    // Price drops 50% → shorts profit: PnL = 500M * 50% = 250M > vault ~100M
    set_btc_price(&fixture, 50_000 * SCALAR_7);

    fixture.trading.set_status(&2u32); // AdminOnIce
    fixture.trading.trigger_adl();

    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    let scalar_18: i128 = 1_000_000_000_000_000_000;

    // Short ADL index should have decreased
    assert!(market.data.short_adl_index < scalar_18);
    // Long ADL index should be unchanged (longs are losing)
    assert_eq!(market.data.long_adl_index, scalar_18);

    // Market short notional should be reduced
    assert!(market.data.short_notional_size < market_before.data.short_notional_size);
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

    // Trigger ADL
    set_btc_price(&fixture, 150_000 * SCALAR_7);
    fixture.trading.set_status(&2u32); // AdminOnIce
    fixture.trading.trigger_adl();

    // Close position — should not panic (allowed in AdminOnIce)
    let (pnl, fee) = fixture.trading.close_position(&pos_id);

    // Position should be gone
    assert!(!fixture.position_exists(pos_id));

    // PnL should still be positive (price went up, position is long)
    assert!(pnl > 0);
    assert!(fee > 0);
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
    set_btc_price(&fixture, 1_000_000 * SCALAR_7);

    fixture.trading.set_status(&2u32); // AdminOnIce
    fixture.trading.trigger_adl();

    // After ADL, position notional should be >= 0
    let pos = fixture.trading.get_position(&pos_id);
    assert!(pos.notional_size >= 0);

    // Market aggregates should still be non-negative
    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    assert!(market.data.long_notional_size >= 0);
    assert!(market.data.long_adl_index >= 0);
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

    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    assert!(market.data.long_entry_weighted > 0);
    let ew_after_open = market.data.long_entry_weighted;

    // Open another position at same price
    let _pos2 = open_long(&fixture, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);
    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    // Entry weighted should roughly double
    let ew_after_second = market.data.long_entry_weighted;
    assert!(ew_after_second > ew_after_open);

    // Close first position — entry weighted should decrease
    fixture.trading.close_position(&pos1);
    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    assert!(market.data.long_entry_weighted < ew_after_second);
}
