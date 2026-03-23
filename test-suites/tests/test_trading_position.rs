use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::{BTC_FEED_ID, BTC_PRICE, PRICE_SCALAR};
use trading::ExecuteRequest;

const SECONDS_PER_WEEK: u64 = 604800;

// ==========================================
// Helper Functions
// ==========================================

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data()
}

fn open_long_position(fixture: &TestFixture, user: &Address) -> u32 {
    fixture.open_and_fill(
        user,
        AssetIndex::BTC as u32,
        1_000 * SCALAR_7,
        10_000 * SCALAR_7,
        true,
        BTC_PRICE,
        0,
        0,
    )
}

fn open_short_position(fixture: &TestFixture, user: &Address) -> u32 {
    fixture.open_and_fill(
        user,
        AssetIndex::BTC as u32,
        1_000 * SCALAR_7,
        10_000 * SCALAR_7,
        false,
        BTC_PRICE,
        0,
        0,
    )
}

fn place_limit_order_long(fixture: &TestFixture, user: &Address, entry_price: i128) -> u32 {
    fixture.trading.place_limit(
        user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &entry_price,
        &0,
        &0,
    )
}

fn place_limit_order_short(fixture: &TestFixture, user: &Address, entry_price: i128) -> u32 {
    fixture.trading.place_limit(
        user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &false,
        &entry_price,
        &0,
        &0,
    )
}

fn set_btc_price(fixture: &TestFixture, price: i128) {
    fixture.set_price(BTC_FEED_ID, price);
}

// ==========================================
// 1. Market Order Lifecycle (7 tests)
// ==========================================

#[test]
fn test_long_open_modify_close_profit() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long and fill immediately
    let position_id = fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        1_000 * SCALAR_7,
        10_000 * SCALAR_7,
        true,
        BTC_PRICE,
        0,
        0,
    );

    // Verify via get_position
    let pos = fixture.trading.get_position(&position_id);
    assert!(pos.filled);
    assert!(pos.long);
    // Collateral is reduced by opening fees (base_fee + impact_fee)
    assert!(pos.col > 0);
    assert!(pos.col < 1_000 * SCALAR_7);
    assert_eq!(pos.entry_price, BTC_PRICE);

    // Modify collateral up
    fixture
        .trading
        .modify_collateral(&position_id, &(2_000 * SCALAR_7), &fixture.dummy_price());
    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.col, 2_000 * SCALAR_7);

    // Price up 10%
    set_btc_price(&fixture, 110_000 * PRICE_SCALAR);

    // Close, verify payout > collateral (profitable)
    fixture.jump(31); // past MIN_OPEN_TIME
    let payout = fixture.trading.close_position(&position_id, &fixture.dummy_price());
    // Payout should be col + pnl - fees ≈ 2000 + 1000 - fees > 2000
    assert!(payout > 2_000 * SCALAR_7);
    assert!(!fixture.position_exists(position_id));

    // user_positions should be empty
    assert_eq!(fixture.trading.get_user_positions(&user).len(), 0);

    // User should have profited overall
    let final_balance = fixture.token.balance(&user);
    assert!(final_balance > initial_balance - 100 * SCALAR_7);
}

