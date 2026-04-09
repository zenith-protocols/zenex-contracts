use soroban_fixed_point_math::FixedPoint;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::TestFixture;
use test_suites::pyth_helper;
use test_suites::constants::{BTC_PRICE_I64, SCALAR_7, SCALAR_18, SECONDS_IN_HOUR};
use trading::testutils::{default_config, default_market, FEED_BTC, FEED_ETH, FEED_XLM, PRICE_SCALAR};

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data()
}

// ==========================================
// 1. Funding accrues and settles on close
// ==========================================

#[test]
fn test_funding_accrues_and_settles() {
    let fixture = setup_fixture();
    let user_long = Address::generate(&fixture.env);
    let user_short = Address::generate(&fixture.env);

    fixture.token.mint(&user_long, &(500_000 * SCALAR_7));
    fixture.token.mint(&user_short, &(500_000 * SCALAR_7));

    // Open unequal sides: 2x long vs 1x short (long is dominant, funding flows L->S)
    let long_pos = fixture.open_long(&user_long, FEED_BTC, 5_000, 20_000, BTC_PRICE_I64);
    let short_pos = fixture.open_short(&user_short, FEED_BTC, 5_000, 10_000, BTC_PRICE_I64);

    // Let 1 hour pass, then apply_funding to set the funding rate
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // Let another hour pass so funding accrues
    fixture.jump(SECONDS_IN_HOUR);

    // Close both positions at the same price (no PnL from price movement)
    let close_bytes = fixture.btc_price(BTC_PRICE_I64);
    let payout_long = fixture.trading.close_position(&long_pos, &close_bytes);
    let payout_short = fixture.trading.close_position(&short_pos, &close_bytes);

    assert!(payout_long > 0, "long should have some payout");
    assert!(payout_short > 0, "short should have some payout");
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
    let long_pos = fixture.open_long(&user_long, FEED_BTC, 5_000, 20_000, BTC_PRICE_I64);

    let short_pos = fixture.open_short(&user_short, FEED_BTC, 5_000, 10_000, BTC_PRICE_I64);

    // Apply funding + accrue for 1 hour
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();
    fixture.jump(SECONDS_IN_HOUR);

    // Close both at same price (no PnL from price movement)
    let close_bytes = fixture.btc_price(BTC_PRICE_I64);
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
    let _long_pos = fixture.open_long(&user1, FEED_BTC, 5_000, 10_000, BTC_PRICE_I64);

    let _short_pos = fixture.open_short(&user2, FEED_BTC, 5_000, 10_000, BTC_PRICE_I64);

    // Apply funding to compute the rate
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // When L == S, funding rate should be 0
    let market_data = fixture.trading.get_market_data(&(FEED_BTC));
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
    // Additive formula: rate = r_base + r_var × UtilVault^5 + r_var_market × UtilMarket^3
    //
    // In a single-market scenario, total_notional == market_notional.
    // UtilVault = notional / (max_util_global × vault / SCALAR_7)
    // UtilMarket = notional / (max_util_market × vault / SCALAR_7)
    //
    // Strategy: Open positions to push notional, then jump 1 hour.
    // The borrowing index delta over 1 hour equals the hourly rate.

    let config = default_config();
    let r_base = config.r_base;
    let r_var = config.r_var; // SCALAR_18
    let e_standalone = soroban_sdk::Env::default();
    let market_config = default_market(&e_standalone);
    let r_var_market = market_config.r_var_market; // SCALAR_18
    let max_util_global = config.max_util;       // 10 * SCALAR_7
    let max_util_market = market_config.max_util; // 5 * SCALAR_7

    // Vault has 10_000_000 * SCALAR_7 (10M tokens).
    let vault_balance: i128 = 10_000_000 * SCALAR_7;
    let cap_vault = vault_balance * max_util_global / SCALAR_7;    // max global capacity
    let cap_market = vault_balance * max_util_market / SCALAR_7;   // max market capacity

    // Test at 4 notional levels as fractions of cap_market (the smaller cap):
    //   25%, 50%, 75%, 90% of cap_market
    // Each gives different util_vault and util_market because caps differ.
    let pct_points: [(i128, &str); 4] = [
        (2_500_000, "25%"),
        (5_000_000, "50%"),
        (7_500_000, "75%"),
        (9_000_000, "90%"),
    ];

    for (pct_s7, label) in pct_points.iter() {
        let fixture = setup_fixture();
        let user = Address::generate(&fixture.env);
        fixture.token.mint(&user, &(100_000_000 * SCALAR_7));

        // Bump max_notional for rate-curve test that needs large positions at high utilization
        let mut trading_config = fixture.trading.get_config();
        trading_config.max_notional = 100_000_000 * SCALAR_7;
        fixture.trading.set_config(&trading_config);

        // Target notional = pct of cap_market
        let target_notional = (*pct_s7 as i128)
            .fixed_mul_floor(cap_market, SCALAR_7)
            .unwrap();

        let collateral = target_notional / 2;
        let _pos = fixture.open_long(&user, FEED_BTC, collateral / SCALAR_7, target_notional / SCALAR_7, BTC_PRICE_I64);

        let market_before = fixture.trading.get_market_data(&(FEED_BTC));
        let borr_idx_before = market_before.l_borr_idx;

        fixture.jump(SECONDS_IN_HOUR);
        fixture.trading.apply_funding();

        let market_after = fixture.trading.get_market_data(&(FEED_BTC));
        let observed_borr_delta = market_after.l_borr_idx - borr_idx_before;

        // Compute expected rate off-chain (additive formula):
        // util_vault = notional / cap_vault (SCALAR_7)
        let util_vault = target_notional
            .fixed_div_ceil(cap_vault, SCALAR_7)
            .unwrap()
            .min(SCALAR_7);
        // util_market = notional / cap_market = pct_s7 (SCALAR_7)
        let util_market = target_notional
            .fixed_div_ceil(cap_market, SCALAR_7)
            .unwrap()
            .min(SCALAR_7);

        // Vault: util_vault^5
        let uv2 = util_vault.fixed_mul_ceil(util_vault, SCALAR_7).unwrap();
        let uv4 = uv2.fixed_mul_ceil(uv2, SCALAR_7).unwrap();
        let uv5 = uv4.fixed_mul_ceil(util_vault, SCALAR_7).unwrap();

        // Market: util_market^3
        let um2 = util_market.fixed_mul_ceil(util_market, SCALAR_7).unwrap();
        let um3 = um2.fixed_mul_ceil(util_market, SCALAR_7).unwrap();

        let vault_term = r_var.fixed_mul_ceil(uv5, SCALAR_7).unwrap();
        let market_term = r_var_market.fixed_mul_ceil(um3, SCALAR_7).unwrap();
        let expected_borr_rate = r_base + vault_term + market_term;

        // Allow 5% tolerance (vault_balance shifts slightly due to open fees)
        let tolerance_pct = 5;
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
// 6. Fee accrual increases with time
// ==========================================

#[test]
fn test_fee_accrual_increases_with_time() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(500_000 * SCALAR_7));

    // Open a position
    let pos = fixture.open_long(&user, FEED_BTC, 5_000, 20_000, BTC_PRICE_I64);

    // Jump 1 hour and apply funding to set the rate
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // Record borrowing index after 1 hour
    let market_1h = fixture.trading.get_market_data(&(FEED_BTC));
    let borr_idx_1h = market_1h.l_borr_idx;

    // Jump another hour
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // Record borrowing index after 2 hours
    let market_2h = fixture.trading.get_market_data(&(FEED_BTC));
    let borr_idx_2h = market_2h.l_borr_idx;

    // Borrowing index should be larger after 2 hours than after 1 hour
    assert!(
        borr_idx_2h > borr_idx_1h,
        "borrowing index should increase with time: 1h={}, 2h={}",
        borr_idx_1h,
        borr_idx_2h
    );

    // Close the position after 2 hours
    let close_bytes = fixture.btc_price(BTC_PRICE_I64);
    let payout = fixture.trading.close_position(&pos, &close_bytes);
    assert!(payout > 0, "position should have some payout");

    // The payout should be less than the collateral deposited (fees accumulated)
    // Collateral was 5000, but fees were deducted on open so effective is less.
    // Borrowing and funding fees further reduce the payout.
    let final_balance = fixture.token.balance(&user);
    let total_cost = 500_000 * SCALAR_7 - final_balance;
    assert!(total_cost > 0, "user should have paid net fees");
}

