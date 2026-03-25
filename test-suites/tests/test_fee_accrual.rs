use soroban_fixed_point_math::FixedPoint;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::{default_config, default_market};

#[allow(dead_code)]
const SCALAR_18: i128 = 1_000_000_000_000_000_000;
const SECONDS_IN_HOUR: u64 = 3600;

/// BTC $100k as i64 for price_for_feed / btc_price
const BTC_100K: i64 = 10_000_000_000_000;

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data()
}

// ==========================================
// 1. Funding is zero-sum (token conservation)
// ==========================================

#[test]
fn test_funding_is_zero_sum() {
    let fixture = setup_fixture();
    let user_long = Address::generate(&fixture.env);
    let user_short = Address::generate(&fixture.env);

    fixture.token.mint(&user_long, &(500_000 * SCALAR_7));
    fixture.token.mint(&user_short, &(500_000 * SCALAR_7));

    // Record total system tokens before trading
    let total_before = fixture.token.balance(&user_long)
        + fixture.token.balance(&user_short)
        + fixture.token.balance(&fixture.vault.address)
        + fixture.token.balance(&fixture.treasury.address)
        + fixture.token.balance(&fixture.trading.address);

    // Open unequal sides: 2x long vs 1x short (long is dominant, funding flows L->S)
    let long_notional = 20_000 * SCALAR_7;
    let short_notional = 10_000 * SCALAR_7;

    let long_pos = fixture.open_and_fill(
        &user_long,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        long_notional,
        true,
        BTC_100K,
        0,
        0,
    );

    let short_pos = fixture.open_and_fill(
        &user_short,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        short_notional,
        false,
        BTC_100K,
        0,
        0,
    );

    // Let 1 hour pass, then apply_funding to set the funding rate
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // Let another hour pass so funding accrues
    fixture.jump(SECONDS_IN_HOUR);

    // Close both positions at the same price (no PnL from price movement)
    let close_bytes = fixture.btc_price(BTC_100K);
    let payout_long = fixture.trading.close_position(&long_pos, &close_bytes);
    let payout_short = fixture.trading.close_position(&short_pos, &close_bytes);

    assert!(payout_long > 0, "long should have some payout");
    assert!(payout_short > 0, "short should have some payout");

    // Record total system tokens after all trades settled
    let total_after = fixture.token.balance(&user_long)
        + fixture.token.balance(&user_short)
        + fixture.token.balance(&fixture.vault.address)
        + fixture.token.balance(&fixture.treasury.address)
        + fixture.token.balance(&fixture.trading.address);

    // Total tokens must be conserved within rounding tolerance (2 units per position = 4 total)
    let diff = (total_after - total_before).abs();
    assert!(
        diff <= 4,
        "Token conservation violated: total_before={}, total_after={}, diff={}",
        total_before,
        total_after,
        diff
    );
}

// ==========================================
// 2. Funding: dominant side pays
// ==========================================

#[test]
fn test_funding_dominant_side_pays() {
    let fixture = setup_fixture();
    let user_long = Address::generate(&fixture.env);
    let user_short = Address::generate(&fixture.env);

    fixture.token.mint(&user_long, &(500_000 * SCALAR_7));
    fixture.token.mint(&user_short, &(500_000 * SCALAR_7));

    let initial_long = fixture.token.balance(&user_long);
    let initial_short = fixture.token.balance(&user_short);

    // Open $20k long (dominant) and $10k short (non-dominant)
    let long_pos = fixture.open_and_fill(
        &user_long,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        20_000 * SCALAR_7,
        true,
        BTC_100K,
        0,
        0,
    );

    let short_pos = fixture.open_and_fill(
        &user_short,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        10_000 * SCALAR_7,
        false,
        BTC_100K,
        0,
        0,
    );

    // Apply funding + accrue for 1 hour
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();
    fixture.jump(SECONDS_IN_HOUR);

    // Close both at same price (no PnL from price movement)
    let close_bytes = fixture.btc_price(BTC_100K);
    fixture.trading.close_position(&long_pos, &close_bytes);
    fixture.trading.close_position(&short_pos, &close_bytes);

    let final_long = fixture.token.balance(&user_long);
    let final_short = fixture.token.balance(&user_short);

    // Both lose money due to fees, but the long (dominant) should lose MORE than the short
    // because the long pays both borrowing (dominant side) and funding (dominant direction)
    let long_delta = final_long - initial_long;
    let short_delta = final_short - initial_short;

    // Both deltas are negative (both lose from fees)
    assert!(long_delta < 0, "long should lose money from fees");
    assert!(short_delta < 0, "short should lose money from fees");

    // The dominant long should lose more than the non-dominant short
    // (long pays higher open fee, borrowing, and funding; short receives funding)
    assert!(
        long_delta < short_delta,
        "dominant side (long) should lose more: long_delta={}, short_delta={}",
        long_delta,
        short_delta
    );
}