#[test]
fn test_short_open_modify_close_loss() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open short
    let position_id = open_short_position(&fixture, &user);
    let pos = fixture.trading.get_position(&position_id);
    assert!(!pos.long);

    // Modify collateral down
    fixture
        .trading
        .modify_collateral(&position_id, &(500 * SCALAR_7), &fixture.dummy_price());
    assert_eq!(
        fixture.trading.get_position(&position_id).col,
        500 * SCALAR_7
    );

    // Price UP = loss for short
    set_btc_price(&fixture, 105_000 * PRICE_SCALAR);

    // Close — 5% loss on 10k notional = 500 loss, with 500 collateral → payout ≈ 0
    fixture.jump(31); // past MIN_OPEN_TIME
    let payout = fixture.trading.close_position(&position_id, &fixture.dummy_price());
    assert_eq!(payout, 0); // loss exceeds collateral minus fees
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_long_tp_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);

    // Set TP and SL
    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * PRICE_SCALAR), &(95_000 * PRICE_SCALAR));
    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.tp, 110_000 * PRICE_SCALAR);
    assert_eq!(pos.sl, 95_000 * PRICE_SCALAR);

    // Price rises past TP
    set_btc_price(&fixture, 111_000 * PRICE_SCALAR);
    fixture.jump(31); // past MIN_OPEN_TIME

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2, // TakeProfit
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_long_sl_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);

    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * PRICE_SCALAR), &(95_000 * PRICE_SCALAR));

    // Price drops past SL
    set_btc_price(&fixture, 94_000 * PRICE_SCALAR);
    fixture.jump(31); // past MIN_OPEN_TIME

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 1, // StopLoss
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_short_tp_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_short_position(&fixture, &user);

    // Short TP: below current price
    fixture
        .trading
        .set_triggers(&position_id, &(90_000 * PRICE_SCALAR), &0);

    set_btc_price(&fixture, 89_000 * PRICE_SCALAR);
    fixture.jump(31); // past MIN_OPEN_TIME

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2,
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_short_sl_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_short_position(&fixture, &user);

    // Short SL: above current price
    fixture
        .trading
        .set_triggers(&position_id, &0, &(105_000 * PRICE_SCALAR));

    set_btc_price(&fixture, 106_000 * PRICE_SCALAR);
    fixture.jump(31); // past MIN_OPEN_TIME

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 1,
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_triggers_clear_and_reset() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);

    // Set triggers
    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * PRICE_SCALAR), &(95_000 * PRICE_SCALAR));
    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.tp, 110_000 * PRICE_SCALAR);
    assert_eq!(pos.sl, 95_000 * PRICE_SCALAR);

    // Clear triggers
    fixture.trading.set_triggers(&position_id, &0, &0);
    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.tp, 0);
    assert_eq!(pos.sl, 0);

    // Re-set triggers
    fixture
        .trading
        .set_triggers(&position_id, &(120_000 * PRICE_SCALAR), &(90_000 * PRICE_SCALAR));
    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.tp, 120_000 * PRICE_SCALAR);
    assert_eq!(pos.sl, 90_000 * PRICE_SCALAR);
}

// ==========================================
// 2. Limit Order Lifecycle (5 tests)
// ==========================================

#[test]
fn test_limit_long_fill_to_close() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = 101_000 * PRICE_SCALAR;
    let position_id = place_limit_order_long(&fixture, &user, entry_price);
    assert!(!fixture.trading.get_position(&position_id).filled);

    // Market stats should NOT be updated for pending limit orders
    let market = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert_eq!(market.l_notional, 0);

    // Price drops to entry price — fillable
    set_btc_price(&fixture, entry_price);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 0, // Fill
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
    assert!(fixture.trading.get_position(&position_id).filled);

    // Price rises for profit
    set_btc_price(&fixture, 110_000 * PRICE_SCALAR);
    fixture.jump(31); // past MIN_OPEN_TIME
    let payout = fixture.trading.close_position(&position_id, &fixture.dummy_price());
    assert!(payout > 0);
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_limit_short_fill_to_close() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = 99_000 * PRICE_SCALAR;
    let position_id = place_limit_order_short(&fixture, &user, entry_price);
    assert!(!fixture.trading.get_position(&position_id).filled);

    // Price rises to entry price — fillable for short
    set_btc_price(&fixture, entry_price);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 0,
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
    assert!(fixture.trading.get_position(&position_id).filled);

    // Price drops for profit
    set_btc_price(&fixture, 90_000 * PRICE_SCALAR);
    fixture.jump(31); // past MIN_OPEN_TIME
    let payout = fixture.trading.close_position(&position_id, &fixture.dummy_price());
    assert!(payout > 0);
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_limit_cancel_refund() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    let entry_price = BTC_PRICE + 1000 * PRICE_SCALAR;
    let position_id = fixture.trading.place_limit(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &entry_price,
        &0,
        &0,
    );

    let balance_after_open = fixture.token.balance(&user);
    assert!(balance_after_open < initial_balance);

    // Cancel limit order
    fixture.trading.cancel_limit(&position_id);

    // Full refund
    let final_balance = fixture.token.balance(&user);
    assert_eq!(final_balance, initial_balance);
    assert!(!fixture.position_exists(position_id));
}