fn price_update_all(fixture: &TestFixture, btc: i64, eth: i64, xlm: i64) -> soroban_sdk::Bytes {
    let ts = fixture.env.ledger().timestamp();
    pyth_helper::build_price_update(
        &fixture.env,
        &fixture.signing_key,
        &[
            pyth_helper::FeedInput { feed_id: 1, price: btc, exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: 2, price: eth, exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: 3, price: xlm, exponent: -8, confidence: 0 },
        ],
        ts,
    )
}

// ==========================================
// 7. Funding rounding dust bounded
// ==========================================

/// 100 hours of funding with Alice 100k long (dominant), Bob 50k short.
/// The rounding residual (alice_paid - bob_received) should be < 10k units.
#[test]
fn test_funding_rounding_dust_bounded() {
    let fixture = create_fixture_with_data();

    let mut config = fixture.trading.get_config();
    config.fee_dom = 0; config.fee_non_dom = 0;
    config.r_base = 0; config.r_var = 0;
    fixture.trading.set_config(&config);

    let mut mc = default_market(&fixture.env);
    mc.r_var_market = 0; mc.impact = i128::MAX;
    fixture.create_market(FEED_BTC, &mc);

    let alice = Address::generate(&fixture.env);
    let bob = Address::generate(&fixture.env);
    fixture.token.mint(&alice, &(200_000 * SCALAR_7));
    fixture.token.mint(&bob, &(200_000 * SCALAR_7));

    let alice_pos = fixture.open_long(&alice, FEED_BTC, 20_000, 100_000, BTC_PRICE_I64);
    let bob_pos = fixture.open_short(&bob, FEED_BTC, 10_000, 50_000, BTC_PRICE_I64);

    let vault_before = fixture.vault.total_assets();

    for _ in 0..100 {
        fixture.jump(SECONDS_IN_HOUR);
        fixture.trading.apply_funding();
    }

    let close_bytes = fixture.btc_price(BTC_PRICE_I64);
    let payout_alice = fixture.trading.close_position(&alice_pos, &close_bytes);
    let payout_bob = fixture.trading.close_position(&bob_pos, &close_bytes);

    let vault_after = fixture.vault.total_assets();
    let alice_loss = 20_000 * SCALAR_7 - payout_alice;
    let bob_gain = payout_bob - 10_000 * SCALAR_7;
    let residual = alice_loss - bob_gain;
    let vault_change = vault_after - vault_before;

    assert_eq!(vault_change, residual);
    assert!(residual.abs() < 10_000, "rounding residual {} exceeds bound", residual);
}

