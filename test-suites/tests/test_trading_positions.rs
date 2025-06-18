use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::{Request, RequestType, PositionStatus};

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data(false)
}

#[test]
fn test_open_market_position_long() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open a long market position (entry_price = 0 means market order)
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7), // 1000 token collateral
        &200, // 2x leverage
        &true, // is_long
        &0, // market order
    );

    // Verify position was created correctly
    let position = fixture.read_position(position_id);
    assert_eq!(position.user, user);
    assert_eq!(position.collateral, 1_000 * SCALAR_7);
    assert_eq!(position.leverage, 200);
    assert_eq!(position.is_long, true);
    assert_eq!(position.status, PositionStatus::Open);
    assert_eq!(position.entry_price, 100_000_0000000); // BTC price from oracle
    assert_eq!(position.id, position_id);

    // Check that user's token balance decreased
    let user_balance = fixture.token.balance(&user);
    assert_eq!(user_balance, 99_000 * SCALAR_7);

    // Check market data was updated for long position
    let market_data = fixture.read_market_data(fixture.assets[AssetIndex::BTC].clone());
    assert_eq!(market_data.long_collateral, 1_000 * SCALAR_7);
    assert_eq!(market_data.long_borrowed, 1_000 * SCALAR_7); // 2x leverage means borrowed = collateral
    assert_eq!(market_data.long_count, 1);
    assert_eq!(market_data.short_collateral, 0);
    assert_eq!(market_data.short_count, 0);
}

#[test]
fn test_open_market_position_short() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open a short market position
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(2_000 * SCALAR_7), // 2000 token collateral
        &300, // 3x leverage
        &false, // is_short
        &0, // market order
    );

    // Verify position was created correctly
    let position = fixture.read_position(position_id);
    assert_eq!(position.user, user);
    assert_eq!(position.collateral, 2_000 * SCALAR_7);
    assert_eq!(position.leverage, 300);
    assert_eq!(position.is_long, false);
    assert_eq!(position.status, PositionStatus::Open);
    assert_eq!(position.entry_price, 2000_0000000); // ETH price from oracle

    // Check market data was updated for short position
    let market_data = fixture.read_market_data(fixture.assets[AssetIndex::ETH].clone());
    assert_eq!(market_data.short_collateral, 2_000 * SCALAR_7);
    assert_eq!(market_data.short_borrowed, 4_000 * SCALAR_7); // 3x leverage: borrowed = 2 * collateral
    assert_eq!(market_data.short_count, 1);
    assert_eq!(market_data.long_collateral, 0);
    assert_eq!(market_data.long_count, 0);
}

#[test]
fn test_open_limit_position_long() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open a limit position below current price for long (should be pending)
    let entry_price = 99_000_0000000; // Below current BTC price of 100k
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200, // 2x leverage
        &true, // is_long
        &entry_price,
    );

    // Verify position is pending
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::Pending);
    assert_eq!(position.entry_price, entry_price);
    assert_eq!(position.user, user);
    assert_eq!(position.collateral, 1_000 * SCALAR_7);
    assert_eq!(position.is_long, true);

    // User's tokens should still be transferred (locked in contract)
    let user_balance = fixture.token.balance(&user);
    assert_eq!(user_balance, 99_000 * SCALAR_7);

    // Market data should NOT be updated for pending positions
    let market_data = fixture.read_market_data(fixture.assets[AssetIndex::BTC].clone());
    assert_eq!(market_data.long_collateral, 0);
    assert_eq!(market_data.long_count, 0);
    assert_eq!(market_data.short_collateral, 0);
    assert_eq!(market_data.short_count, 0);
}

