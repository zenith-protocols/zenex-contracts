use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
const SECONDS_PER_WEEK: u64 = 604800;
use trading::testutils::BTC_PRICE;

// ==========================================
// Helper Functions
// ==========================================

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data(false)
}

// ==========================================
// Open Position - Market Order Tests
// ==========================================

#[test]
fn test_open_market_order_long() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let (position_id, fee) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),  // collateral
        &(10_000 * SCALAR_7), // notional_size (10x leverage)
        &true,                // is_long
        &0,                   // entry_price = 0 for market order
        &0,                   // take_profit
        &0,                   // stop_loss
    );

    assert_eq!(position_id, 0);
    assert!(fee > 0);

    // Verify position was created
    let position = fixture.trading.get_position(&position_id);
    assert!(position.filled);
    assert!(position.is_long);
    assert_eq!(position.collateral, 1_000 * SCALAR_7);
    assert_eq!(position.notional_size, 10_000 * SCALAR_7);
    assert_eq!(position.entry_price, BTC_PRICE);
}

#[test]
fn test_open_market_order_short() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let (position_id, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &false, // short
        &0,
        &0,
        &0,
    );

    let position = fixture.trading.get_position(&position_id);
    assert!(!position.is_long);
    assert!(position.filled);
}

#[test]
fn test_open_market_order_updates_market_stats() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let initial_market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    assert_eq!(initial_market.data.long_notional_size, 0);

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
}

// ==========================================
// Open Position - Limit Order Tests
// ==========================================

#[test]
fn test_open_limit_order_long() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // For long, entry_price must be >= current price
    let entry_price = BTC_PRICE + 1000 * SCALAR_7; // Above current

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
    assert!(!position.filled); // Limit order starts as pending
    assert_eq!(position.entry_price, entry_price);

    // Market stats should NOT be updated for pending limit orders
    let market = fixture.trading.get_market(&(AssetIndex::BTC as u32));
    assert_eq!(market.data.long_notional_size, 0);
}

#[test]
fn test_open_limit_order_short() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // For short, entry_price must be <= current price
    let entry_price = BTC_PRICE - 1000 * SCALAR_7; // Below current

    let (position_id, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &false,
        &entry_price,
        &0,
        &0,
    );

    let position = fixture.trading.get_position(&position_id);
    assert!(!position.filled);
    assert!(!position.is_long);
}

#[test]
#[should_panic(expected = "Error(Contract, #334)")]
fn test_open_limit_order_invalid_long_entry_price() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // For long, entry_price below current is invalid
    let entry_price = BTC_PRICE - 1000 * SCALAR_7;

    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &entry_price,
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #334)")]
fn test_open_limit_order_invalid_short_entry_price() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // For short, entry_price above current is invalid
    let entry_price = BTC_PRICE + 1000 * SCALAR_7;

    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &false,
        &entry_price,
        &0,
        &0,
    );
}

// ==========================================
// Open Position - Validation Tests
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #382)")]
fn test_open_position_contract_paused() {
    let fixture = setup_fixture();
    fixture.trading.set_status(&2u32); // Frozen
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

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
}

#[test]
#[should_panic(expected = "Error(Contract, #331)")]
fn test_open_position_collateral_below_minimum() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(SCALAR_7 - 1), // Below min_collateral
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #332)")]
fn test_open_position_collateral_above_maximum() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(2_000_000 * SCALAR_7), // Above max_collateral (1M)
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #330)")]
fn test_open_position_negative_collateral() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(-1),
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #329)")]
fn test_open_position_max_positions_reached() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(1_000_000 * SCALAR_7));

    // Open 10 positions (max_positions default is 10)
    for _ in 0..10 {
        fixture.trading.open_position(
            &user,
            &(AssetIndex::BTC as u32),
            &(10 * SCALAR_7),
            &(100 * SCALAR_7),
            &true,
            &0,
            &0,
            &0,
        );
    }

    // 11th should fail
    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(10 * SCALAR_7),
        &(100 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
}

// ==========================================
// Close Position Tests
// ==========================================

