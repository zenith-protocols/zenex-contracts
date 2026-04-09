use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::TestFixture;
use test_suites::constants::{BTC_PRICE_I64, SCALAR_7, SECONDS_PER_WEEK};
use trading::testutils::{FEED_BTC, PRICE_SCALAR};

// ==========================================
// Helper Functions
// ==========================================

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data()
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
// 1. Core Lifecycle (3 tests)
// ==========================================

#[test]
fn test_long_open_and_close_profit() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long at $100k
    let position_id = open_long(&fixture, &user);

    // Verify position filled
    let pos = fixture.trading.get_position(&position_id);
    assert!(pos.filled);
    assert!(pos.long);
    assert!(pos.col > 0);
    assert!(pos.col < 1_000 * SCALAR_7); // reduced by opening fees

    // Jump past MIN_OPEN_TIME, close at higher price ($110k)
    fixture.jump(31);
    let close_price = fixture.btc_price(110_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&position_id, &close_price);

    // Profitable: payout should exceed original collateral minus fees
    // 10% gain on 10k notional = ~1000 profit
    assert!(payout > 1_000 * SCALAR_7, "payout should reflect profit");
    assert!(!fixture.position_exists(position_id));
    assert_eq!(fixture.trading.get_user_positions(&user).len(), 0);

    // User should have profited overall
    let final_balance = fixture.token.balance(&user);
    assert!(final_balance > initial_balance - 100 * SCALAR_7);
}

#[test]
fn test_short_open_and_close_loss() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open short at $100k
    let position_id = open_short(&fixture, &user);
    let pos = fixture.trading.get_position(&position_id);
    assert!(!pos.long);

    // Jump past MIN_OPEN_TIME, close at higher price ($105k) -- loss for short
    fixture.jump(31);
    let close_price = fixture.btc_price(105_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&position_id, &close_price);

    // 5% loss on 10k notional = 500 loss, with ~1000 collateral minus fees
    // Payout should be reduced (loss ate into collateral)
    assert!(payout < 600 * SCALAR_7, "payout should reflect loss");
    assert!(!fixture.position_exists(position_id));

    // User lost tokens overall
    let final_balance = fixture.token.balance(&user);
    assert!(final_balance < initial_balance);
}

#[test]
fn test_modify_collateral_add_and_remove() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long(&fixture, &user);
    let pos_before = fixture.trading.get_position(&position_id);
    let col_before = pos_before.col;

    // Add collateral (set to 2000)
    let price_bytes = fixture.btc_price(BTC_PRICE_I64);
    fixture
        .trading
        .modify_collateral(&position_id, &(2_000 * SCALAR_7), &price_bytes);

    let pos_added = fixture.trading.get_position(&position_id);
    assert_eq!(pos_added.col, 2_000 * SCALAR_7);
    assert!(pos_added.col > col_before);

    // Remove collateral (set to 500)
    let balance_before_remove = fixture.token.balance(&user);
    fixture
        .trading
        .modify_collateral(&position_id, &(500 * SCALAR_7), &price_bytes);

    let pos_removed = fixture.trading.get_position(&position_id);
    assert_eq!(pos_removed.col, 500 * SCALAR_7);

    // User should have received tokens back
    let balance_after_remove = fixture.token.balance(&user);
    assert!(balance_after_remove > balance_before_remove);
}

// ==========================================
// 2. Keeper Triggers (4 tests)
// ==========================================

#[test]
fn test_long_take_profit_trigger() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long(&fixture, &user);

    // Set TP above entry, SL below entry
    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * PRICE_SCALAR), &(95_000 * PRICE_SCALAR));

    // Jump past MIN_OPEN_TIME, price rises past TP
    fixture.jump(31);
    let tp_price = fixture.btc_price(111_000 * PRICE_SCALAR as i64);

    let ids = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &ids, &tp_price);
    assert!(!fixture.position_exists(position_id));
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
        .set_triggers(&position_id, &(110_000 * PRICE_SCALAR), &(95_000 * PRICE_SCALAR));

    // Jump past MIN_OPEN_TIME, price drops past SL
    fixture.jump(31);
    let sl_price = fixture.btc_price(94_000 * PRICE_SCALAR as i64);

    let ids = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &ids, &sl_price);
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_short_take_profit_trigger() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_short(&fixture, &user);

    // Short TP: below entry price
    fixture
        .trading
        .set_triggers(&position_id, &(90_000 * PRICE_SCALAR), &0);

    fixture.jump(31);
    let tp_price = fixture.btc_price(89_000 * PRICE_SCALAR as i64);

    let ids = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &ids, &tp_price);
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_short_stop_loss_trigger() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_short(&fixture, &user);

    // Short SL: above entry price
    fixture
        .trading
        .set_triggers(&position_id, &0, &(105_000 * PRICE_SCALAR));

    fixture.jump(31);
    let sl_price = fixture.btc_price(106_000 * PRICE_SCALAR as i64);

    let ids = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &ids, &sl_price);
    assert!(!fixture.position_exists(position_id));
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

    // Place long limit at $101k (fill when price <= entry)
    let entry_price = 101_000 * PRICE_SCALAR;
    let position_id = place_limit_long(&fixture, &user, entry_price);
    assert!(!fixture.trading.get_position(&position_id).filled);

    // Market data should not yet reflect the pending order
    let market = fixture.trading.get_market_data(&(FEED_BTC));
    assert_eq!(market.l_notional, 0);

    // Price drops to entry -- fillable for long limit
    let fill_price = fixture.btc_price(101_000 * PRICE_SCALAR as i64);
    let ids = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &ids, &fill_price);
    assert!(fixture.trading.get_position(&position_id).filled);

    // Price rises for profit, close
    fixture.jump(31);
    let close_price = fixture.btc_price(110_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&position_id, &close_price);
    assert!(payout > 0);
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_limit_order_cancel_refund() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Place limit order
    let entry_price = 101_000 * PRICE_SCALAR;
    let position_id = place_limit_long(&fixture, &user, entry_price);
    let balance_after_place = fixture.token.balance(&user);
    assert!(balance_after_place < initial_balance); // collateral deducted

    // Cancel limit order
    fixture.trading.cancel_position(&position_id);

    // Full refund
    let final_balance = fixture.token.balance(&user);
    assert_eq!(final_balance, initial_balance);
    assert!(!fixture.position_exists(position_id));
}