// ==========================================
// 8. Borrowing dominance flip accrual timing
// ==========================================

/// Opens longs-dominant (200k vs 100k), jumps 30 min, then Carol opens 250k short
/// flipping dominance. Verifies accrual uses pre-trade dominance (longs charged,
/// not shorts). This is correct behavior, not a bug.
#[test]
fn test_borrowing_dominance_flip_accrual_timing() {
    let fixture = create_fixture_with_data();

    let mut config = fixture.trading.get_config();
    config.r_funding = 0; config.fee_dom = 0; config.fee_non_dom = 0;
    fixture.trading.set_config(&config);

    let mut mc = default_market(&fixture.env);
    mc.impact = i128::MAX;
    fixture.create_market(FEED_BTC, &mc);

    let alice = Address::generate(&fixture.env);
    let bob = Address::generate(&fixture.env);
    let carol = Address::generate(&fixture.env);
    fixture.token.mint(&alice, &(500_000 * SCALAR_7));
    fixture.token.mint(&bob, &(500_000 * SCALAR_7));
    fixture.token.mint(&carol, &(500_000 * SCALAR_7));

    fixture.open_long(&alice, FEED_BTC, 50_000, 200_000, BTC_PRICE_I64);
    fixture.open_short(&bob, FEED_BTC, 50_000, 100_000, BTC_PRICE_I64);

    let data_after_open = fixture.trading.get_market_data(&FEED_BTC);
    let l_borr_after_open = data_after_open.l_borr_idx;
    let s_borr_after_open = data_after_open.s_borr_idx;

    fixture.jump(1800); // 30 min, no accrual yet

    // Carol's open triggers Context::load → accrue with PRE-TRADE dominance
    fixture.open_short(&carol, FEED_BTC, 100_000, 250_000, BTC_PRICE_I64);

    let data_after_carol = fixture.trading.get_market_data(&FEED_BTC);

    // Longs were dominant at accrual → l_borr_idx advanced
    assert!(data_after_carol.l_borr_idx > l_borr_after_open);
    // Shorts were non-dominant → s_borr_idx unchanged
    assert_eq!(data_after_carol.s_borr_idx, s_borr_after_open);

    // After Carol: shorts now dominant (350k > 200k)
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    let data_after_flip = fixture.trading.get_market_data(&FEED_BTC);

    // Now shorts dominant → s_borr_idx advances, l_borr_idx stays
    assert!(data_after_flip.s_borr_idx > data_after_carol.s_borr_idx);
    assert_eq!(data_after_flip.l_borr_idx, data_after_carol.l_borr_idx);
}

