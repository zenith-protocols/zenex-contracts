use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::pyth_helper;
use test_suites::SCALAR_7;
use trading::testutils::{BTC_FEED_ID, PRICE_SCALAR};

const SCALAR_18: i128 = 1_000_000_000_000_000_000;

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
    // entry_price is i64 for real price verifier: BTC $100k = 10_000_000_000_000 raw
    fixture.open_and_fill(
        user,
        AssetIndex::BTC as u32,
        collateral,
        notional,
        true,
        10_000_000_000_000i64, // $100k in Pyth raw format
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
        10_000_000_000_000i64, // $100k in Pyth raw format
        0,
        0,
    )
}

/// Build a multi-feed price update with custom BTC price and default ETH/XLM prices.
/// All markets must have prices when calling update_status.
fn price_update_btc(fixture: &TestFixture, btc_price_raw: i64) -> soroban_sdk::Bytes {
    let ts = fixture.env.ledger().timestamp();
    pyth_helper::build_price_update(
        &fixture.env,
        &fixture.signing_key,
        &[
            pyth_helper::FeedInput {
                feed_id: 1, // BTC
                price: btc_price_raw,
                exponent: -8,
                confidence: None,
            },
            pyth_helper::FeedInput {
                feed_id: 2, // ETH
                price: 200_000_000_000, // $2k default
                exponent: -8,
                confidence: None,
            },
            pyth_helper::FeedInput {
                feed_id: 3, // XLM
                price: 10_000_000, // $0.10 default
                exponent: -8,
                confidence: None,
            },
        ],
        ts,
    )
}

/// Opens enough long positions to create a deficit when price moves up.
/// Vault has ~100M. Opens 500M notional total (5 x 100M).
/// At 50% price increase: PnL = 250M > vault 100M -> deficit.
fn create_deficit_long_positions(fixture: &TestFixture, user: &Address) -> Vec<u32> {
    let mut ids = vec![];
    for _ in 0..5 {
        ids.push(open_long(
            fixture,
            user,
            1_100_000 * SCALAR_7,   // 1.1M collateral (margin + fees headroom)
            100_000_000 * SCALAR_7,  // 100M notional
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
            100_000_000 * SCALAR_7,  // 100M notional
        ));
    }
    ids
}

/// Helper: trigger the Active -> OnIce + ADL flow.
/// Moves BTC price to create a deficit, then calls update_status which
/// transitions Active -> OnIce and runs ADL in one step.
fn trigger_adl(fixture: &TestFixture, btc_price_raw: i64) {
    let price_bytes = price_update_btc(fixture, btc_price_raw);
    fixture.trading.update_status(&price_bytes);
}

// ==========================================
// 1. No deficit reverts
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #780)")]
fn test_adl_threshold_not_met_reverts() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Small position -- ~80% utilization is far below UTIL_ONICE (95%)
    open_long(&fixture, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);

    // Price up 10% -- PnL ~1000 tokens << vault 100M
    let price_bytes = price_update_btc(&fixture, 110_000 * PRICE_SCALAR as i64);
    fixture.trading.update_status(&price_bytes);
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

    // Move price up 50% -> PnL = 250M > vault ~100M -> deficit
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR as i64);

    let market_after = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert!(market_after.l_notional < long_notional_before);
    assert!(market_after.l_entry_wt < market_before.l_entry_wt);
    assert!(market_after.l_adl_idx < SCALAR_18);
    // Short side should be unaffected
    assert_eq!(market_after.s_adl_idx, SCALAR_18);
}

// ==========================================
// 3. Close position after ADL settles correctly
// ==========================================

#[test]
fn test_adl_close_after_adl_settles() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    let ids = create_deficit_long_positions(&fixture, &user);
    let pos_id = ids[0];

    let position_before = fixture.trading.get_position(&pos_id);
    let notional_before = position_before.notional;

    // Create deficit and trigger ADL
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR as i64);

    // Market aggregates should be reduced (ADL acts on aggregates, not individual positions)
    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert!(market.l_notional < 500_000_000 * SCALAR_7);

    // Raw position in storage is unchanged (ADL applied via index on close)
    let position_raw = fixture.trading.get_position(&pos_id);
    assert_eq!(position_raw.notional, notional_before);

    // Close applies ADL reduction -- payout reflects the reduced notional
    fixture.jump(31); // past MIN_OPEN_TIME
    let close_price_bytes = price_update_btc(&fixture, 150_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&pos_id, &close_price_bytes);
    assert!(payout > 0); // Still profitable (price went up)
}

