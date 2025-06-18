use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::{Request, RequestType, PositionStatus};

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data(false)
}

// ========== STOP LOSS TESTS ==========

#[test]
fn test_stop_loss_trigger_long_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position at 100K
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200, // 2x leverage
        &true,
        &0, // market order at 100K
    );

    // Set stop loss at 95K (5% below entry)
    let stop_loss = 95_000_0000000;
    fixture.trading.modify_risk(&position_id, &stop_loss, &0);

    // Verify stop loss was set
    let position = fixture.read_position(position_id);
    assert_eq!(position.stop_loss, stop_loss);
    assert_eq!(position.take_profit, 0);

    // Price drops to 95K, triggering stop loss
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        95_000_0000000,     // BTC = 95K (-5%)
        2000_0000000,       // ETH
        0_1000000,          // XLM
    ]);

    // Trigger stop loss
    let requests = svec![&fixture.env, Request {
        action: RequestType::StopLoss,
        position: position_id,
    }];

    fixture.trading.submit(&keeper, &requests);

    // Verify position was closed with StopLossClosed status
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::StopLossClosed);

    // Calculate expected loss: 5% price decrease * 2x leverage = 10% loss on collateral
    // Loss = 1000 * 0.10 = 100 tokens
    // User should receive: 1000 - 100 = 900 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (900 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_stop_loss_trigger_short_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open short position at 2000 (ETH)
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(1_000 * SCALAR_7),
        &300, // 3x leverage
        &false, // short
        &0, // market order at 2000
    );

    // Set stop loss at 2100 (5% above entry for short)
    let stop_loss = 2100_0000000;
    fixture.trading.modify_risk(&position_id, &stop_loss, &0);

    // Price rises to 2100, triggering stop loss
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        100_000_0000000,    // BTC
        2100_0000000,       // ETH = 2100 (+5%)
        0_1000000,          // XLM
    ]);

    // Trigger stop loss
    let requests = svec![&fixture.env, Request {
        action: RequestType::StopLoss,
        position: position_id,
    }];

    fixture.trading.submit(&keeper, &requests);

    // Verify position was closed with StopLossClosed status
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::StopLossClosed);

    // Calculate expected loss: 5% price increase * 3x leverage = 15% loss on collateral
    // Loss = 1000 * 0.15 = 150 tokens
    // User should receive: 1000 - 150 = 850 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (850 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_stop_loss_not_triggered_long() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open long position at 100K
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    // Set stop loss at 95K
    let stop_loss = 95_000_0000000;
    fixture.trading.modify_risk(&position_id, &stop_loss, &0);

    // Price only drops to 96K (above stop loss)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        96_000_0000000,     // BTC = 96K (above stop loss)
        2000_0000000,       // ETH
        0_1000000,          // XLM
    ]);

    // Try to trigger stop loss (should fail)
    let requests = svec![&fixture.env, Request {
        action: RequestType::StopLoss,
        position: position_id,
    }];

    fixture.trading.submit(&keeper, &requests); // Should panic
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_stop_loss_not_triggered_short() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open short position at 2000
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(1_000 * SCALAR_7),
        &300,
        &false,
        &0,
    );

    // Set stop loss at 2100
    let stop_loss = 2100_0000000;
    fixture.trading.modify_risk(&position_id, &stop_loss, &0);

    // Price only rises to 2050 (below stop loss trigger)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        100_000_0000000,    // BTC
        2050_0000000,       // ETH = 2050 (below stop loss)
        0_1000000,          // XLM
    ]);

    // Try to trigger stop loss (should fail)
    let requests = svec![&fixture.env, Request {
        action: RequestType::StopLoss,
        position: position_id,
    }];

    fixture.trading.submit(&keeper, &requests); // Should panic
}

// ========== TAKE PROFIT TESTS ==========