// ==========================================
// 3. Funding rate is zero with balanced sides
// ==========================================

#[test]
fn test_funding_rate_zero_with_balanced_sides() {
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);

    fixture.token.mint(&user1, &(500_000 * SCALAR_7));
    fixture.token.mint(&user2, &(500_000 * SCALAR_7));

    // Open equal long and short notionals
    let _long_pos = fixture.open_and_fill(
        &user1,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        10_000 * SCALAR_7,
        true,
        BTC_100K,
        0,
        0,
    );

    let _short_pos = fixture.open_and_fill(
        &user2,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        10_000 * SCALAR_7,
        false,
        BTC_100K,
        0,
        0,
    );

    // Apply funding to compute the rate
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // When L == S, funding rate should be 0
    let market_data = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert_eq!(
        market_data.fund_rate, 0,
        "funding rate should be zero with balanced sides, got {}",
        market_data.fund_rate
    );
}

// ==========================================
// 4. Borrowing curve at multiple utilization points
// ==========================================

#[test]
fn test_borrowing_curve_at_utilization_points() {
    // We test the borrowing curve at multiple utilization levels.
    // Formula: r_base * (1 + r_var * util^5) * r_borrow
    // From default_config(): r_base = 10_000_000_000_000 (SCALAR_18), r_var = SCALAR_7, r_borrow = SCALAR_7
    // At util=0: rate = r_base * 1 * 1 = r_base
    // At util=100%: rate = r_base * (1 + 1) * 1 = 2 * r_base
    //
    // Strategy: Open positions to reach target utilization, then jump 1 hour.
    // The borrowing index delta over 1 hour equals the hourly rate:
    //   borrow_delta = borr_rate * seconds / ONE_HOUR = borr_rate * 1
    // So l_borr_idx after 1 hour accrual = borr_rate (starting from 0).

    let config = default_config();
    let r_base = config.r_base;
    let r_var = config.r_var;
    let e_standalone = soroban_sdk::Env::default();
    let market_config = default_market(&e_standalone);
    let r_borrow = market_config.r_borrow;

    // Test at 4 utilization points: 25%, 50%, 75%, 90%
    let util_points: [(i128, &str); 4] = [
        (2_500_000, "25%"),  // 25% in SCALAR_7
        (5_000_000, "50%"),  // 50%
        (7_500_000, "75%"),  // 75%
        (9_000_000, "90%"),  // 90%
    ];

    for (util_s7, label) in util_points.iter() {
        let fixture = setup_fixture();
        let user = Address::generate(&fixture.env);
        fixture.token.mint(&user, &(500_000_000 * SCALAR_7));

        // Bump max_notional so large positions are allowed
        let mut trading_config = fixture.trading.get_config();
        trading_config.max_notional = 1_000_000_000 * SCALAR_7; // 1B
        fixture.trading.set_config(&trading_config);

        // Vault has 100_000_000 * SCALAR_7 (100M). To reach target utilization:
        // total_notional = util * vault_balance
        let vault_balance: i128 = 100_000_000 * SCALAR_7;
        let target_notional = (*util_s7 as i128)
            .fixed_mul_floor(vault_balance, SCALAR_7)
            .unwrap();

        // Open a big long position to reach target utilization
        // Use generous collateral to avoid margin violations
        let collateral = target_notional / 2; // ~2x leverage
        let _pos = fixture.open_and_fill(
            &user,
            AssetIndex::BTC as u32,
            collateral,
            target_notional,
            true,
            BTC_100K,
            0,
            0,
        );

        // Read initial borrowing index (should be 0 after first accrual at open time)
        let market_before = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
        let borr_idx_before = market_before.l_borr_idx;

        // Jump exactly 1 hour and apply_funding to trigger accrual
        fixture.jump(SECONDS_IN_HOUR);
        fixture.trading.apply_funding();

        // Read market data: borrowing index delta = borr_rate * 1 hour / 1 hour = borr_rate
        let market_after = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
        let observed_borr_delta = market_after.l_borr_idx - borr_idx_before;

        // Compute expected rate off-chain:
        // util^5 in SCALAR_7 precision
        let u2 = (*util_s7 as i128)
            .fixed_mul_ceil(*util_s7 as i128, SCALAR_7)
            .unwrap();
        let u4 = u2.fixed_mul_ceil(u2, SCALAR_7).unwrap();
        let u5 = u4.fixed_mul_ceil(*util_s7 as i128, SCALAR_7).unwrap();

        // multiplier = 1 + r_var * util^5 (in SCALAR_7)
        let util_factor = r_var.fixed_mul_ceil(u5, SCALAR_7).unwrap();
        let multiplier = SCALAR_7 + util_factor;

        // rate = r_base * multiplier * r_borrow (this is per-hour in SCALAR_18)
        let global_rate = r_base
            .fixed_mul_ceil(multiplier, SCALAR_7)
            .unwrap();
        let expected_borr_rate = global_rate
            .fixed_mul_ceil(r_borrow, SCALAR_7)
            .unwrap();

        // After 1 hour, the borrow delta should equal the hourly rate
        // Allow 5% tolerance because vault_balance changes after opening (fees paid to
        // vault/treasury on open), which changes the actual utilization slightly.
        let tolerance_pct = 5; // 5%
        let abs_diff = (observed_borr_delta - expected_borr_rate).abs();
        let max_diff = expected_borr_rate * tolerance_pct / 100;
        assert!(
            abs_diff <= max_diff,
            "Borrowing rate at {} off by too much: observed_delta={}, expected_rate={}, diff={}, max_diff={}",
            label,
            observed_borr_delta,
            expected_borr_rate,
            abs_diff,
            max_diff
        );
    }
}

