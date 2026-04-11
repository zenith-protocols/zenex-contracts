use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::TestFixture;
use test_suites::constants::{BTC_PRICE_I64, SCALAR_7, SECONDS_PER_WEEK};
use trading::testutils::{default_market, FEED_BTC, PRICE_SCALAR};

// ==========================================
// Helper Functions
// ==========================================

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data()
}

/// Create a fixture with time-dependent rates zeroed out.
/// Open/close trading fees remain (fee_dom, fee_non_dom, impact).
/// This isolates PnL + fee-split testing from borrowing/funding math
/// (covered separately in test_fee_accrual.rs).
fn setup_zero_rate_fixture() -> TestFixture<'static> {
    let fixture = TestFixture::create();

    fixture.token.mint(&fixture.owner, &(10_000_000 * SCALAR_7));
    fixture.vault.deposit(
        &(10_000_000 * SCALAR_7),
        &fixture.owner,
        &fixture.owner,
        &fixture.owner,
    );

    let mut config = fixture.trading.get_config();
    config.r_base = 0;
    config.r_var = 0;
    config.r_funding = 0;
    fixture.trading.set_config(&config);

    let mut mc = default_market(&fixture.env);
    mc.r_var_market = 0;
    fixture.create_market(FEED_BTC, &mc);

    fixture
}

fn open_long(fixture: &TestFixture, user: &Address) -> u32 {
    fixture.open_long(user, FEED_BTC, 1_000, 10_000, BTC_PRICE_I64)
}

fn open_short(fixture: &TestFixture, user: &Address) -> u32 {
    fixture.open_short(user, FEED_BTC, 1_000, 10_000, BTC_PRICE_I64)
}

fn place_limit_long(fixture: &TestFixture, user: &Address, entry_price: i128) -> u32 {
    fixture.trading.place_limit(
        user,
        &(FEED_BTC),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &entry_price,
        &0,
        &0,
    )
}

// ==========================================
// 1. Fund Flow — 4 canonical tests
//
// All rates zeroed (r_base, r_var, r_var_market, r_funding = 0).
// Only trading fees apply: fee_dom, fee_non_dom, impact.
// Each test traces every token through user → contract → vault/treasury
// with inline derivation matching the contract's fixed-point formulas.
//
// Common parameters:
//   collateral   = 1_000 × S7 = 10_000_000_000
//   notional     = 10_000 × S7 = 100_000_000_000
//   entry_price  = $100k = 10_000_000_000_000
//   price_scalar = 10^8 = 100_000_000
//   fee_dom      = 5_000   (0.05%)
//   fee_non_dom  = 1_000   (0.01%)
//   impact       = 8B × S7 = 80_000_000_000_000_000
//   treasury_rate = 500_000 (5%)
//
// Open fees (first position → dominant side → fee_dom):
//   base_fee   = ceil(100B × 5_000 / S7)  = 50_000_000
//   impact_fee = floor(100B × S7 / 80Q)   = 12
//   total_open = 50_000_012
//   post_fee_col = 10B - 50_000_012       = 9_949_999_988
//
// Open fee split:
//   treasury = floor(50_000_012 × 500_000 / S7) = 2_500_000
//   vault    = 50_000_012 - 2_500_000            = 47_500_012
//
// Close fees (only position, closing → dom fee, no borrowing/funding):
//   base_fee   = 50_000_000
//   impact_fee = 12
//   total_close = 50_000_012
//
// Close fee split:
//   protocol_fee = base + impact + borrowing(0) = 50_000_012
//   treasury     = floor(50_000_012 × 500_000 / S7) = 2_500_000
// ==========================================