#[test]
fn test_take_profit_trigger_long_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position at 100K
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200, // 2x leverage
        &true,
        &0,
    );

    // Set take profit at 110K (10% above entry)
    let take_profit = 110_000_0000000;
    fixture.trading.modify_risk(&position_id, &0, &take_profit);

    // Price rises to 110K, triggering take profit
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        110_000_0000000,    // BTC = 110K (+10%)
        2000_0000000,       // ETH
        0_1000000,          // XLM
    ]);

    // Trigger take profit
    let requests = svec![&fixture.env, Request {
        action: RequestType::TakeProfit,
        position: position_id,
    }];

    fixture.trading.submit(&keeper, &requests);

    // Verify position was closed with TakeProfitClosed status
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::TakeProfitClosed);

    // Calculate expected profit: 10% price increase * 2x leverage = 20% profit on collateral
    // Profit = 1000 * 0.20 = 200 tokens
    // User should receive: 1000 + 200 = 1200 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (1_200 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_take_profit_trigger_short_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open short position at 2000
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(1_000 * SCALAR_7),
        &400, // 4x leverage
        &false,
        &0,
    );

    // Set take profit at 1800 (10% below entry for short)
    let take_profit = 1800_0000000;
    fixture.trading.modify_risk(&position_id, &0, &take_profit);

    // Price drops to 1800, triggering take profit
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        100_000_0000000,    // BTC
        1800_0000000,       // ETH = 1800 (-10%)
        0_1000000,          // XLM
    ]);

    // Trigger take profit
    let requests = svec![&fixture.env, Request {
        action: RequestType::TakeProfit,
        position: position_id,
    }];

    fixture.trading.submit(&keeper, &requests);

    // Verify position was closed with TakeProfitClosed status
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::TakeProfitClosed);

    // Calculate expected profit: 10% price decrease * 4x leverage = 40% profit on collateral
    // Profit = 1000 * 0.40 = 400 tokens
    // User should receive: 1000 + 400 = 1400 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (1_400 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_take_profit_not_triggered_long() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open long position at 100K
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    // Set take profit at 110K
    let take_profit = 110_000_0000000;
    fixture.trading.modify_risk(&position_id, &0, &take_profit);

    // Price only rises to 105K (below take profit)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        105_000_0000000,    // BTC = 105K (below take profit)
        2000_0000000,       // ETH
        0_1000000,          // XLM
    ]);

    // Try to trigger take profit (should fail)
    let requests = svec![&fixture.env, Request {
        action: RequestType::TakeProfit,
        position: position_id,
    }];

    fixture.trading.submit(&keeper, &requests); // Should panic
}

// ========== BOTH STOP LOSS AND TAKE PROFIT TESTS ==========

#[test]
fn test_set_both_stop_loss_and_take_profit() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open long position at 100K
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    // Set both stop loss and take profit
    let stop_loss = 95_000_0000000;    // 5% below
    let take_profit = 110_000_0000000; // 10% above
    fixture.trading.modify_risk(&position_id, &stop_loss, &take_profit);

    // Verify both were set
    let position = fixture.read_position(position_id);
    assert_eq!(position.stop_loss, stop_loss);
    assert_eq!(position.take_profit, take_profit);
}

#[test]
fn test_stop_loss_triggers_before_take_profit() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open long position
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    // Set both levels
    let stop_loss = 95_000_0000000;
    let take_profit = 110_000_0000000;
    fixture.trading.modify_risk(&position_id, &stop_loss, &take_profit);

    // Price drops to stop loss level
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        95_000_0000000,     // BTC = 95K (stop loss triggered)
        2000_0000000,       // ETH
        0_1000000,          // XLM
    ]);

    // Trigger stop loss
    let requests = svec![&fixture.env, Request {
        action: RequestType::StopLoss,
        position: position_id,
    }];

    fixture.trading.submit(&keeper, &requests);

    // Verify position closed with stop loss status
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::StopLossClosed);
}

#[test]
fn test_take_profit_triggers_before_stop_loss() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open long position
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    // Set both levels
    let stop_loss = 95_000_0000000;
    let take_profit = 105_000_0000000; // Closer target
    fixture.trading.modify_risk(&position_id, &stop_loss, &take_profit);

    // Price rises to take profit level
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        105_000_0000000,    // BTC = 105K (take profit triggered)
        2000_0000000,       // ETH
        0_1000000,          // XLM
    ]);

    // Trigger take profit
    let requests = svec![&fixture.env, Request {
        action: RequestType::TakeProfit,
        position: position_id,
    }];

    fixture.trading.submit(&keeper, &requests);

    // Verify position closed with take profit status
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::TakeProfitClosed);
}

// ========== LIQUIDATION TESTS ==========

#[test]
fn test_liquidation_underwater_long_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let liquidator = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open high leverage long position
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &1000, // 10x leverage (max)
        &true,
        &0,
    );

    // Massive price drop that makes position underwater
    // With 10x leverage, a 10% drop = 100% loss, making it liquidatable
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        90_000_0000000,     // BTC = 90K (-10%)
        2000_0000000,       // ETH
        0_1000000,          // XLM
    ]);

    // Liquidate the position
    let requests = svec![&fixture.env, Request {
        action: RequestType::Liquidation,
        position: position_id,
    }];

    let liquidator_fee = fixture.trading.submit(&liquidator, &requests);
    assert!(liquidator_fee >= 0); // Liquidator should receive fee

    // Verify position was liquidated
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::Liquidated);

    // User receives nothing (total loss due to liquidation)
    let final_balance = fixture.token.balance(&user);
    assert_eq!(final_balance, initial_balance - (1_000 * SCALAR_7));
}

