use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::ExecuteRequest;

// ==========================================
// Helper Functions
// ==========================================

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data(false)
}

fn open_long_position(fixture: &TestFixture, user: &Address) -> u32 {
    let (id, _) = fixture.trading.open_position(
        user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );
    id
}

fn open_short_position(fixture: &TestFixture, user: &Address) -> u32 {
    let (id, _) = fixture.trading.open_position(
        user,
        &(AssetIndex::BTC as u32),
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &false,
        &0,
        &0,
        &0,
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

// ==========================================
// Fill Limit Order Tests
// ==========================================

#[test]
fn test_fill_limit_order_long() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open limit order at entry price above current (101k vs 100k)
    let entry_price = 101_000 * SCALAR_7;
    let position_id = open_limit_order_long(&fixture, &user, entry_price);

    // Verify position is pending
    assert!(!fixture.read_position(position_id).filled);

    // Price drops to entry price - should be fillable
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,   // USD
        entry_price, // BTC = entry_price
        2000_0000000,
        0_1000000,
    ]);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 0, // Fill
            position_id,
        },
    ];

    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0)); // Success

    // Position should now be filled
    assert!(fixture.read_position(position_id).filled);
}

#[test]
fn test_fill_limit_order_short() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open limit order at entry price below current (99k vs 100k)
    let entry_price = 99_000 * SCALAR_7;
    let position_id = open_limit_order_short(&fixture, &user, entry_price);

    // Price rises to entry price - should be fillable
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,
        entry_price,
        2000_0000000,
        0_1000000,
    ]);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 0,
            position_id,
        },
    ];

    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));

    assert!(fixture.read_position(position_id).filled);
}

#[test]
fn test_fill_limit_order_not_fillable() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open limit order at entry price slightly above current (100k)
    // For long: created when entry_price >= current_price
    // For long: fillable when current_price <= entry_price
    let entry_price = 101_000 * SCALAR_7;
    let position_id = open_limit_order_long(&fixture, &user, entry_price);

    // Raise the price above entry_price so the order is NOT fillable
    // current_price (105k) > entry_price (101k) → not fillable
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,
        105_000_0000000,
        2000_0000000,
        0_1000000,
    ]);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 0,
            position_id,
        },
    ];

    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(346)); // TradingError::LimitOrderNotFillable

    // Position should still be pending
    assert!(!fixture.read_position(position_id).filled);
}

#[test]
fn test_fill_already_filled_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open market order (already filled)
    let position_id = open_long_position(&fixture, &user);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 0,
            position_id,
        },
    ];

    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(328)); // TradingError::PositionNotPending
}

// ==========================================
// Stop Loss Tests
// ==========================================

#[test]
fn test_stop_loss_long_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);

    // Set stop loss
    fixture
        .trading
        .set_triggers(&position_id, &0, &(95_000 * SCALAR_7));

    // Price drops below stop loss
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,
        94_000_0000000, // BTC = 94K
        2000_0000000,
        0_1000000,
    ]);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 1, // StopLoss
            position_id,
        },
    ];

    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0)); // Success

    // Position should be closed
    assert!(!fixture.position_exists(position_id));
}

#[test]
fn test_stop_loss_short_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_short_position(&fixture, &user);

    // Set stop loss above current price
    fixture
        .trading
        .set_triggers(&position_id, &0, &(105_000 * SCALAR_7));

    // Price rises above stop loss
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,
        106_000_0000000, // BTC = 106K
        2000_0000000,
        0_1000000,
    ]);

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
fn test_stop_loss_not_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);

    fixture
        .trading
        .set_triggers(&position_id, &0, &(95_000 * SCALAR_7));

    // Price stays above stop loss
    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 1,
            position_id,
        },
    ];

    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(343)); // TradingError::StopLossNotTriggered

    // Position should still exist
    assert!(fixture.position_exists(position_id));
}

// ==========================================
// Take Profit Tests
// ==========================================

#[test]
fn test_take_profit_long_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);

    // Set take profit
    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * SCALAR_7), &0);

    // Price rises to take profit
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,
        111_000_0000000, // BTC = 111K
        2000_0000000,
        0_1000000,
    ]);

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
fn test_take_profit_short_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_short_position(&fixture, &user);

    // Set take profit below current
    fixture
        .trading
        .set_triggers(&position_id, &(90_000 * SCALAR_7), &0);

    // Price drops to take profit
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,
        89_000_0000000, // BTC = 89K
        2000_0000000,
        0_1000000,
    ]);

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
fn test_take_profit_not_triggered() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);

    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * SCALAR_7), &0);

    // Price stays below take profit
    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2,
            position_id,
        },
    ];

    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(342)); // TradingError::TakeProfitNotTriggered

    assert!(fixture.position_exists(position_id));
}

// ==========================================
// Liquidation Tests
// ==========================================