#[test]
fn test_open_limit_position_short() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open a limit position above current price for short (should be pending)
    let entry_price = 2100_0000000; // Above current ETH price of 2000
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(500 * SCALAR_7),
        &400, // 4x leverage
        &false, // is_short
        &entry_price,
    );

    // Verify position is pending
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::Pending);
    assert_eq!(position.entry_price, entry_price);
    assert_eq!(position.user, user);
    assert_eq!(position.collateral, 500 * SCALAR_7);
    assert_eq!(position.is_long, false);

    // Market data should not be updated for pending positions
    let market_data = fixture.read_market_data(fixture.assets[AssetIndex::ETH].clone());
    assert_eq!(market_data.short_collateral, 0);
    assert_eq!(market_data.short_count, 0);
}

#[test]
fn test_fill_limit_order_long() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open limit position below current price
    let entry_price = 99_000_0000000;
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &entry_price,
    );

    // Verify position is pending
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::Pending);

    // Lower BTC price to trigger fill
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        99_000_0000000,     // BTC = 99K (at limit price)
        2000_0000000,       // ETH
        0_1000000,          // XLM
    ]);

    // Fill the order
    let requests = svec![&fixture.env, Request {
        action: RequestType::Fill,
        position: position_id,
    }];

    let caller_fee = fixture.trading.submit(&keeper, &requests);
    assert!(caller_fee >= 0); // Keeper should receive fee for filling

    // Verify position is now open
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::Open);
    assert_eq!(position.entry_price, 99_000_0000000); // Filled at market price

    // Market data should now be updated
    let market_data = fixture.read_market_data(fixture.assets[AssetIndex::BTC].clone());
    assert_eq!(market_data.long_collateral, 1_000 * SCALAR_7);
    assert_eq!(market_data.long_borrowed, 1_000 * SCALAR_7);
    assert_eq!(market_data.long_count, 1);
}

#[test]
fn test_fill_limit_order_short() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open limit position above current price for short
    let entry_price = 2100_0000000;
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(500 * SCALAR_7),
        &200,
        &false, // short
        &entry_price,
    );

    // Raise ETH price to trigger fill
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,          // USD
        100_000_0000000,    // BTC
        2100_0000000,       // ETH = 2.1K (at limit price)
        0_1000000,          // XLM
    ]);

    // Fill the order
    let requests = svec![&fixture.env, Request {
        action: RequestType::Fill,
        position: position_id,
    }];

    let caller_fee = fixture.trading.submit(&keeper, &requests);
    assert!(caller_fee >= 0);

    // Verify position is now open
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::Open);
    assert_eq!(position.entry_price, 2100_0000000);

    // Market data should be updated
    let market_data = fixture.read_market_data(fixture.assets[AssetIndex::ETH].clone());
    assert_eq!(market_data.short_collateral, 500 * SCALAR_7);
    assert_eq!(market_data.short_borrowed, 500 * SCALAR_7);
    assert_eq!(market_data.short_count, 1);
}

#[test]
fn test_close_position_manual() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);
    assert_eq!(initial_balance, 100_000 * SCALAR_7);

    // Open position
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200, // 2x leverage
        &true,
        &0, // market order
    );

    let balance_after_opening = fixture.token.balance(&user);
    assert_eq!(balance_after_opening, 99_000 * SCALAR_7); // 100k - 1k collateral

    // Close position manually
    let requests = svec![&fixture.env, Request {
        action: RequestType::Close,
        position: position_id,
    }];

    fixture.trading.submit(&user, &requests);

    // Verify position is closed
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::UserClosed);

    // User should receive exactly their collateral back (no fees, no price movement)
    let final_balance = fixture.token.balance(&user);
    assert_eq!(final_balance, 100_000 * SCALAR_7); // Should be back to original amount

    // Market data should be reset to zero since no positions remain
    let market_data = fixture.read_market_data(fixture.assets[AssetIndex::BTC].clone());
    assert_eq!(market_data.long_collateral, 0);
    assert_eq!(market_data.long_borrowed, 0);
    assert_eq!(market_data.long_count, 0);
    assert_eq!(market_data.short_collateral, 0);
    assert_eq!(market_data.short_borrowed, 0);
    assert_eq!(market_data.short_count, 0);
}
#[test]
fn test_cancel_pending_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open limit position (will be pending)
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &99_000_0000000, // Below current price
    );

    let initial_balance = fixture.token.balance(&user);

    // Cancel the position
    let requests = svec![&fixture.env, Request {
        action: RequestType::Cancel,
        position: position_id,
    }];

    fixture.trading.submit(&user, &requests);

    // Verify position is cancelled
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::Cancelled);

    // User should get their full collateral back
    let final_balance = fixture.token.balance(&user);
    assert_eq!(final_balance, initial_balance + (1_000 * SCALAR_7));
}