#[test]
fn test_long_profit() {
    let fixture = setup_zero_rate_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let user_0 = fixture.token.balance(&user);
    let vault_0 = fixture.vault.total_assets();
    let treasury_0 = fixture.token.balance(&fixture.treasury.address);

    // ── Open long at $100k ──
    let pos_id = open_long(&fixture, &user);

    let user_1 = fixture.token.balance(&user);
    let vault_1 = fixture.vault.total_assets();
    let treasury_1 = fixture.token.balance(&fixture.treasury.address);

    // User pays collateral = 10_000_000_000
    assert_eq!(user_0 - user_1, 10_000_000_000);
    // Vault gets open_total - treasury = 50_000_012 - 2_500_000 = 47_500_012
    assert_eq!(vault_1 - vault_0, 47_500_012);
    // Treasury gets floor(50_000_012 × 500_000 / S7) = 2_500_000
    assert_eq!(treasury_1 - treasury_0, 2_500_000);

    let pos = fixture.trading.get_position(&user, &pos_id);
    // col = 10B - open_fees(50_000_012) = 9_949_999_988
    assert_eq!(pos.col, 9_949_999_988);
    assert_eq!(pos.notional, 100_000_000_000);

    // ── Close at $110k (+10%) ──
    fixture.jump(31);
    let close_price = fixture.btc_price(110_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&user, &pos_id, &close_price);

    let user_2 = fixture.token.balance(&user);
    let vault_2 = fixture.vault.total_assets();
    let treasury_2 = fixture.token.balance(&fixture.treasury.address);

    // PnL: ratio = floor(($110k - $100k) × 10^8 / $100k) = 10_000_000
    //      pnl   = floor(100B × 10_000_000 / 10^8) = 10_000_000_000
    //
    // equity = col + pnl - close_fees
    //        = 9_949_999_988 + 10_000_000_000 - 50_000_012 = 19_899_999_976
    assert_eq!(payout, 19_899_999_976);
    assert_eq!(user_2 - user_1, payout);

    // Close treasury = floor(50_000_012 × 500_000 / S7) = 2_500_000
    assert_eq!(treasury_2 - treasury_1, 2_500_000);

    // Vault transfer = col - user_payout - treasury
    //                = 9_949_999_988 - 19_899_999_976 - 2_500_000 = -9_952_499_988
    // Negative → vault pays out (user profited)
    let vault_close_delta = vault_2 as i128 - vault_1 as i128;
    assert_eq!(vault_close_delta, 9_949_999_988 - payout - 2_500_000);

    // ── Conservation: user + vault + treasury changes sum to zero ──
    let user_delta = user_2 as i128 - user_0 as i128;
    let vault_delta = vault_2 as i128 - vault_0 as i128;
    let treasury_delta = treasury_2 as i128 - treasury_0 as i128;
    assert_eq!(user_delta + vault_delta + treasury_delta, 0);
}

#[test]
fn test_long_loss() {
    let fixture = setup_zero_rate_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let user_0 = fixture.token.balance(&user);
    let vault_0 = fixture.vault.total_assets();
    let treasury_0 = fixture.token.balance(&fixture.treasury.address);

    // ── Open long at $100k ──
    let pos_id = open_long(&fixture, &user);

    let user_1 = fixture.token.balance(&user);
    let vault_1 = fixture.vault.total_assets();
    let treasury_1 = fixture.token.balance(&fixture.treasury.address);

    assert_eq!(user_0 - user_1, 10_000_000_000);
    assert_eq!(vault_1 - vault_0, 47_500_012);
    assert_eq!(treasury_1 - treasury_0, 2_500_000);

    // ── Close at $95k (-5%) ──
    fixture.jump(31);
    let close_price = fixture.btc_price(95_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&user, &pos_id, &close_price);

    let user_2 = fixture.token.balance(&user);
    let vault_2 = fixture.vault.total_assets();
    let treasury_2 = fixture.token.balance(&fixture.treasury.address);

    // PnL: ratio = floor(($95k - $100k) × 10^8 / $100k) = -5_000_000
    //      pnl   = floor(100B × -5_000_000 / 10^8) = -5_000_000_000
    //
    // equity = 9_949_999_988 + (-5_000_000_000) - 50_000_012 = 4_899_999_976
    assert_eq!(payout, 4_899_999_976);
    assert_eq!(user_2 - user_1, payout);
    assert_eq!(treasury_2 - treasury_1, 2_500_000);

    // Vault gains: col - user - treasury = 9_949_999_988 - 4_899_999_976 - 2_500_000
    //            = 5_047_500_012 (positive → vault absorbs the loss)
    let vault_close_delta = vault_2 as i128 - vault_1 as i128;
    assert_eq!(vault_close_delta, 9_949_999_988 - payout - 2_500_000);
    assert!(vault_close_delta > 0);

    // ── Conservation ──
    let user_delta = user_2 as i128 - user_0 as i128;
    let vault_delta = vault_2 as i128 - vault_0 as i128;
    let treasury_delta = treasury_2 as i128 - treasury_0 as i128;
    assert_eq!(user_delta + vault_delta + treasury_delta, 0);
}

