use soroban_fixed_point_math::FixedPoint;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::default_market;
use trading::{PositionStatus, Request, RequestType};

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data(false)
}

#[test]
fn test_profitable_long_position_small_gain() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(150_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position at 100K
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &(2_000 * SCALAR_7), // 1000 tokens collateral, // 2x leverage
        &true,
        &0, // market order at 100K
    );

    let balance_after_open = fixture.token.balance(&user);
    let market = default_market();
    let open_fee = (1_000 * SCALAR_7)
        .fixed_mul_ceil(market.base_fee, SCALAR_7)
        .unwrap();
    assert_eq!(
        balance_after_open,
        initial_balance - (1_000 * SCALAR_7) - open_fee
    );

    // Price goes up 5% to 105K
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        105_000_0000000, // BTC = 105K (+5%)
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    let result = fixture.trading.submit(&user, &requests);

    // Verify position is closed
    let position = fixture.read_position(position_id);
    assert_eq!(position.status, PositionStatus::Closed);

    fixture.print_transfers(&result);
    // Calculate expected profit: 5% price increase * 2x leverage = 10% gain on collateral
    // Profit = 1000 * 0.10 = 100 tokens
    // User should receive: 1000 (collateral) + 100 (profit) = 1100 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance + (2_100 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_profitable_long_position_large_gain() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position at 100K with higher leverage
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &500,                // 5x leverage
        &true,
        &0, // market order at 100K
    );

    // Price goes up 20% to 120K
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        120_000_0000000, // BTC = 120K (+20%)
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // Calculate expected profit: 20% price increase * 5x leverage = 100% gain on collateral
    // Profit = 1000 * 1.0 = 1000 tokens
    // User should receive: 1000 (collateral) + 1000 (profit) = 2000 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (2_000 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_profitable_short_position_small_gain() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open short position at 2000 (ETH)
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &300,                // 3x leverage
        &false,              // short
        &0,                  // market order at 2000
    );

    // Price goes down 10% to 1800
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC
        1800_0000000,    // ETH = 1800 (-10%)
        0_1000000,       // XLM
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // Calculate expected profit: 10% price decrease * 3x leverage = 30% gain on collateral
    // Profit = 1000 * 0.30 = 300 tokens
    // User should receive: 1000 (collateral) + 300 (profit) = 1300 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (1_300 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_profitable_short_position_large_gain() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open short position at 2000 (ETH)
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(500 * SCALAR_7), // 500 tokens collateral
        &400,              // 4x leverage
        &false,            // short
        &0,                // market order at 2000
    );

    // Price crashes 25% to 1500
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC
        1500_0000000,    // ETH = 1500 (-25%)
        0_1000000,       // XLM
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // Calculate expected profit: 25% price decrease * 4x leverage = 100% gain on collateral
    // Profit = 500 * 1.0 = 500 tokens
    // User should receive: 500 (collateral) + 500 (profit) = 1000 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (500 * SCALAR_7) + (1_000 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_losing_long_position_small_loss() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position at 100K
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &200,                // 2x leverage
        &true,
        &0, // market order at 100K
    );

    // Price goes down 3% to 97K
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,      // USD
        97_000_0000000, // BTC = 97K (-3%)
        2000_0000000,   // ETH
        0_1000000,      // XLM
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // Calculate expected loss: 3% price decrease * 2x leverage = 6% loss on collateral
    // Loss = 1000 * 0.06 = 60 tokens
    // User should receive: 1000 (collateral) - 60 (loss) = 940 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (940 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_losing_long_position_large_loss() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position at 100K
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &400,                // 4x leverage
        &true,
        &0, // market order at 100K
    );

    // Price goes down 20% to 80K
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,      // USD
        80_000_0000000, // BTC = 80K (-20%)
        2000_0000000,   // ETH
        0_1000000,      // XLM
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // Calculate expected loss: 20% price decrease * 4x leverage = 80% loss on collateral
    // Loss = 1000 * 0.80 = 800 tokens
    // User should receive: 1000 (collateral) - 800 (loss) = 200 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (200 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_losing_short_position_small_loss() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open short position at 2000 (ETH)
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &250,                // 2.5x leverage
        &false,              // short
        &0,                  // market order at 2000
    );

    // Price goes up 4% to 2080
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC
        2080_0000000,    // ETH = 2080 (+4%)
        0_1000000,       // XLM
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // Calculate expected loss: 4% price increase * 2.5x leverage = 10% loss on collateral
    // Loss = 1000 * 0.10 = 100 tokens
    // User should receive: 1000 (collateral) - 100 (loss) = 900 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (900 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_losing_short_position_large_loss() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open short position at 2000 (ETH)
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &300,                // 3x leverage
        &false,              // short
        &0,                  // market order at 2000
    );

    // Price goes up 25% to 2500
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC
        2500_0000000,    // ETH = 2500 (+25%)
        0_1000000,       // XLM
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // Calculate expected loss: 25% price increase * 3x leverage = 75% loss on collateral
    // Loss = 1000 * 0.75 = 750 tokens
    // User should receive: 1000 (collateral) - 750 (loss) = 250 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (250 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_total_loss_scenario_long() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position at 100K with high leverage
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &400,                // 4x leverage
        &true,
        &0, // market order at 100K
    );

    // Price crashes 30% to 70K (loss exceeds collateral)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,      // USD
        70_000_0000000, // BTC = 70K (-30%)
        2000_0000000,   // ETH
        0_1000000,      // XLM
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // Calculate expected loss: 30% price decrease * 4x leverage = 120% loss on collateral
    // Loss = 1000 * 1.2 = 1200 tokens (exceeds collateral)
    // User should receive: 0 tokens (total loss)
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7); // Lost all collateral
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_total_loss_scenario_short() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open short position at 2000 (ETH) with high leverage
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::ETH],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &500,                // 5x leverage
        &false,              // short
        &0,                  // market order at 2000
    );

    // Price doubles to 4000 (+100%)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC
        4000_0000000,    // ETH = 4000 (+100%)
        0_1000000,       // XLM
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // Calculate expected loss: 100% price increase * 5x leverage = 500% loss on collateral
    // Loss far exceeds collateral
    // User should receive: 0 tokens (total loss)
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7); // Lost all collateral
    assert_eq!(final_balance, expected_balance);
}