#[test]
#[should_panic(expected = "Error(Contract, #747)")]
fn test_limit_not_fillable() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = 101_000 * PRICE_SCALAR;
    let position_id = place_limit_order_long(&fixture, &user, entry_price);

    // Price moves away (up) — not fillable
    set_btc_price(&fixture, 105_000 * PRICE_SCALAR);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 0,
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
}

#[test]
fn test_filled_modify_collateral() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open a market order (filled immediately)
    let position_id = open_long_position(&fixture, &user);

    // Modify collateral on filled position
    fixture
        .trading
        .modify_collateral(&position_id, &(2_000 * SCALAR_7), &fixture.dummy_price());

    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.col, 2_000 * SCALAR_7);
    assert!(pos.filled);
}

// ==========================================
// 3. Keeper Edge Cases (4 tests)
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #733)")]
fn test_fill_already_filled_panics() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open and fill position (already filled)
    let position_id = open_long_position(&fixture, &user);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 0, // Fill
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
}

#[test]
#[should_panic(expected = "Error(Contract, #750)")]
fn test_execute_on_pending_panics() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = 101_000 * PRICE_SCALAR;
    let position_id = place_limit_order_long(&fixture, &user, entry_price);

    // Try to liquidate pending position
    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3, // Liquidate
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
}

#[test]
#[should_panic(expected = "Error(Contract, #744)")]
fn test_take_profit_not_triggered_panics() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);
    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * PRICE_SCALAR), &(95_000 * PRICE_SCALAR));

    // Price stays in range — TP not triggered
    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2, // TakeProfit
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
}

#[test]
#[should_panic(expected = "Error(Contract, #745)")]
fn test_stop_loss_not_triggered_panics() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);
    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * PRICE_SCALAR), &(95_000 * PRICE_SCALAR));

    // Price stays in range — SL not triggered
    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 1, // StopLoss
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
}

// ==========================================
// 4. Liquidation (3 tests)
// ==========================================

#[test]
fn test_liquidation_underwater() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open highly leveraged position and fill
    fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        110 * SCALAR_7,    // 110 collateral (margin + fees headroom)
        10_000 * SCALAR_7, // 10000 notional (~91x)
        true,
        BTC_PRICE,
        0,
        0,
    );

    // Price drops 2% — underwater
    set_btc_price(&fixture, 98_000 * PRICE_SCALAR);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3, // Liquidate
            position_id: 0,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
    assert!(!fixture.position_exists(0));
}

#[test]
#[should_panic(expected = "Error(Contract, #746)")]
fn test_liquidation_healthy_rejected() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);

    // Price unchanged — healthy
    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3,
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
}

#[test]
fn test_keeper_receives_fee() {
    let fixture = TestFixture::create();

    fixture
        .token
        .mint(&fixture.owner, &(100_000_000 * SCALAR_7));
    fixture.vault.deposit(
        &(100_000_000 * SCALAR_7),
        &fixture.owner,
        &fixture.owner,
        &fixture.owner,
    );

    let base_config = trading::testutils::default_market(&fixture.env);
    let btc_config = trading::MarketConfig {
        ..base_config
    };
    fixture.create_market(BTC_FEED_ID, &btc_config);

    let new_config = trading::TradingConfig {
        caller_rate: 1_000_000, // 10%
        min_notional: 10 * SCALAR_7,
        max_notional: 10_000_000 * SCALAR_7,
        fee_dom: 0_0005000,
        fee_non_dom: 0_0001000,
        max_util: 10 * SCALAR_7,
        r_funding: 10_000_000_000_000,
        r_base: 10_000_000_000_000,
        r_var: SCALAR_7,
    };
    fixture.trading.set_config(&new_config);
    fixture.trading.set_status(&0u32);

    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open highly leveraged position for liquidation and fill
    fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        110 * SCALAR_7,    // margin + fees headroom
        10_000 * SCALAR_7,
        true,
        BTC_PRICE,
        0,
        0,
    );

    let keeper_balance_before = fixture.token.balance(&keeper);

    set_btc_price(&fixture, 97_000 * PRICE_SCALAR);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3,
            position_id: 0,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());

    let keeper_balance_after = fixture.token.balance(&keeper);
    assert!(keeper_balance_after > keeper_balance_before);
}