// ==========================================
// 4. ADL on short side when shorts are winning
// ==========================================

#[test]
fn test_adl_on_short_side() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    create_deficit_short_positions(&fixture, &user);

    let market_before = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));

    // Price drops 50% -> shorts profit: PnL = 500M * 50% = 250M > vault ~100M
    trigger_adl(&fixture, 50_000 * PRICE_SCALAR as i64);

    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));

    // Short ADL index should have decreased
    assert!(market.s_adl_idx < SCALAR_18);
    // Long ADL index should be unchanged (longs are losing)
    assert_eq!(market.l_adl_idx, SCALAR_18);
    // Market short notional should be reduced
    assert!(market.s_notional < market_before.s_notional);
}

// ==========================================
// 5. ADL restores Active when utilization drops
// ==========================================

#[test]
fn test_adl_restores_active_when_utilization_drops() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    create_deficit_long_positions(&fixture, &user);

    // ADL at $150k: Active -> OnIce
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR as i64);
    assert_eq!(fixture.trading.get_status(), 1); // OnIce

    // Drop price -> PnL falls below 90% of vault -> restores Active
    let price_bytes = price_update_btc(&fixture, 120_000 * PRICE_SCALAR as i64);
    fixture.trading.update_status(&price_bytes);
    assert_eq!(fixture.trading.get_status(), 0); // Active
}

// ==========================================
// OPTIONAL TESTS (Extended Parity)
// ==========================================

// 6. Multiple ADL events compound correctly

#[test]
fn test_adl_compounds_correctly() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    create_deficit_long_positions(&fixture, &user);

    let market_original = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let notional_original = market_original.l_notional;

    // First ADL
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR as i64);

    let market_after_first = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let notional_after_first = market_after_first.l_notional;
    assert!(notional_after_first < notional_original);

    // Price rises further -- second ADL (already OnIce, just need one update_status)
    let price_bytes = price_update_btc(&fixture, 250_000 * PRICE_SCALAR as i64);
    fixture.trading.update_status(&price_bytes);

    let market_after_second = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let notional_after_second = market_after_second.l_notional;
    assert!(notional_after_second < notional_after_first);

    // ADL index compounds
    assert!(market_after_second.l_adl_idx < market_after_first.l_adl_idx);
    assert!(market_after_second.l_adl_idx < SCALAR_18);
}

// 7. Position opened after ADL is unaffected

#[test]
fn test_adl_position_after_adl_unaffected() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(20_000_000 * SCALAR_7));

    create_deficit_long_positions(&fixture, &user);

    // Trigger ADL (status ends up as OnIce)
    trigger_adl(&fixture, 150_000 * PRICE_SCALAR as i64);

    // Admin restores Active so we can open new positions
    fixture.trading.set_status(&0u32);
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

// 8. 100% reduction cap

#[test]
fn test_adl_100_percent_reduction_cap() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    let ids = create_deficit_long_positions(&fixture, &user);
    let pos_id = ids[0];

    // Extreme price movement -- 10x
    trigger_adl(&fixture, 1_000_000 * PRICE_SCALAR as i64);

    // After ADL, position notional should still be >= 0
    let pos = fixture.trading.get_position(&pos_id);
    assert!(pos.notional >= 0);

    // Market aggregates should still be non-negative
    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert!(market.l_notional >= 0);
    assert!(market.l_adl_idx >= 0);
}

// 9. Entry-weighted accuracy tracking

#[test]
fn test_adl_entry_weighted_tracking() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    // Open position -- entry_weighted should be updated
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

    // Close first position -- entry weighted should decrease
    fixture.jump(31); // past MIN_OPEN_TIME
    let close_bytes = fixture.btc_price(10_000_000_000_000i64);
    fixture.trading.close_position(&pos1, &close_bytes);
    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert!(market.l_entry_wt < ew_after_second);
}