#[test]
fn test_liquidation_underwater_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open highly leveraged position
    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7),     // 100 collateral
        &(10_000 * SCALAR_7),  // 10000 notional (100x leverage)
        &true,
        &0,
        &0,
        &0,
    );

    // Price drops 2% - position is underwater
    // maintenance_margin = 0.5%, so 10000 * 0.005 = 50 margin required
    // 2% drop = 200 loss, with 100 collateral = -100 equity
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,
        98_000_0000000, // BTC = 98K (-2%)
        2000_0000000,
        0_1000000,
    ]);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3, // Liquidate
            position_id: 1,
        },
    ];

    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));

    assert!(!fixture.position_exists(1));
}

#[test]
fn test_liquidation_healthy_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);

    // Price stays same - position is healthy
    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3,
            position_id,
        },
    ];

    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(345)); // TradingError::PositionNotLiquidatable

    assert!(fixture.position_exists(position_id));
}

// ==========================================
// Batch Execution Tests
// ==========================================

#[test]
fn test_batch_execute_mixed_results() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open two positions
    let position1 = open_long_position(&fixture, &user);
    let position2 = open_long_position(&fixture, &user);

    // Set TP on position1, SL on position2
    fixture
        .trading
        .set_triggers(&position1, &(110_000 * SCALAR_7), &0);
    fixture
        .trading
        .set_triggers(&position2, &0, &(95_000 * SCALAR_7));

    // Price goes up - TP should trigger, SL should not
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,
        111_000_0000000, // BTC = 111K
        2000_0000000,
        0_1000000,
    ]);

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
    assert_eq!(results.get(0), Some(0)); // TP succeeded
    assert_eq!(results.get(1), Some(343)); // SL failed (StopLossNotTriggered)

    assert!(!fixture.position_exists(position1)); // Closed
    assert!(fixture.position_exists(position2)); // Still open
}

#[test]
fn test_execute_on_pending_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let entry_price = 101_000 * SCALAR_7;
    let position_id = open_limit_order_long(&fixture, &user, entry_price);

    // Try to liquidate pending position - should fail with specific error
    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3,
            position_id,
        },
    ];

    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(351)); // TradingError::ActionNotAllowedForStatus
}

// ==========================================
// Keeper Fee Tests
// ==========================================

#[test]
fn test_keeper_receives_fee() {
    // Create fixture with caller_take_rate set
    let mut fixture = TestFixture::create(false);

    // Mint to owner and deposit to vault
    fixture.token.mint(&fixture.owner, &(100_000_000 * SCALAR_7));
    fixture.vault.deposit(
        &(100_000_000 * SCALAR_7),
        &fixture.owner,
        &fixture.owner,
        &fixture.owner,
    );

    // Create market
    let base_config = trading::testutils::default_market(&fixture.env);
    let btc_config = trading::MarketConfig {
        asset: fixture.assets[AssetIndex::BTC as usize].clone(),
        ..base_config
    };
    fixture.create_market(&btc_config);

    // Update config to have caller_take_rate
    let new_config = trading::TradingConfig {
        oracle: fixture.oracle.address.clone(),
        caller_take_rate: 1_000_000, // 10%
        max_positions: 10,
        max_utilization: 0,
        max_price_age: 900,
    };
    fixture.trading.queue_set_config(&new_config);
    fixture.trading.set_config();

    fixture.trading.set_status(&0u32); // Active

    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open highly leveraged position for liquidation
    fixture.trading.open_position(
        &user,
        &(AssetIndex::BTC as u32),
        &(100 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &0,
    );

    let keeper_balance_before = fixture.token.balance(&keeper);

    // Price drops significantly
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,
        97_000_0000000, // BTC = 97K
        2000_0000000,
        0_1000000,
    ]);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3,
            position_id: 1,
        },
    ];

    fixture.trading.execute(&keeper, &requests);

    let keeper_balance_after = fixture.token.balance(&keeper);

    // Keeper should have received fee (caller_take_rate is 10% of fees)
    assert!(keeper_balance_after > keeper_balance_before);
}

// ==========================================
// Contract Status Tests
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #380)")]
fn test_execute_when_frozen() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let position_id = open_long_position(&fixture, &user);

    // Freeze contract
    fixture.trading.set_status(&2u32);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 3,
            position_id,
        },
    ];

    fixture.trading.execute(&keeper, &requests);
}

#[test]
fn test_execute_when_on_ice() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open position while active
    let position_id = open_long_position(&fixture, &user);

    fixture
        .trading
        .set_triggers(&position_id, &(110_000 * SCALAR_7), &0);

    // Set to OnIce
    fixture.trading.set_status(&1u32);

    // Price rises to TP
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,
        111_000_0000000,
        2000_0000000,
        0_1000000,
    ]);

    let requests = svec![
        &fixture.env,
        ExecuteRequest {
            request_type: 2,
            position_id,
        },
    ];

    // Should still work in OnIce mode
    let results = fixture.trading.execute(&keeper, &requests);
    assert_eq!(results.get(0), Some(0));

    assert!(!fixture.position_exists(position_id));
}