#[test]
fn test_modify_risk_parameters() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open position
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    // Verify initial risk parameters are zero
    let position = fixture.read_position(position_id);
    assert_eq!(position.stop_loss, 0);
    assert_eq!(position.take_profit, 0);

    // Set stop loss and take profit
    let stop_loss = 95_000_0000000; // 5% below entry
    let take_profit = 105_000_0000000; // 5% above entry

    fixture.trading.modify_risk(&position_id, &stop_loss, &take_profit);

    // Verify risk parameters were set
    let position = fixture.read_position(position_id);
    assert_eq!(position.stop_loss, stop_loss);
    assert_eq!(position.take_profit, take_profit);
}

#[test]
fn test_modify_risk_parameters_partial() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open position
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    // Set only stop loss
    let stop_loss = 95_000_0000000;
    fixture.trading.modify_risk(&position_id, &stop_loss, &0);

    let position = fixture.read_position(position_id);
    assert_eq!(position.stop_loss, stop_loss);
    assert_eq!(position.take_profit, 0);

    // Set only take profit
    let take_profit = 105_000_0000000;
    fixture.trading.modify_risk(&position_id, &0, &take_profit);

    let position = fixture.read_position(position_id);
    assert_eq!(position.stop_loss, 0); // Should be reset to 0 as per contract logic
    assert_eq!(position.take_profit, take_profit);
}

#[test]
fn test_multiple_positions_same_user() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user generously
    fixture.token.mint(&user, &(1_000_000 * SCALAR_7));

    // Open multiple positions on different assets
    let btc_position = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    let eth_position = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(500 * SCALAR_7),
        &300, // 3x leverage
        &false, // short
        &0,
    );

    let xlm_position = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::XLM],
        &(100 * SCALAR_7),
        &150, // 1.5x leverage
        &true,
        &0,
    );

    // Verify all positions were created with correct properties
    let btc_pos = fixture.read_position(btc_position);
    let eth_pos = fixture.read_position(eth_position);
    let xlm_pos = fixture.read_position(xlm_position);

    assert_eq!(btc_pos.collateral, 1_000 * SCALAR_7);
    assert_eq!(eth_pos.collateral, 500 * SCALAR_7);
    assert_eq!(xlm_pos.collateral, 100 * SCALAR_7);

    assert_eq!(btc_pos.is_long, true);
    assert_eq!(eth_pos.is_long, false);
    assert_eq!(xlm_pos.is_long, true);

    assert_eq!(btc_pos.status, PositionStatus::Open);
    assert_eq!(eth_pos.status, PositionStatus::Open);
    assert_eq!(xlm_pos.status, PositionStatus::Open);

    // Check market data for each asset
    let btc_market = fixture.read_market_data(fixture.assets[AssetIndex::BTC].clone());
    let eth_market = fixture.read_market_data(fixture.assets[AssetIndex::ETH].clone());
    let xlm_market = fixture.read_market_data(fixture.assets[AssetIndex::XLM].clone());

    assert_eq!(btc_market.long_count, 1);
    assert_eq!(eth_market.short_count, 1);
    assert_eq!(xlm_market.long_count, 1);
}