// ==========================================
// 5. Borrowing: dominant side only
// ==========================================

#[test]
fn test_borrowing_dominant_side_only() {
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);

    fixture.token.mint(&user1, &(500_000 * SCALAR_7));
    fixture.token.mint(&user2, &(500_000 * SCALAR_7));

    // Long dominant: 20k long > 10k short
    let _long_pos = fixture.open_and_fill(
        &user1,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        20_000 * SCALAR_7,
        true,
        BTC_100K,
        0,
        0,
    );

    let _short_pos = fixture.open_and_fill(
        &user2,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        10_000 * SCALAR_7,
        false,
        BTC_100K,
        0,
        0,
    );

    // Read initial indices
    let market_initial = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let initial_l_borr_idx = market_initial.l_borr_idx;
    let initial_s_borr_idx = market_initial.s_borr_idx;

    // Jump 1 hour and apply funding to trigger accrual
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // Read updated indices
    let market_after = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));

    // Long is dominant -> long borrowing index should advance
    assert!(
        market_after.l_borr_idx > initial_l_borr_idx,
        "dominant side (long) borrowing index should advance: initial={}, after={}",
        initial_l_borr_idx,
        market_after.l_borr_idx
    );

    // Short is non-dominant -> short borrowing index should be unchanged
    assert_eq!(
        market_after.s_borr_idx, initial_s_borr_idx,
        "non-dominant side (short) borrowing index should not change: initial={}, after={}",
        initial_s_borr_idx,
        market_after.s_borr_idx
    );
}