#[test]
fn test_liquidation_underwater_short_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let liquidator = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open high leverage short position
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(1_000 * SCALAR_7),
        &1000, // 10x leverage
        &false, // short
        &0,
    );

    // Price pumps 10% (devastating for 10x short)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        100_000_0000000,    // BTC
        2200_0000000,       // ETH = 2200 (+10%)
        0_1000000,          // XLM
    ]);

    // Liquidate the position
    let requests = svec![&fixture.env, Request {
        action: RequestType::Liquidation,
        position: position_id,
    }];

    let liquidator_fee = fixture.trading.submit(&liquidator, &requests);
    assert!(liquidator_fee >= 0);

    // Verify position was liquidated
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::Liquidated);

    // User receives nothing
    let final_balance = fixture.token.balance(&user);
    assert_eq!(final_balance, initial_balance - (1_000 * SCALAR_7));
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_liquidation_healthy_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let liquidator = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open conservative position
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200, // Only 2x leverage
        &true,
        &0,
    );

    // Small price drop (not enough for liquidation)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        98_000_0000000,     // BTC = 98K (-2%)
        2000_0000000,       // ETH
        0_1000000,          // XLM
    ]);

    // Try to liquidate healthy position (should fail)
    let requests = svec![&fixture.env, Request {
        action: RequestType::Liquidation,
        position: position_id,
    }];

    fixture.trading.submit(&liquidator, &requests); // Should panic
}

// ========== RISK PARAMETER VALIDATION TESTS ==========

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_invalid_stop_loss_long_above_current_price() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user and open position
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0, // Opens at 100K
    );

    // Try to set stop loss above current price (invalid for long)
    let invalid_stop_loss = 105_000_0000000; // Above current 100K
    fixture.trading.modify_risk(&position_id, &invalid_stop_loss, &0); // Should panic
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_invalid_take_profit_long_below_current_price() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user and open position
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0, // Opens at 100K
    );

    // Try to set take profit below current price (invalid for long)
    let invalid_take_profit = 95_000_0000000; // Below current 100K
    fixture.trading.modify_risk(&position_id, &0, &invalid_take_profit); // Should panic
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_invalid_stop_loss_short_below_current_price() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user and open short position
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(1_000 * SCALAR_7),
        &200,
        &false, // short
        &0, // Opens at 2000
    );

    // Try to set stop loss below current price (invalid for short)
    let invalid_stop_loss = 1900_0000000; // Below current 2000
    fixture.trading.modify_risk(&position_id, &invalid_stop_loss, &0); // Should panic
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_invalid_take_profit_short_above_current_price() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user and open short position
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(1_000 * SCALAR_7),
        &200,
        &false, // short
        &0, // Opens at 2000
    );

    // Try to set take profit above current price (invalid for short)
    let invalid_take_profit = 2100_0000000; // Above current 2000
    fixture.trading.modify_risk(&position_id, &0, &invalid_take_profit); // Should panic
}

#[test]
fn test_remove_risk_parameters() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user and open position
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    // First set risk parameters
    let stop_loss = 95_000_0000000;
    let take_profit = 110_000_0000000;
    fixture.trading.modify_risk(&position_id, &stop_loss, &take_profit);

    // Verify they were set
    let position = fixture.read_position(position_id);
    assert_eq!(position.stop_loss, stop_loss);
    assert_eq!(position.take_profit, take_profit);

    // Remove both by setting to 0
    fixture.trading.modify_risk(&position_id, &0, &0);

    // Verify they were removed
    let position = fixture.read_position(position_id);
    assert_eq!(position.stop_loss, 0);
    assert_eq!(position.take_profit, 0);
}

#[test]
fn test_partial_risk_parameter_update() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user and open position
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    // Set only stop loss first
    let stop_loss = 95_000_0000000;
    fixture.trading.modify_risk(&position_id, &stop_loss, &0);

    let position = fixture.read_position(position_id);
    assert_eq!(position.stop_loss, stop_loss);
    assert_eq!(position.take_profit, 0);

    // Now set only take profit (should reset stop loss per current logic)
    let take_profit = 110_000_0000000;
    fixture.trading.modify_risk(&position_id, &0, &take_profit);

    let position = fixture.read_position(position_id);
    assert_eq!(position.stop_loss, 0); // Reset by passing 0
    assert_eq!(position.take_profit, take_profit);
}