#[test]
fn test_short_profit() {
    let fixture = setup_zero_rate_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let user_0 = fixture.token.balance(&user);
    let vault_0 = fixture.vault.total_assets();
    let treasury_0 = fixture.token.balance(&fixture.treasury.address);

    // ── Open short at $100k ──
    let pos_id = open_short(&fixture, &user);

    let user_1 = fixture.token.balance(&user);
    let vault_1 = fixture.vault.total_assets();
    let treasury_1 = fixture.token.balance(&fixture.treasury.address);

    assert_eq!(user_0 - user_1, 10_000_000_000);
    assert_eq!(vault_1 - vault_0, 47_500_012);
    assert_eq!(treasury_1 - treasury_0, 2_500_000);

    let pos = fixture.trading.get_position(&user, &pos_id);
    assert!(!pos.long);
    assert_eq!(pos.col, 9_949_999_988);

    // ── Close at $90k (short profits from price drop) ──
    fixture.jump(31);
    let close_price = fixture.btc_price(90_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&user, &pos_id, &close_price);

    let user_2 = fixture.token.balance(&user);
    let vault_2 = fixture.vault.total_assets();
    let treasury_2 = fixture.token.balance(&fixture.treasury.address);

    // PnL (short): ratio = floor(($100k - $90k) × 10^8 / $100k) = 10_000_000
    //              pnl   = floor(100B × 10_000_000 / 10^8) = +10_000_000_000
    //
    // equity = 9_949_999_988 + 10_000_000_000 - 50_000_012 = 19_899_999_976
    assert_eq!(payout, 19_899_999_976);
    assert_eq!(user_2 - user_1, payout);
    assert_eq!(treasury_2 - treasury_1, 2_500_000);

    // ── Conservation ──
    let user_delta = user_2 as i128 - user_0 as i128;
    let vault_delta = vault_2 as i128 - vault_0 as i128;
    let treasury_delta = treasury_2 as i128 - treasury_0 as i128;
    assert_eq!(user_delta + vault_delta + treasury_delta, 0);
}

#[test]
fn test_short_loss() {
    let fixture = setup_zero_rate_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let user_0 = fixture.token.balance(&user);
    let vault_0 = fixture.vault.total_assets();
    let treasury_0 = fixture.token.balance(&fixture.treasury.address);

    // ── Open short at $100k ──
    let pos_id = open_short(&fixture, &user);

    let user_1 = fixture.token.balance(&user);
    let vault_1 = fixture.vault.total_assets();
    let treasury_1 = fixture.token.balance(&fixture.treasury.address);

    assert_eq!(user_0 - user_1, 10_000_000_000);
    assert_eq!(vault_1 - vault_0, 47_500_012);
    assert_eq!(treasury_1 - treasury_0, 2_500_000);

    // ── Close at $105k (short loses from price rise) ──
    fixture.jump(31);
    let close_price = fixture.btc_price(105_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&user, &pos_id, &close_price);

    let user_2 = fixture.token.balance(&user);
    let vault_2 = fixture.vault.total_assets();
    let treasury_2 = fixture.token.balance(&fixture.treasury.address);

    // PnL (short): ratio = floor(($100k - $105k) × 10^8 / $100k) = -5_000_000
    //              pnl   = floor(100B × -5_000_000 / 10^8) = -5_000_000_000
    //
    // equity = 9_949_999_988 + (-5_000_000_000) - 50_000_012 = 4_899_999_976
    assert_eq!(payout, 4_899_999_976);
    assert_eq!(user_2 - user_1, payout);
    assert_eq!(treasury_2 - treasury_1, 2_500_000);

    // Vault gains: positive (absorbs loss)
    let vault_close_delta = vault_2 as i128 - vault_1 as i128;
    assert_eq!(vault_close_delta, 9_949_999_988 - payout - 2_500_000);
    assert!(vault_close_delta > 0);

    // ── Conservation ──
    let user_delta = user_2 as i128 - user_0 as i128;
    let vault_delta = vault_2 as i128 - vault_0 as i128;
    let treasury_delta = treasury_2 as i128 - treasury_0 as i128;
    assert_eq!(user_delta + vault_delta + treasury_delta, 0);
}

// ==========================================
// 2. Keeper Triggers (4 tests)
//
// Uses default fixture (rates enabled) — trigger tests care about
// control flow (does TP/SL fire?), not exact PnL arithmetic.
// Keeper fee is deterministic: floor(trading_fee × caller_rate / S7)
//   = floor((50_000_000 + 12) × 1_000_000 / S7) = 5_000_001
// ==========================================

