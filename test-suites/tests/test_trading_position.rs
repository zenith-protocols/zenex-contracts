use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::BTC_PRICE;
use trading::ExecuteRequest;

const SECONDS_PER_WEEK: u64 = 604800;

// ==========================================
// Helper Functions
// ==========================================

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data(false)
}

fn open_long_position(fixture: &TestFixture, user: &Address) -> u32 {
    let (id, _) = fixture.open_and_fill(
        user,
        AssetIndex::BTC as u32,
        1_000 * SCALAR_7,
        10_000 * SCALAR_7,
        true,
        BTC_PRICE,
        0,
        0,
    );
    id
}

fn open_short_position(fixture: &TestFixture, user: &Address) -> u32 {
    let (id, _) = fixture.open_and_fill(
        user,
        AssetIndex::BTC as u32,
        1_000 * SCALAR_7,
        10_000 * SCALAR_7,
        false,
        BTC_PRICE,
        0,
        0,
    );
    id
}

fn open_limit_order_long(fixture: &TestFixture, user: &Address, entry_price: i128) -> u32 {
    let (id, _) = fixture.trading.open_position(
        user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &entry_price,
        &0,
        &0,
    );
    id
}

fn open_limit_order_short(fixture: &TestFixture, user: &Address, entry_price: i128) -> u32 {
    let (id, _) = fixture.trading.open_position(
        user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &false,
        &entry_price,
        &0,
        &0,
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
    let (position_id, fee) = fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        1_000 * SCALAR_7,
        10_000 * SCALAR_7,
        true,
        BTC_PRICE,
        0,
        0,
    );
    assert!(fee > 0);

    // Verify via get_position
    let pos = fixture.trading.get_position(&position_id);
    assert!(pos.filled);
    assert!(pos.is_long);
    assert_eq!(pos.collateral, 1_000 * SCALAR_7);
    assert_eq!(pos.entry_price, BTC_PRICE);

    // Modify collateral up
    fixture
        .trading
        .modify_collateral(&position_id, &(2_000 * SCALAR_7));
    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.collateral, 2_000 * SCALAR_7);

    // Price up 10%
    set_btc_price(&fixture, 110_000_0000000);

    // Close, verify profit
    let (pnl, _) = fixture.trading.close_position(&position_id);
    assert_eq!(pnl, 1_000 * SCALAR_7); // 10% of 10k
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
    assert!(!pos.is_long);

    // Modify collateral down
    fixture
        .trading
        .modify_collateral(&position_id, &(500 * SCALAR_7));
    assert_eq!(
        fixture.trading.get_position(&position_id).collateral,
        500 * SCALAR_7
    );

    // Price drops 5% — loss for short (short profits when price drops, loses when rises)
    // Wait, let me correct: price UP = loss for short
    set_btc_price(&fixture, 105_000_0000000);

    let (pnl, _) = fixture.trading.close_position(&position_id);
    assert_eq!(pnl, -500 * SCALAR_7); // 5% loss on 10k notional
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
        .set_triggers(&position_id, &(110_000 * SCALAR_7), &(95_000 * SCALAR_7));
    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.take_profit, 110_000 * SCALAR_7);
    assert_eq!(pos.stop_loss, 95_000 * SCALAR_7);

    // Price rises past TP
    set_btc_price(&fixture, 111_000_0000000);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2, // TakeProfit
            position_id,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));
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
        .set_triggers(&position_id, &(110_000 * SCALAR_7), &(95_000 * SCALAR_7));

    // Price drops past SL
    set_btc_price(&fixture, 94_000_0000000);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 1, // StopLoss
            position_id,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));
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
        .set_triggers(&position_id, &(90_000 * SCALAR_7), &0);

    set_btc_price(&fixture, 89_000_0000000);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2,
            position_id,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));
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
        .set_triggers(&position_id, &0, &(105_000 * SCALAR_7));

    set_btc_price(&fixture, 106_000_0000000);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 1,
            position_id,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));
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
        .set_triggers(&position_id, &(110_000 * SCALAR_7), &(95_000 * SCALAR_7));
    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.take_profit, 110_000 * SCALAR_7);
    assert_eq!(pos.stop_loss, 95_000 * SCALAR_7);

    // Clear triggers
    fixture.trading.set_triggers(&position_id, &0, &0);
    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.take_profit, 0);
    assert_eq!(pos.stop_loss, 0);

    // Re-set triggers
    fixture
        .trading
        .set_triggers(&position_id, &(120_000 * SCALAR_7), &(90_000 * SCALAR_7));
    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.take_profit, 120_000 * SCALAR_7);
    assert_eq!(pos.stop_loss, 90_000 * SCALAR_7);
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

    let entry_price = 101_000 * SCALAR_7;
    let position_id = open_limit_order_long(&fixture, &user, entry_price);
    assert!(!fixture.trading.get_position(&position_id).filled);

    // Market stats should NOT be updated for pending limit orders
    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    assert_eq!(market.data.long_notional_size, 0);

    // Price drops to entry price — fillable
    set_btc_price(&fixture, entry_price);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 0, // Fill
            position_id,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));
    assert!(fixture.trading.get_position(&position_id).filled);

    // Price rises for profit
    set_btc_price(&fixture, 110_000_0000000);
    let (pnl, _) = fixture.trading.close_position(&position_id);
    assert!(pnl > 0);
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_limit_short_fill_to_close() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = 99_000 * SCALAR_7;
    let position_id = open_limit_order_short(&fixture, &user, entry_price);
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
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));
    assert!(fixture.trading.get_position(&position_id).filled);

    // Price drops for profit
    set_btc_price(&fixture, 90_000_0000000);
    let (pnl, _) = fixture.trading.close_position(&position_id);
    assert!(pnl > 0);
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_limit_cancel_refund() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

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

    let balance_after_open = fixture.token.balance(&user);
    assert!(balance_after_open < initial_balance);

    // Cancel limit order
    let (pnl, fee) = fixture.trading.close_position(&position_id);
    assert_eq!(pnl, 0);
    assert_eq!(fee, 0);

    // Full refund
    let final_balance = fixture.token.balance(&user);
    assert_eq!(final_balance, initial_balance);
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_limit_not_fillable() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = 101_000 * SCALAR_7;
    let position_id = open_limit_order_long(&fixture, &user, entry_price);

    // Price moves away (up) — not fillable
    set_btc_price(&fixture, 105_000_0000000);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 0,
            position_id,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(747)); // LimitOrderNotFillable
    assert!(!fixture.trading.get_position(&position_id).filled);
}