// ==========================================
// 6. Fee accrual increases with time
// ==========================================

#[test]
fn test_fee_accrual_increases_with_time() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(500_000 * SCALAR_7));

    // Open a position
    let pos = fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        20_000 * SCALAR_7,
        true,
        BTC_100K,
        0,
        0,
    );

    // Jump 1 hour and apply funding to set the rate
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // Record borrowing index after 1 hour
    let market_1h = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let borr_idx_1h = market_1h.l_borr_idx;

    // Jump another hour
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // Record borrowing index after 2 hours
    let market_2h = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    let borr_idx_2h = market_2h.l_borr_idx;

    // Borrowing index should be larger after 2 hours than after 1 hour
    assert!(
        borr_idx_2h > borr_idx_1h,
        "borrowing index should increase with time: 1h={}, 2h={}",
        borr_idx_1h,
        borr_idx_2h
    );

    // Close the position after 2 hours
    let close_bytes = fixture.btc_price(BTC_100K);
    let payout = fixture.trading.close_position(&pos, &close_bytes);
    assert!(payout > 0, "position should have some payout");

    // The payout should be less than the collateral deposited (fees accumulated)
    // Collateral was 5000, but fees were deducted on open so effective is less.
    // Borrowing and funding fees further reduce the payout.
    let final_balance = fixture.token.balance(&user);
    let total_cost = 500_000 * SCALAR_7 - final_balance;
    assert!(total_cost > 0, "user should have paid net fees");
}

// ==========================================
// 7. Token conservation after open/close cycle
// ==========================================

#[test]
fn test_token_conservation_after_open_close() {
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);
    let user3 = Address::generate(&fixture.env);

    fixture.token.mint(&user1, &(500_000 * SCALAR_7));
    fixture.token.mint(&user2, &(500_000 * SCALAR_7));
    fixture.token.mint(&user3, &(500_000 * SCALAR_7));

    // Record total tokens across all addresses
    let total_before = fixture.token.balance(&user1)
        + fixture.token.balance(&user2)
        + fixture.token.balance(&user3)
        + fixture.token.balance(&fixture.vault.address)
        + fixture.token.balance(&fixture.treasury.address)
        + fixture.token.balance(&fixture.trading.address);

    // Open several positions
    let pos1 = fixture.open_and_fill(
        &user1,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        20_000 * SCALAR_7,
        true,
        BTC_100K,
        0,
        0,
    );

    let pos2 = fixture.open_and_fill(
        &user2,
        AssetIndex::BTC as u32,
        5_000 * SCALAR_7,
        15_000 * SCALAR_7,
        false,
        BTC_100K,
        0,
        0,
    );

    let pos3 = fixture.open_and_fill(
        &user3,
        AssetIndex::BTC as u32,
        3_000 * SCALAR_7,
        10_000 * SCALAR_7,
        true,
        BTC_100K,
        0,
        0,
    );

    // Let time pass, apply funding
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();
    fixture.jump(SECONDS_IN_HOUR);

    // Close all positions at original price (no PnL from price movement)
    let close_bytes = fixture.btc_price(BTC_100K);
    fixture.trading.close_position(&pos1, &close_bytes);
    fixture.trading.close_position(&pos2, &close_bytes);
    fixture.trading.close_position(&pos3, &close_bytes);

    // Record total tokens after
    let total_after = fixture.token.balance(&user1)
        + fixture.token.balance(&user2)
        + fixture.token.balance(&user3)
        + fixture.token.balance(&fixture.vault.address)
        + fixture.token.balance(&fixture.treasury.address)
        + fixture.token.balance(&fixture.trading.address);

    // Total tokens must be conserved (2 units tolerance per position = 6 total)
    let diff = (total_after - total_before).abs();
    assert!(
        diff <= 6,
        "Token conservation violated: total_before={}, total_after={}, diff={}",
        total_before,
        total_after,
        diff
    );
}