#[test]
fn test_multiple_positions_different_users() {
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);
    let user3 = Address::generate(&fixture.env);

    // Fund all users
    fixture.token.mint(&user1, &(100_000 * SCALAR_7));
    fixture.token.mint(&user2, &(100_000 * SCALAR_7));
    fixture.token.mint(&user3, &(100_000 * SCALAR_7));

    // Each user opens a position on BTC
    let pos1 = fixture.trading.open_position(
        &user1,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &0,
    );

    let pos2 = fixture.trading.open_position(
        &user2,
        &fixture.assets[AssetIndex::BTC],
        &(2_000 * SCALAR_7),
        &150,
        &false, // short
        &0,
    );

    let pos3 = fixture.trading.open_position(
        &user3,
        &fixture.assets[AssetIndex::BTC],
        &(500 * SCALAR_7),
        &300,
        &true,
        &0,
    );

    // Verify positions belong to correct users
    let position1 = fixture.read_position(pos1);
    let position2 = fixture.read_position(pos2);
    let position3 = fixture.read_position(pos3);

    assert_eq!(position1.user, user1);
    assert_eq!(position2.user, user2);
    assert_eq!(position3.user, user3);

    assert_eq!(position1.is_long, true);
    assert_eq!(position2.is_long, false);
    assert_eq!(position3.is_long, true);

    // Check aggregated market data
    let market_data = fixture.read_market_data(fixture.assets[AssetIndex::BTC].clone());
    assert_eq!(market_data.long_count, 2); // user1 and user3
    assert_eq!(market_data.short_count, 1); // user2
    assert_eq!(market_data.long_collateral, 1_500 * SCALAR_7); // 1000 + 500
    assert_eq!(market_data.short_collateral, 2_000 * SCALAR_7); // user2's position
}

// Error condition tests

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_invalid_leverage_too_high() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Try to open position with leverage over max (1000 = 10x is max)
    fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &1500, // 15x leverage, over max of 10x
        &true,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_invalid_leverage_too_low() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Try to open position with leverage below minimum (should be > 100)
    fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &50, // 0.5x leverage, below minimum
        &true,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_invalid_collateral_too_small() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    fixture.token.mint(&user, &(5 * SCALAR_7)); // Only 5 tokens

    // Try to open position with collateral below minimum (10 tokens is minimum)
    fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(5 * SCALAR_7), // Below minimum of 10 tokens
        &200,
        &true,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_invalid_collateral_too_large() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user with massive amount
    let huge_amount = 200_000_000_0000000; // 200M tokens
    fixture.token.mint(&user, &huge_amount);

    // Try to open position with collateral above maximum
    fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(150_000_000_0000000), // 150M tokens, above max of 100M
        &200,
        &true,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #68)")] // MaxPositions
fn test_max_positions_limit() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user generously
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    // First, let's verify the max_positions config
    let config = fixture.read_config();
    println!("Max positions configured: {}", config.max_positions);

    // Create exactly max_positions (should succeed)
    for i in 0..config.max_positions {
        println!("Creating position {}", i);

        // Use very low entry prices to ensure they become pending orders
        // BTC current price is 100K, so using 50K will make it pending for longs
        let entry_price = 50_000_0000000i128; // 50K USD - way below current BTC price

        let position_id = fixture.trading.open_position(
            &user,
            &fixture.assets[AssetIndex::BTC],
            &(100 * SCALAR_7),
            &200,
            &true, // long position
            &entry_price, // This will create a pending order since 50K < 100K
        );

        println!("Created position {} with ID {}", i, position_id);

        // Verify the position was created and is pending
        let position = fixture.read_position(position_id);
        assert_eq!(position.status, PositionStatus::Pending);
    }

    println!("Successfully created {} positions", config.max_positions);

    // Now try to create one more (this should panic)
    println!("Attempting to create position beyond limit...");
    let new_pos = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(100 * SCALAR_7),
        &200,
        &true,
        &50_000_0000000i128, // Same low entry price
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // BadRequest
fn test_fill_order_wrong_price_direction() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open long limit order below current price
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200,
        &true,
        &99_000_0000000, // Below current price
    );

    // Try to fill without price movement (price is still above limit)
    let requests = svec![&fixture.env, Request {
        action: RequestType::Fill,
        position: position_id,
    }];

    fixture.trading.submit(&keeper, &requests); // Should panic - price conditions not met
}