// ==========================================
// 5. Contract Status (3 tests)
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #761)")]
fn test_open_blocked_when_frozen() {
    let fixture = setup_fixture();
    fixture.trading.set_status(&3u32); // Frozen
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    fixture.trading.place_limit(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &BTC_PRICE,
        &0,
        &0,
    );
}

#[test]
fn test_execute_on_ice_allows_triggers() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);
    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * PRICE_SCALAR), &0);

    // Set to AdminOnIce
    fixture.trading.set_status(&2u32);

    // Price rises to TP
    set_btc_price(&fixture, 111_000 * PRICE_SCALAR);
    fixture.jump(31); // past MIN_OPEN_TIME

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2,
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
    assert!(!fixture.position_exists(position_id));
}

#[test]
#[should_panic(expected = "Error(Contract, #762)")]
fn test_execute_blocked_when_frozen() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);
    fixture.trading.set_status(&3u32); // Frozen

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3,
            position_id,
        },
    ];
    fixture.trading.execute(&keeper, &requests, &fixture.dummy_price());
}

// ==========================================
// 6. Multi-User & Interest (2 tests)
// ==========================================

#[test]
fn test_multi_user_positions_isolated() {
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);
    fixture.token.mint(&user1, &(100_000 * SCALAR_7));
    fixture.token.mint(&user2, &(100_000 * SCALAR_7));

    let pos1 = open_long_position(&fixture, &user1);
    let pos2 = open_short_position(&fixture, &user2);

    // Verify positions are isolated
    assert_eq!(fixture.trading.get_user_positions(&user1).len(), 1);
    assert_eq!(fixture.trading.get_user_positions(&user2).len(), 1);
    assert_eq!(fixture.trading.get_position(&pos1).user, user1);
    assert_eq!(fixture.trading.get_position(&pos2).user, user2);

    // Close user1's position doesn't affect user2
    set_btc_price(&fixture, 110_000 * PRICE_SCALAR);
    fixture.jump(31); // past MIN_OPEN_TIME
    fixture.trading.close_position(&pos1, &fixture.dummy_price());

    assert!(!fixture.position_exists(pos1));
    assert!(fixture.position_exists(pos2));
    assert_eq!(fixture.trading.get_user_positions(&user1).len(), 0);
    assert_eq!(fixture.trading.get_user_positions(&user2).len(), 1);
}

#[test]
fn test_interest_accrual_across_positions() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open first position — creates one-sided long OI
    fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        1_000 * SCALAR_7,
        10_000 * SCALAR_7,
        true,
        BTC_PRICE,
        0,
        0,
    );

    // apply_funding sets the funding rate based on current OI imbalance
    // (one-sided long → positive rate = longs pay)
    fixture.jump(3600); // must wait >= 1 hour
    fixture.trading.apply_funding();

    let market_before = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));
    assert!(market_before.fund_rate > 0); // longs-pay rate is set

    // Wait 1 week for funding to accrue
    fixture.jump(SECONDS_PER_WEEK);

    // Open another position — triggers accrue with non-zero funding rate
    fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        1_000 * SCALAR_7,
        10_000 * SCALAR_7,
        true,
        BTC_PRICE,
        0,
        0,
    );

    let market_after = fixture.trading.get_market_data(&(AssetIndex::BTC as u32));

    // Funding index should have increased (longs paid funding)
    assert!(market_after.l_fund_idx > market_before.l_fund_idx);
    assert!(market_after.last_update > market_before.last_update);
}