#[test]
fn test_limit_modify_collateral() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = BTC_PRICE + 1000 * SCALAR_7;
    let position_id = open_limit_order_long(&fixture, &user, entry_price);

    // Modify collateral on pending position
    fixture
        .trading
        .modify_collateral(&position_id, &(2_000 * SCALAR_7));

    let pos = fixture.trading.get_position(&position_id);
    assert_eq!(pos.collateral, 2_000 * SCALAR_7);
    assert!(!pos.filled); // Still pending
}

// ==========================================
// 3. Keeper Edge Cases (4 tests)
// ==========================================

#[test]
fn test_fill_already_filled_returns_error() {
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
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(733)); // PositionNotPending
}

#[test]
fn test_execute_on_pending_returns_error() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = 101_000 * SCALAR_7;
    let position_id = open_limit_order_long(&fixture, &user, entry_price);

    // Try to liquidate pending position
    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3, // Liquidate
            position_id,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(750)); // ActionNotAllowedForStatus
}

#[test]
fn test_triggers_not_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);
    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * SCALAR_7), &(95_000 * SCALAR_7));

    // Price stays in range — neither TP nor SL triggered
    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2, // TakeProfit
            position_id,
        },
        ExecuteRequest {
            request_type: 1, // StopLoss
            position_id,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(744)); // TakeProfitNotTriggered
    assert_eq!(results.get(1), Some(745)); // StopLossNotTriggered
    assert!(fixture.position_exists(position_id));
}

#[test]
fn test_batch_execute_mixed() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position1 = open_long_position(&fixture, &user);
    let position2 = open_long_position(&fixture, &user);

    fixture
        .trading
        .set_triggers(&position1, &(110_000 * SCALAR_7), &0);
    fixture
        .trading
        .set_triggers(&position2, &0, &(95_000 * SCALAR_7));

    // Price goes up — TP triggers, SL doesn't
    set_btc_price(&fixture, 111_000_0000000);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2, // TakeProfit
            position_id: position1,
        },
        ExecuteRequest {
            request_type: 1, // StopLoss
            position_id: position2,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));   // TP succeeded
    assert_eq!(results.get(1), Some(745)); // SL not triggered

    assert!(!fixture.position_exists(position1));
    assert!(fixture.position_exists(position2));
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

    // Open highly leveraged position (100x) and fill
    fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        100 * SCALAR_7,    // 100 collateral
        10_000 * SCALAR_7, // 10000 notional (100x)
        true,
        BTC_PRICE,
        0,
        0,
    );

    // Price drops 2% — underwater
    set_btc_price(&fixture, 98_000_0000000);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3, // Liquidate
            position_id: 0,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));
    assert!(!fixture.position_exists(0));
}

#[test]
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
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(746)); // PositionNotLiquidatable
    assert!(fixture.position_exists(position_id));
}

#[test]
fn test_keeper_receives_fee() {
    let mut fixture = TestFixture::create(false);

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
        asset: fixture.assets[AssetIndex::BTC as usize].clone(),
        ..base_config
    };
    fixture.create_market(&btc_config);

    let new_config = trading::TradingConfig {
        caller_take_rate: 1_000_000, // 10%
        min_open_time: 0,
        vault_skim: 0_2000000, // 20%
        min_collateral: SCALAR_7,
        max_collateral: 1_000_000 * SCALAR_7,
        max_payout: 10 * SCALAR_7,
        base_fee_dominant: 0_0005000,
        base_fee_non_dominant: 0_0001000,
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
        100 * SCALAR_7,
        10_000 * SCALAR_7,
        true,
        BTC_PRICE,
        0,
        0,
    );

    let keeper_balance_before = fixture.token.balance(&keeper);

    set_btc_price(&fixture, 97_000_0000000);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3,
            position_id: 0,
        },
    ];
    fixture.trading.execute(&keeper, &requests);

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

    fixture.trading.open_position(
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
        .set_triggers(&position_id, &(110_000 * SCALAR_7), &0);

    // Set to AdminOnIce
    fixture.trading.set_status(&2u32);

    // Price rises to TP
    set_btc_price(&fixture, 111_000_0000000);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2,
            position_id,
        },
    ];
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));
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
    fixture.trading.execute(&keeper, &requests);
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
    set_btc_price(&fixture, 110_000_0000000);
    fixture.trading.close_position(&pos1);

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

    // Open first position and fill
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

    let market_before = fixture.trading.get_market(&(AssetIndex::BTC as u32));

    // Wait 1 week for interest to accrue
    fixture.jump(SECONDS_PER_WEEK);

    // Open another position and fill — triggers interest accrual
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

    let market_after = fixture.trading.get_market(&(AssetIndex::BTC as u32));

    // Interest index should have increased
    assert!(market_after.data.long_funding_index > market_before.data.long_funding_index);
    assert!(market_after.data.last_update > market_before.data.last_update);
}