#[test]
#[should_panic(expected = "Error(Contract, #731)")]
fn test_limit_order_not_fillable_at_price() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Place long limit at $101k
    let entry_price = 101_000 * PRICE_SCALAR;
    let position_id = place_limit_long(&fixture, &user, entry_price);

    // Price moves up to $105k (above entry) -- NOT fillable for a long limit
    let bad_price = fixture.btc_price(105_000 * PRICE_SCALAR as i64);
    let ids = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &ids, &bad_price);
}

// ==========================================
// 4. Contract Status Edge Cases (3 tests)
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #741)")]
fn test_open_blocked_when_frozen() {
    let fixture = setup_fixture();
    // Set to Frozen (3)
    fixture.trading.set_status(&3u32);

    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Any attempt to open should fail -- place_limit uses require_active
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
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open position while Active
    let position_id = open_long(&fixture, &user);

    // Set to AdminOnIce (2)
    fixture.trading.set_status(&2u32);

    // Close should still work (require_can_manage allows OnIce/AdminOnIce)
    fixture.jump(31);
    let close_price = fixture.btc_price(BTC_PRICE_I64);
    let payout = fixture.trading.close_position(&position_id, &close_price);
    assert!(payout >= 0);
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_execute_keeper_triggers_when_on_ice() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open while Active, set TP
    let position_id = open_long(&fixture, &user);
    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * PRICE_SCALAR), &0);

    // Set to AdminOnIce (2) -- keeper triggers still allowed
    fixture.trading.set_status(&2u32);

    fixture.jump(31);
    let tp_price = fixture.btc_price(111_000 * PRICE_SCALAR as i64);

    let ids = svec![&fixture.env, position_id];
    fixture.trading.execute(&keeper, &FEED_BTC, &ids, &tp_price);
    assert!(!fixture.position_exists(position_id));
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

    // Equal notional: long and short
    let collateral = 50_000;
    let notional = 200_000;

    fixture.open_long(&user1, FEED_BTC, collateral, notional, BTC_PRICE_I64);
    fixture.open_short(&user2, FEED_BTC, collateral, notional, BTC_PRICE_I64);

    // Apply funding -- equal notional should yield zero funding rate
    fixture.jump(3600);
    fixture.trading.apply_funding();

    let market = fixture.trading.get_market_data(&(FEED_BTC));
    assert_eq!(market.fund_rate, 0, "equal notional should yield zero funding rate");
}

#[test]
fn test_loss_exceeds_collateral_clamped() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open highly leveraged long: 1k collateral, 20k notional (20x)
    let position_id = fixture.open_long(&user, FEED_BTC, 1_000, 20_000, BTC_PRICE_I64);

    // Jump 1 week for interest, price drops 10%
    fixture.jump(SECONDS_PER_WEEK);
    let crash_price = fixture.btc_price(90_000 * PRICE_SCALAR as i64);
    let payout = fixture.trading.close_position(&position_id, &crash_price);

    // Loss (10% of 20k = 2000) exceeds collateral (1k) -- payout clamped to 0
    assert_eq!(payout, 0, "payout should be 0 when loss exceeds collateral");
    assert!(!fixture.position_exists(position_id));
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

    // Verify isolation
    assert_eq!(fixture.trading.get_user_positions(&user1).len(), 1);
    assert_eq!(fixture.trading.get_user_positions(&user2).len(), 1);
    assert_eq!(fixture.trading.get_position(&pos1).user, user1);
    assert_eq!(fixture.trading.get_position(&pos2).user, user2);

    // Close user1's position -- should not affect user2
    fixture.jump(31);
    let close_price = fixture.btc_price(110_000 * PRICE_SCALAR as i64);
    fixture.trading.close_position(&pos1, &close_price);

    assert!(!fixture.position_exists(pos1));
    assert!(fixture.position_exists(pos2));
    assert_eq!(fixture.trading.get_user_positions(&user1).len(), 0);
    assert_eq!(fixture.trading.get_user_positions(&user2).len(), 1);
}