#[test]
fn test_long_take_profit_trigger() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long(&fixture, &user);

    fixture
        .trading
        .set_triggers(&user, &position_id, &(110_000 * PRICE_SCALAR), &(95_000 * PRICE_SCALAR));

    fixture.jump(31);
    let tp_price = fixture.btc_price(111_000 * PRICE_SCALAR as i64);

    let keeper_before = fixture.token.balance(&keeper);
    let user_before = fixture.token.balance(&user);
    let users = svec![&fixture.env, user.clone()];
    let seqs = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &tp_price);

    assert!(!fixture.position_exists(&user, position_id));
    assert!(fixture.token.balance(&user) > user_before);
    assert_eq!(fixture.token.balance(&keeper) - keeper_before, 5_000_001);
}

#[test]
fn test_long_stop_loss_trigger() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long(&fixture, &user);

    fixture
        .trading
        .set_triggers(&user, &position_id, &(110_000 * PRICE_SCALAR), &(95_000 * PRICE_SCALAR));

    fixture.jump(31);
    let sl_price = fixture.btc_price(94_000 * PRICE_SCALAR as i64);

    let keeper_before = fixture.token.balance(&keeper);
    let user_before = fixture.token.balance(&user);
    let users = svec![&fixture.env, user.clone()];
    let seqs = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &sl_price);

    assert!(!fixture.position_exists(&user, position_id));
    assert!(fixture.token.balance(&user) > user_before);
    assert_eq!(fixture.token.balance(&keeper) - keeper_before, 5_000_001);
}

#[test]
fn test_short_take_profit_trigger() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_short(&fixture, &user);

    fixture
        .trading
        .set_triggers(&user, &position_id, &(90_000 * PRICE_SCALAR), &0);

    fixture.jump(31);
    let tp_price = fixture.btc_price(89_000 * PRICE_SCALAR as i64);

    let keeper_before = fixture.token.balance(&keeper);
    let users = svec![&fixture.env, user.clone()];
    let seqs = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &tp_price);

    assert!(!fixture.position_exists(&user, position_id));
    assert_eq!(fixture.token.balance(&keeper) - keeper_before, 5_000_001);
}

#[test]
fn test_short_stop_loss_trigger() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_short(&fixture, &user);

    fixture
        .trading
        .set_triggers(&user, &position_id, &0, &(105_000 * PRICE_SCALAR));

    fixture.jump(31);
    let sl_price = fixture.btc_price(106_000 * PRICE_SCALAR as i64);

    let keeper_before = fixture.token.balance(&keeper);
    let users = svec![&fixture.env, user.clone()];
    let seqs = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &sl_price);

    assert!(!fixture.position_exists(&user, position_id));
    assert_eq!(fixture.token.balance(&keeper) - keeper_before, 5_000_001);
}

// ==========================================
// 3. Limit Orders (3 tests)
// ==========================================

#[test]
fn test_limit_order_place_fill_close() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = 101_000 * PRICE_SCALAR;
    let position_id = place_limit_long(&fixture, &user, entry_price);

    // Pending: collateral locked, no fees yet
    let pos = fixture.trading.get_position(&user, &position_id);
    assert!(!pos.filled);
    assert_eq!(pos.col, 1_000 * SCALAR_7);
    assert_eq!(fixture.trading.get_market_data(&FEED_BTC).l_notional, 0);

    // Fill at $101k
    let fill_price = fixture.btc_price(101_000 * PRICE_SCALAR as i64);
    let keeper_before = fixture.token.balance(&keeper);
    let users = svec![&fixture.env, user.clone()];
    let seqs = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &fill_price);

    let pos_filled = fixture.trading.get_position(&user, &position_id);
    assert!(pos_filled.filled);
    assert_eq!(pos_filled.entry_price, 101_000 * PRICE_SCALAR);
    // col = 10B - open_fees(50_000_012) = 9_949_999_988
    assert_eq!(pos_filled.col, 9_949_999_988);
    // Keeper fill fee = floor(50_000_012 × 1_000_000 / S7) = 5_000_001
    assert_eq!(fixture.token.balance(&keeper) - keeper_before, 5_000_001);

    // Close at $110k for profit
    fixture.jump(31);
    let close_price = fixture.btc_price(110_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&user, &position_id, &close_price);
    assert!(payout > 1_000 * SCALAR_7);
    assert!(!fixture.position_exists(&user, position_id));
}

#[test]
fn test_limit_order_cancel_refund() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    let position_id = place_limit_long(&fixture, &user, 101_000 * PRICE_SCALAR);
    assert_eq!(fixture.token.balance(&user), initial_balance - 1_000 * SCALAR_7);

    // Cancel: full refund (no fees on unfilled limits)
    fixture.trading.cancel_position(&user, &position_id);
    assert_eq!(fixture.token.balance(&user), initial_balance);
    assert!(!fixture.position_exists(&user, position_id));
}