// ==========================================
// 9. ADL funding undercharge bounded
// ==========================================

/// After ADL reduces notional, the simplified funding formula under-charges
/// relative to the correct two-phase calculation. This is a known design
/// tradeoff. The gap should be < 0.1% of vault TVL.
#[test]
fn test_adl_funding_undercharge_bounded() {
    let fixture = TestFixture::create();
    fixture.token.mint(&fixture.owner, &(500_000 * SCALAR_7));
    fixture.vault.deposit(&(500_000 * SCALAR_7), &fixture.owner, &fixture.owner, &fixture.owner);

    let mut config = fixture.trading.get_config();
    config.fee_dom = 0; config.fee_non_dom = 0;
    config.r_base = 0; config.r_var = 0;
    config.r_funding = 100_000_000_000_000; // max rate for measurable effect
    fixture.trading.set_config(&config);

    let mut mc = default_market(&fixture.env);
    mc.r_var_market = 0; mc.impact = i128::MAX;
    fixture.create_market(FEED_BTC, &mc);
    mc.feed_id = FEED_ETH;
    fixture.create_market(FEED_ETH, &mc);
    mc.feed_id = FEED_XLM;
    fixture.create_market(FEED_XLM, &mc);

    let alice = Address::generate(&fixture.env);
    let bob = Address::generate(&fixture.env);
    fixture.token.mint(&alice, &(200_000 * SCALAR_7));
    fixture.token.mint(&bob, &(200_000 * SCALAR_7));

    let alice_pos = fixture.open_long(&alice, FEED_BTC, 50_000, 200_000, BTC_PRICE_I64);
    fixture.open_short(&bob, FEED_BTC, 50_000, 100_000, BTC_PRICE_I64);

    let alice_data = fixture.trading.get_position(&alice_pos);
    let original_notional = alice_data.notional;
    let alice_fund_idx_at_open = alice_data.fund_idx;

    // Pre-ADL: 2 hours of funding
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    let fund_idx_pre_adl = fixture.trading.get_market_data(&FEED_BTC).l_fund_idx;

    // ADL at BTC $700k
    fixture.trading.update_status(&price_update_all(
        &fixture, 700_000 * PRICE_SCALAR as i64, 200_000_000_000, 10_000_000,
    ));
    let adl_idx = fixture.trading.get_market_data(&FEED_BTC).l_adl_idx;
    assert!(adl_idx < SCALAR_18);

    // Post-ADL: 2 more hours of funding
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    let fund_idx_final = fixture.trading.get_market_data(&FEED_BTC).l_fund_idx;

    fixture.jump(31);
    let payout_alice = fixture.trading.close_position(&alice_pos, &fixture.btc_price(BTC_PRICE_I64));

    let alice_col = 50_000 * SCALAR_7;
    let actual_funding = alice_col - payout_alice;

    let pre_adl_delta = fund_idx_pre_adl - alice_fund_idx_at_open;
    let post_adl_delta = fund_idx_final - fund_idx_pre_adl;
    let reduced_notional = original_notional * adl_idx / SCALAR_18;

    let correct_funding = original_notional * pre_adl_delta / SCALAR_18
        + reduced_notional * post_adl_delta / SCALAR_18;

    let gap = correct_funding - actual_funding;
    assert!(gap >= 0, "gap should be non-negative: {}", gap);
    assert!(gap < 500_000 * SCALAR_7 / 1_000, "gap {} exceeds 0.1% of vault", gap);
}