#[test]
fn test_close_market_order_profit() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open position
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

    // Price goes up 10%
    fixture.oracle.set_price_stable(&soroban_sdk::vec![
        &fixture.env,
        1_0000000,       // USD
        110_000_0000000, // BTC = 110K (+10%)
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // Close position
    let (pnl, fee) = fixture.trading.close_position(&position_id);

    // 10% gain on 10k = 1000 profit
    assert_eq!(pnl, 1_000 * SCALAR_7);
    assert!(fee > 0);

    // Position should be deleted
    assert!(!fixture.position_exists(position_id));

    // User should have more than initial (profit minus fees)
    let final_balance = fixture.token.balance(&user);
    assert!(final_balance > initial_balance - 100 * SCALAR_7); // profit minus fees
}

#[test]
fn test_close_market_order_loss() {
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

    // Price goes down 5%
    fixture.oracle.set_price_stable(&soroban_sdk::vec![
        &fixture.env,
        1_0000000,      // USD
        95_000_0000000, // BTC = 95K (-5%)
        2000_0000000,   // ETH
        0_1000000,      // XLM
    ]);

    let (pnl, _) = fixture.trading.close_position(&position_id);

    // 5% loss on 10k = -500
    assert_eq!(pnl, -500 * SCALAR_7);
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_close_pending_limit_order_refunds() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open limit order
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

    // Cancel the limit order
    let (pnl, fee) = fixture.trading.close_position(&position_id);

    // Cancelled orders have no PnL or fee
    assert_eq!(pnl, 0);
    assert_eq!(fee, 0);

    // User should get refund (collateral + fees)
    let final_balance = fixture.token.balance(&user);
    assert_eq!(final_balance, initial_balance);
}

// ==========================================
// Modify Collateral Tests
// ==========================================

#[test]
fn test_modify_collateral_add() {
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

    let initial_collateral = fixture.trading.get_position(&position_id).collateral;

    fixture
        .trading
        .modify_collateral(&position_id, &(2_000 * SCALAR_7)); // Increase to 2000

    let position = fixture.trading.get_position(&position_id);
    assert_eq!(position.collateral, 2_000 * SCALAR_7);
    assert!(position.collateral > initial_collateral);
}

#[test]
fn test_modify_collateral_remove() {
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

    fixture
        .trading
        .modify_collateral(&position_id, &(500 * SCALAR_7)); // Decrease to 500

    let position = fixture.trading.get_position(&position_id);
    assert_eq!(position.collateral, 500 * SCALAR_7);
}

#[test]
#[should_panic(expected = "Error(Contract, #337)")]
fn test_modify_collateral_withdrawal_breaks_margin() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open with min init_margin (1%)
    // notional = 10000, init_margin = 100
    let (position_id, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7), // Exactly at init margin
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    // Try to withdraw - should fail
    fixture
        .trading
        .modify_collateral(&position_id, &(50 * SCALAR_7)); // Below init margin
}

#[test]
fn test_modify_collateral_pending_position() {
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

    // Modify collateral on pending position (now allowed)
    fixture
        .trading
        .modify_collateral(&position_id, &(2_000 * SCALAR_7));

    let position = fixture.trading.get_position(&position_id);
    assert_eq!(position.collateral, 2_000 * SCALAR_7);
    assert!(!position.filled); // Still pending
}

#[test]
#[should_panic(expected = "Error(Contract, #330)")]
fn test_modify_collateral_to_zero() {
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

    fixture.trading.modify_collateral(&position_id, &0);
}

// ==========================================
// Set Triggers Tests
// ==========================================

#[test]
fn test_set_triggers_long() {
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

    // For long: TP > current, SL < current
    let take_profit = 110_000 * SCALAR_7;
    let stop_loss = 95_000 * SCALAR_7;

    fixture
        .trading
        .set_triggers(&position_id, &take_profit, &stop_loss);

    let position = fixture.trading.get_position(&position_id);
    assert_eq!(position.take_profit, 110_000 * SCALAR_7);
    assert_eq!(position.stop_loss, 95_000 * SCALAR_7);
}

#[test]
fn test_set_triggers_short() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let (position_id, _) = fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &false, // short
        &0,
        &0,
        &0,
    );

    // For short: TP < current, SL > current
    let take_profit = 90_000 * SCALAR_7;
    let stop_loss = 105_000 * SCALAR_7;

    fixture
        .trading
        .set_triggers(&position_id, &take_profit, &stop_loss);

    let position = fixture.trading.get_position(&position_id);
    assert_eq!(position.take_profit, 90_000 * SCALAR_7);
    assert_eq!(position.stop_loss, 105_000 * SCALAR_7);
}

#[test]
fn test_set_triggers_clear() {
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

    let take_profit = 110_000 * SCALAR_7;
    let stop_loss = 95_000 * SCALAR_7;
    fixture
        .trading
        .set_triggers(&position_id, &take_profit, &stop_loss);

    // Clear triggers
    fixture.trading.set_triggers(&position_id, &0, &0);

    let position = fixture.trading.get_position(&position_id);
    assert_eq!(position.take_profit, 0);
    assert_eq!(position.stop_loss, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #340)")]
fn test_set_triggers_invalid_tp_long() {
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

    // For long, TP below current is invalid
    let take_profit = 95_000 * SCALAR_7;
    fixture.trading.set_triggers(&position_id, &take_profit, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #341)")]
fn test_set_triggers_invalid_sl_long() {
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

    // For long, SL above current is invalid
    let stop_loss = 105_000 * SCALAR_7;
    fixture.trading.set_triggers(&position_id, &0, &stop_loss);
}

// ==========================================
// Interest Accrual Tests
// ==========================================

#[test]
fn test_open_position_accrues_interest() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open first position
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

    let market_before = fixture.trading.get_market(&(AssetIndex::BTC as u32));

    // Time passes - need enough time for interest to accrue
    // With base_hourly_rate = 10^13 and SCALAR_18 = 10^18, need > ~28 hours
    // for the integer math to produce a non-zero result
    fixture.jump(SECONDS_PER_WEEK);

    // Open another position - this should trigger interest accrual
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

    let market_after = fixture.trading.get_market(&(AssetIndex::BTC as u32));

    // Interest index should have increased after a week
    assert!(market_after.data.long_interest_index > market_before.data.long_interest_index);
    // last_update should have been updated
    assert!(market_after.data.last_update > market_before.data.last_update);
}