#[test]
fn test_no_price_movement_break_even() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position at 100K
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &300,                // 3x leverage
        &true,
        &0, // market order at 100K
    );

    // Price stays the same (no oracle update needed)

    // Close position immediately
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // With no price movement and no fees, user should get exactly their collateral back
    let final_balance = fixture.token.balance(&user);
    assert_eq!(final_balance, initial_balance); // Break even
}

#[test]
fn test_different_leverage_same_price_movement() {
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);
    let user3 = Address::generate(&fixture.env);

    // Fund all users equally
    fixture.token.mint(&user1, &(100_000 * SCALAR_7));
    fixture.token.mint(&user2, &(100_000 * SCALAR_7));
    fixture.token.mint(&user3, &(100_000 * SCALAR_7));

    // All open same collateral but different leverage
    let pos1 = fixture.trading.create_position(
        &user1,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &200, // 2x leverage
        &true,
        &0,
    );

    let pos2 = fixture.trading.create_position(
        &user2,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &300, // 3x leverage
        &true,
        &0,
    );

    let pos3 = fixture.trading.create_position(
        &user3,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7),
        &500, // 5x leverage
        &true,
        &0,
    );

    // Same 10% price increase for all
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        110_000_0000000, // BTC = 110K (+10%)
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // Close all positions
    let requests1 = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: pos1,
            data: None
        }
    ];
    let requests2 = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: pos2,
            data: None
        }
    ];
    let requests3 = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: pos3,
            data: None
        }
    ];

    fixture.trading.submit(&user1, &requests1);
    fixture.trading.submit(&user2, &requests2);
    fixture.trading.submit(&user3, &requests3);

    // Check different profit amounts based on leverage
    let balance1 = fixture.token.balance(&user1);
    let balance2 = fixture.token.balance(&user2);
    let balance3 = fixture.token.balance(&user3);

    // 10% price * 2x leverage = 20% profit = 200 tokens profit
    assert_eq!(balance1, 100_000 * SCALAR_7 + 200 * SCALAR_7);

    // 10% price * 3x leverage = 30% profit = 300 tokens profit
    assert_eq!(balance2, 100_000 * SCALAR_7 + 300 * SCALAR_7);

    // 10% price * 5x leverage = 50% profit = 500 tokens profit
    assert_eq!(balance3, 100_000 * SCALAR_7 + 500 * SCALAR_7);
}

#[test]
fn test_xlm_position_pnl() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position on XLM at 0.1
    let position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::XLM],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &300,                // 3x leverage
        &true,
        &0, // market order at 0.1
    );

    // XLM price doubles to 0.2 (+100%)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC
        2000_0000000,    // ETH
        0_2000000,       // XLM = 0.2 (+100%)
    ]);

    // Close position
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: position_id,
            data: None,
        }
    ];

    fixture.trading.submit(&user, &requests);

    // Calculate expected profit: 100% price increase * 3x leverage = 300% gain on collateral
    // Profit = 1000 * 3.0 = 3000 tokens
    // User should receive: 1000 (collateral) + 3000 (profit) = 4000 tokens
    let final_balance = fixture.token.balance(&user);
    let expected_balance = initial_balance - (1_000 * SCALAR_7) + (4_000 * SCALAR_7);
    assert_eq!(final_balance, expected_balance);
}