#[test]
#[should_panic(expected = "Error(Contract, #731)")]
fn test_limit_order_not_fillable_at_price() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = place_limit_long(&fixture, &user, 101_000 * PRICE_SCALAR);

    // Price $105k > entry $101k — NOT fillable for long limit
    let bad_price = fixture.btc_price(105_000 * PRICE_SCALAR as i64);
    let users = svec![&fixture.env, user.clone()];
    let seqs = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &bad_price);
}

// ==========================================
// 4. Contract Status Edge Cases (3 tests)
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #741)")]
fn test_open_blocked_when_frozen() {
    let fixture = setup_fixture();
    fixture.trading.set_status(&3u32);

    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    fixture.trading.place_limit(
        &user,
        &(FEED_BTC),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &(100_000 * PRICE_SCALAR),
        &0,
        &0,
    );
}

#[test]
fn test_close_allowed_when_on_ice() {
    let fixture = setup_zero_rate_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long(&fixture, &user);
    fixture.trading.set_status(&2u32);

    // Close at same price: pnl=0, only trading fees
    // equity = col - close_fees = 9_949_999_988 - 50_000_012 = 9_899_999_976
    fixture.jump(31);
    let close_price = fixture.btc_price(BTC_PRICE_I64);
    let payout = fixture.trading.close_position(&user, &position_id, &close_price);

    assert_eq!(payout, 9_899_999_976);
    assert!(!fixture.position_exists(&user, position_id));
}

#[test]
fn test_execute_keeper_triggers_when_on_ice() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long(&fixture, &user);
    fixture
        .trading
        .set_triggers(&user, &position_id, &(110_000 * PRICE_SCALAR), &0);

    fixture.trading.set_status(&2u32);

    fixture.jump(31);
    let tp_price = fixture.btc_price(111_000 * PRICE_SCALAR as i64);

    let users = svec![&fixture.env, user.clone()];
    let seqs = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &tp_price);
    assert!(!fixture.position_exists(&user, position_id));
}

// ==========================================
// 5. PnL Edge Cases (2 tests)
// ==========================================

#[test]
fn test_equal_notional_zero_funding() {
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);
    fixture.token.mint(&user1, &(1_000_000 * SCALAR_7));
    fixture.token.mint(&user2, &(1_000_000 * SCALAR_7));

    fixture.open_long(&user1, FEED_BTC, 50_000, 200_000, BTC_PRICE_I64);
    fixture.open_short(&user2, FEED_BTC, 50_000, 200_000, BTC_PRICE_I64);

    fixture.jump(3600);
    fixture.trading.apply_funding();

    let market = fixture.trading.get_market_data(&(FEED_BTC));
    assert_eq!(market.fund_rate, 0);
}

#[test]
fn test_loss_exceeds_collateral_clamped() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = fixture.open_long(&user, FEED_BTC, 1_000, 20_000, BTC_PRICE_I64);

    // 20x leverage, 10% drop → loss = $2000 > $1000 collateral → payout clamped to 0
    fixture.jump(SECONDS_PER_WEEK);
    let crash_price = fixture.btc_price(90_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&user, &position_id, &crash_price);

    assert_eq!(payout, 0);
    assert!(!fixture.position_exists(&user, position_id));
}

// ==========================================
// 6. Multi-User Isolation (1 test)
// ==========================================

#[test]
fn test_multi_user_position_isolation() {
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);
    fixture.token.mint(&user1, &(100_000 * SCALAR_7));
    fixture.token.mint(&user2, &(100_000 * SCALAR_7));

    let pos1 = open_long(&fixture, &user1);
    let pos2 = open_short(&fixture, &user2);

    assert_eq!(fixture.trading.get_user_counter(&user1), 1);
    assert_eq!(fixture.trading.get_user_counter(&user2), 1);
    assert_eq!(fixture.trading.get_position(&user1, &pos1).user, user1);
    assert_eq!(fixture.trading.get_position(&user2, &pos2).user, user2);

    fixture.jump(31);
    let close_price = fixture.btc_price(110_000 * PRICE_SCALAR as i64);
    fixture.trading.close_position(&user1, &pos1, &close_price);

    assert!(!fixture.position_exists(&user1, pos1));
    assert!(fixture.position_exists(&user2, pos2));
}
