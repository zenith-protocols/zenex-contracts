use soroban_fixed_point_math::FixedPoint;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::default_market;
use trading::{PositionStatus, Request, RequestType};

const SECONDS_IN_WEEK: u64 = 604800; // 7 days in seconds
const SECONDS_IN_HOUR: u64 = 3600; // 1 hour in seconds

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
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &(2_000 * SCALAR_7), // 1000 tokens collateral, // 2x leverage
        &true,
        &0, // market order at 100K
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    let balance_after_open = fixture.token.balance(&user);
    let market = default_market();
    let base_fee = (1_000 * SCALAR_7)
        .fixed_mul_ceil(market.base_fee, SCALAR_7)
        .unwrap();
    let price_impact = (2_000 * SCALAR_7)
        .fixed_div_ceil(market.price_impact_scalar, SCALAR_7)
        .unwrap();

    assert_eq!(
        balance_after_open,
        initial_balance - (1_000 * SCALAR_7) - base_fee - price_impact
    );

    fixture.jump(SECONDS_IN_HOUR);

    // Price goes up 5% to 105K
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        10000000,        // USD
        105_000_0000000, // BTC = 105K (+5%)
        2000_0000000,    // ETH
        1000000,         // XLM
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
    let expected_profit = (100 * SCALAR_7) - base_fee - base_fee - price_impact - price_impact; // 10% gain on collateral

    assert_eq!(final_balance, initial_balance + expected_profit);
}

#[test]
fn test_long_short_week_5pct_move_print_balances() {
    // This test tests whether the dominant side pays interest while the short side receives, and whether the dominant side pays a base fee and the other side doesn't.
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);

    // Fund users sufficiently for collateral and fees
    fixture.token.mint(&user1, &(500_000 * SCALAR_7));
    fixture.token.mint(&user2, &(500_000 * SCALAR_7));

    // Record initial balances
    let initial_user1_balance = fixture.token.balance(&user1);
    let initial_user2_balance = fixture.token.balance(&user2);
    let initial_vault_balance = fixture.token.balance(&fixture.vault.address);

    // Set BTC price to 100K before opening positions
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        10000000,        // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        1000000,         // XLM
    ]);

    // User1 opens a long position at 100k with 2x leverage
    // Choose collateral = 25k -> notional size = 50k (2x)
    let user1_position_id = fixture.trading.open_position(
        &user1,
        &fixture.assets[AssetIndex::BTC],
        &(25_000 * SCALAR_7),
        &(50_000 * SCALAR_7),
        &true,
        &0, // market order at current price (100K)
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // User2 opens a short position of 100k notional with 10x leverage -> collateral = 10k
    let user2_position_id = fixture.trading.open_position(
        &user2,
        &fixture.assets[AssetIndex::BTC],
        &(10_000 * SCALAR_7),
        &(100_000 * SCALAR_7),
        &false,
        &0, // market order at current price (100K)
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // A week passes
    fixture.jump(604800);

    // Price goes up 5%: 100K -> 105K
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        10000000,        // USD
        105_000_0000000, // BTC = 105K (+5%)
        2000_0000000,    // ETH
        1000000,         // XLM
    ]);

    // User1 closes the long
    let result1 = fixture.trading.submit(
        &user1,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user1_position_id,
                data: None
            }
        ],
    );

    // User2 closes the short
    let result2 = fixture.trading.submit(
        &user2,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user2_position_id,
                data: None
            }
        ],
    );

    // Ensure positions are closed
    assert_eq!(
        fixture.read_position(user1_position_id).status,
        PositionStatus::Closed
    );
    assert_eq!(
        fixture.read_position(user2_position_id).status,
        PositionStatus::Closed
    );

    // Print transfers for visibility
    fixture.print_transfers(&result1);
    fixture.print_transfers(&result2);

    // Print final balances of User1, User2 and the Vault (token balances)
    let user1_balance = fixture.token.balance(&user1);
    let user2_balance = fixture.token.balance(&user2);
    let vault_balance = fixture.token.balance(&fixture.vault.address);
    let contract_balance = fixture.token.balance(&fixture.trading.address);

    let delta_user1 = user1_balance - initial_user1_balance;
    // delta user1 should be 2500 (5% of 50k) minus the fee. The fee should be -243.8 = 25 + 0.0000125 - 268.8
    let delta_user2 = user2_balance - initial_user2_balance;
    // delta user2 should be minus 5k (5% of 100k) minus the fee. Fee should be 386 = 50 + 0.000025 + 336
    let delta_vault = vault_balance - initial_vault_balance;
    // delta vault should be the fees minus the sum of the pnl of user1 and user2. The vault should receive 2.5k pnl and 142.2 fees (total 2642.2).

    // Assert user deltas are close to the expected values.
    let expected_delta_user1 = (27438 * (SCALAR_7 / 10)); // 2718.8 tokens
    let expected_delta_user2 = -5386 * SCALAR_7;
    let expected_delta_vault = (26422 * (SCALAR_7 / 10)); // 2642.2 tokens
    let tolerance_user1 = 5 * (SCALAR_7 / 10); // 0.5 tokens
    let tolerance_user2 = 5 * SCALAR_7; // 5 tokens
    let tolerance_vault = 5 * (SCALAR_7 / 10); // 0.5 tokens

    let delta_user1_diff = (delta_user1 - expected_delta_user1).abs();
    let delta_user2_diff = (delta_user2 - expected_delta_user2).abs();

    assert!(
        delta_user1_diff <= tolerance_user1,
        "delta_user1 off by {:.7} tokens (expected {:.7}, got {:.7})",
        delta_user1_diff as f64 / SCALAR_7 as f64,
        expected_delta_user1 as f64 / SCALAR_7 as f64,
        delta_user1 as f64 / SCALAR_7 as f64
    );

    assert!(
        delta_user2_diff <= tolerance_user2,
        "delta_user2 off by {:.7} tokens (expected {:.7}, got {:.7})",
        delta_user2_diff as f64 / SCALAR_7 as f64,
        expected_delta_user2 as f64 / SCALAR_7 as f64,
        delta_user2 as f64 / SCALAR_7 as f64
    );

    let delta_vault_diff = (delta_vault - expected_delta_vault).abs();
    assert!(
        delta_vault_diff <= tolerance_vault,
        "delta_vault off by {:.7} tokens (expected {:.7}, got {:.7})",
        delta_vault_diff as f64 / SCALAR_7 as f64,
        expected_delta_vault as f64 / SCALAR_7 as f64,
        delta_vault as f64 / SCALAR_7 as f64
    );

    println!(
        "Contract balance (should be 0): {:.7}",
        contract_balance as f64 / SCALAR_7 as f64
    );

    println!(
        "User1 balance: {:.7} (Δ {:.7})\nUser2 balance: {:.7} (Δ {:.7})\nVault balance: {:.7} (Δ {:.7})",
        user1_balance as f64 / SCALAR_7 as f64,
        delta_user1 as f64 / SCALAR_7 as f64,
        user2_balance as f64 / SCALAR_7 as f64,
        delta_user2 as f64 / SCALAR_7 as f64,
        vault_balance as f64 / SCALAR_7 as f64,
        delta_vault as f64 / SCALAR_7 as f64
    );
    
    
    assert_eq!(contract_balance, 0);
}

#[test]
fn test_equal_short_long_notional() {
    // This test tests whether when long/short notional are equal, both sides pay a base fee and the base hourly rate. 
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env); // short
    let user2 = Address::generate(&fixture.env); // long

    // Fund users sufficiently for collateral, fees and potential PnL
    fixture.token.mint(&user1, &(1_000_000 * SCALAR_7));
    fixture.token.mint(&user2, &(1_000_000 * SCALAR_7));

    // Record initial balances
    let initial_user1_balance = fixture.token.balance(&user1);
    let initial_user2_balance = fixture.token.balance(&user2);
    let initial_vault_balance = fixture.token.balance(&fixture.vault.address);

    // Ensure BTC price is 100K before opening positions
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // Both target 200k notional. Choose ample collateral to avoid liquidation on a 10% move.
    // Collateral = 50k, Notional = 200k (4x leverage)
    let collateral = 50_000 * SCALAR_7;
    let notional = 200_000 * SCALAR_7;

    // User1 opens a SHORT at 100k
    let user1_position_id = fixture.trading.open_position(
        &user1,
        &fixture.assets[AssetIndex::BTC],
        &collateral,
        &notional,
        &false,
        &0,
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // User2 opens a LONG at 100k
    let user2_position_id = fixture.trading.open_position(
        &user2,
        &fixture.assets[AssetIndex::BTC],
        &collateral,
        &notional,
        &true,
        &0,
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // A week passes
    fixture.jump(604800);

    // Price drops 10%: 100K -> 90K
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,      // USD
        90_000_0000000, // BTC = 90K (-10%)
        2000_0000000,   // ETH
        0_1000000,      // XLM
    ]);

    // Close both positions
    let result_short = fixture.trading.submit(
        &user1,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user1_position_id,
                data: None
            }
        ],
    );

    let result_long = fixture.trading.submit(
        &user2,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user2_position_id,
                data: None
            }
        ],
    );

    // Ensure positions are closed
    assert_eq!(
        fixture.read_position(user1_position_id).status,
        PositionStatus::Closed
    );
    assert_eq!(
        fixture.read_position(user2_position_id).status,
        PositionStatus::Closed
    );

    // Print transfers for visibility
    fixture.print_transfers(&result_short);
    fixture.print_transfers(&result_long);

    // Print final balances of User1, User2 and the Vault (token balances) and deltas
    let user1_balance = fixture.token.balance(&user1);
    let user2_balance = fixture.token.balance(&user2);
    let vault_balance = fixture.token.balance(&fixture.vault.address);

    
    let delta_user1 = user1_balance - initial_user1_balance;
    // This should be pnl minus the fee. Pnl is 20k. Fee should be 536 = 200 + 336
    let delta_user2 = user2_balance - initial_user2_balance;
    // This should be pnl minus the fee. Pnl is -20k. Fee should be 436 = 100 + 336
    let delta_vault = vault_balance - initial_vault_balance;
    // Total user pnl is 0, so this is only the fees: 536 + 436 = 972

    // Assert user deltas are close to expected values.
    let expected_delta_user1 = 19_464 * SCALAR_7;
    let expected_delta_user2 = -20_436 * SCALAR_7;
    let tolerance = SCALAR_7 / 2; // 0.5 tokens

    let delta_user1_diff = (delta_user1 - expected_delta_user1).abs();
    let delta_user2_diff = (delta_user2 - expected_delta_user2).abs();

    assert!(
        delta_user1_diff <= tolerance,
        "delta_user1 off by {:.7} tokens (expected {:.7}, got {:.7})",
        delta_user1_diff as f64 / SCALAR_7 as f64,
        expected_delta_user1 as f64 / SCALAR_7 as f64,
        delta_user1 as f64 / SCALAR_7 as f64
    );

    assert!(
        delta_user2_diff <= tolerance,
        "delta_user2 off by {:.7} tokens (expected {:.7}, got {:.7})",
        delta_user2_diff as f64 / SCALAR_7 as f64,
        expected_delta_user2 as f64 / SCALAR_7 as f64,
        delta_user2 as f64 / SCALAR_7 as f64
    );

    println!(
        "User1 (short) balance: {:.7} (Δ {:.7})\nUser2 (long) balance: {:.7} (Δ {:.7})\nVault balance: {:.7} (Δ {:.7})",
        user1_balance as f64 / SCALAR_7 as f64, delta_user1 as f64 / SCALAR_7 as f64,
        user2_balance as f64 / SCALAR_7 as f64, delta_user2 as f64 / SCALAR_7 as f64,
        vault_balance as f64 / SCALAR_7 as f64, delta_vault as f64 / SCALAR_7 as f64
    );
}

#[test]
fn test_changing_long_short_ratio() {
    // This test tests whether the interest accrues as it should through a changing of dominant sides.
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env); // long 100k
    let user2 = Address::generate(&fixture.env); // short 50k
    let user3 = Address::generate(&fixture.env); // short 100k

    // Fund users sufficiently for collateral, fees and funding
    fixture.token.mint(&user1, &(1_000_000 * SCALAR_7));
    fixture.token.mint(&user2, &(1_000_000 * SCALAR_7));
    fixture.token.mint(&user3, &(1_000_000 * SCALAR_7));

    // Record initial balances
    let initial_user1_balance = fixture.token.balance(&user1);
    let initial_user2_balance = fixture.token.balance(&user2);
    let initial_user3_balance = fixture.token.balance(&user3);
    let initial_vault_balance = fixture.token.balance(&fixture.vault.address);

    // Ensure BTC price is 100K before opening positions
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // Notionals per scenario
    let notional_100k = 100_000 * SCALAR_7;
    let notional_50k = 50_000 * SCALAR_7;

    // Choose reasonable collateral to avoid liquidation purely from funding/fees
    let collateral_100k = 20_000 * SCALAR_7; // 5x leverage
    let collateral_50k = 10_000 * SCALAR_7; // 5x leverage

    // User1 opens LONG 100k
    let user1_position_id = fixture.trading.open_position(
        &user1,
        &fixture.assets[AssetIndex::BTC],
        &collateral_100k,
        &notional_100k,
        &true,
        &0,
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // User2 opens SHORT 50k
    let user2_position_id = fixture.trading.open_position(
        &user2,
        &fixture.assets[AssetIndex::BTC],
        &collateral_50k,
        &notional_50k,
        &false,
        &0,
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Price does not move (stay at 100k)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // User3 opens SHORT 100k
    let user3_position_id = fixture.trading.open_position(
        &user3,
        &fixture.assets[AssetIndex::BTC],
        &collateral_100k,
        &notional_100k,
        &false,
        &0,
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // Another week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Price still unchanged at 100k
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // Close all positions
    let result_user1 = fixture.trading.submit(
        &user1,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user1_position_id,
                data: None
            }
        ],
    );

    let result_user2 = fixture.trading.submit(
        &user2,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user2_position_id,
                data: None
            }
        ],
    );

    let result_user3 = fixture.trading.submit(
        &user3,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user3_position_id,
                data: None
            }
        ],
    );

    // Ensure positions are closed
    assert_eq!(
        fixture.read_position(user1_position_id).status,
        PositionStatus::Closed
    );
    assert_eq!(
        fixture.read_position(user2_position_id).status,
        PositionStatus::Closed
    );
    assert_eq!(
        fixture.read_position(user3_position_id).status,
        PositionStatus::Closed
    );

    // Print transfers for visibility
    fixture.print_transfers(&result_user1);
    fixture.print_transfers(&result_user2);
    fixture.print_transfers(&result_user3);

 

    // Balances and deltas
    let user1_balance = fixture.token.balance(&user1);
    let user2_balance = fixture.token.balance(&user2);
    let user3_balance = fixture.token.balance(&user3);
    let vault_balance = fixture.token.balance(&fixture.vault.address);

    let delta_user1 = user1_balance - initial_user1_balance;
    // This should be the fee: -83.6 = -50 - 336 + 302.4
    let delta_user2 = user2_balance - initial_user2_balance;
    // This should be the fee: 117.8 = -25 + 268.8 - 126
    let delta_user3 = user3_balance - initial_user3_balance;
    // This should be the fee: -352.4 = -50 - 302.4
    // User3 closes short when short is dominant (100k > 0), so should pay base fee on close.
    let delta_vault = vault_balance - initial_vault_balance;
    // This should be the total of the fees

    println!(
		"User1 (long 100k) balance: {:.7} (Δ {:.7})\nUser2 (short 50k) balance: {:.7} (Δ {:.7})\nUser3 (short 100k) balance: {:.7} (Δ {:.7})\nVault balance: {:.7} (Δ {:.7})",
		user1_balance as f64 / SCALAR_7 as f64, delta_user1 as f64 / SCALAR_7 as f64,
		user2_balance as f64 / SCALAR_7 as f64, delta_user2 as f64 / SCALAR_7 as f64,
		user3_balance as f64 / SCALAR_7 as f64, delta_user3 as f64 / SCALAR_7 as f64,
		vault_balance as f64 / SCALAR_7 as f64, delta_vault as f64 / SCALAR_7 as f64
	);

    // Assert user deltas are close to the expected values
    let expected_delta_user1 = -(836 * (SCALAR_7 / 10)); // -83.6 tokens
    let expected_delta_user2 = 1178 * (SCALAR_7 / 10); // 117.8 tokens
    let expected_delta_user3 = -(3524 * (SCALAR_7 / 10)); // -352.4 tokens
    let tolerance = 5 * (SCALAR_7 / 10); // 0.5 tokens

    let delta_user1_diff = (delta_user1 - expected_delta_user1).abs();
    assert!(
        delta_user1_diff <= tolerance,
        "delta_user1 ({:.7}) is not approximately -83.6 (difference: {:.7})",
        delta_user1 as f64 / SCALAR_7 as f64,
        delta_user1_diff as f64 / SCALAR_7 as f64
    );

    let delta_user2_diff = (delta_user2 - expected_delta_user2).abs();
    assert!(
        delta_user2_diff <= tolerance,
        "delta_user2 ({:.7}) is not approximately 117.8 (difference: {:.7})",
        delta_user2 as f64 / SCALAR_7 as f64,
        delta_user2_diff as f64 / SCALAR_7 as f64
    );

    let delta_user3_diff = (delta_user3 - expected_delta_user3).abs();
    assert!(
        delta_user3_diff <= tolerance,
        "delta_user3 ({:.7}) is not approximately -352.4 (difference: {:.7})",
        delta_user3 as f64 / SCALAR_7 as f64,
        delta_user3_diff as f64 / SCALAR_7 as f64
    );
}

#[test]
fn test_long_then_short_sequential_weeks() {
    // This test tests whether user1 pays a base fee and the base hourly rate, while there is no other side. 
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);

    // Fund users sufficiently for collateral, fees and funding
    fixture.token.mint(&user1, &(500_000 * SCALAR_7));
    fixture.token.mint(&user2, &(500_000 * SCALAR_7));

    // Record initial balances
    let initial_user1_balance = fixture.token.balance(&user1);
    let initial_user2_balance = fixture.token.balance(&user2);
    let initial_vault_balance = fixture.token.balance(&fixture.vault.address);

    // Ensure BTC price is 100K before opening positions
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // User1 opens a long position with 50k collateral and 2x leverage (100k notional)
    let user1_position_id = fixture.trading.open_position(
        &user1,
        &fixture.assets[AssetIndex::BTC],
        &(50_000 * SCALAR_7), // 50k collateral
        &(100_000 * SCALAR_7), // 100k notional (2x leverage)
        &true, // long
        &0, // market order
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Ensure price is still at 100K before User2 opens position
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // User2 opens a short position with 50k collateral and 2x leverage (100k notional)
    let user2_position_id = fixture.trading.open_position(
        &user2,
        &fixture.assets[AssetIndex::BTC],
        &(50_000 * SCALAR_7), // 50k collateral
        &(100_000 * SCALAR_7), // 100k notional (2x leverage)
        &false, // short
        &0, // market order
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // Another week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Ensure price is still at 100K (no price movement)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // User1 closes the long position
    let result1 = fixture.trading.submit(
        &user1,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user1_position_id,
                data: None
            }
        ],
    );

    // User2 closes the short position
    let result2 = fixture.trading.submit(
        &user2,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user2_position_id,
                data: None
            }
        ],
    );

    // Ensure positions are closed
    assert_eq!(
        fixture.read_position(user1_position_id).status,
        PositionStatus::Closed
    );
    assert_eq!(
        fixture.read_position(user2_position_id).status,
        PositionStatus::Closed
    );

    // Print transfers for visibility
    fixture.print_transfers(&result1);
    fixture.print_transfers(&result2);

    // Calculate deltas
    let user1_balance = fixture.token.balance(&user1);
    let user2_balance = fixture.token.balance(&user2);
    let vault_balance = fixture.token.balance(&fixture.vault.address);

    let delta_user1 = user1_balance - initial_user1_balance;
    let delta_user2 = user2_balance - initial_user2_balance;
    let delta_vault = vault_balance - initial_vault_balance;

    // Assert user deltas are close to the expected values
    let expected_delta_user1 = -(436 * SCALAR_7); // -436 tokens = -100 - 336
    let expected_delta_user2 = -(218 * SCALAR_7); // -218 tokens = -50 - 168
    let tolerance = 5 * (SCALAR_7 / 10); // 0.5 tokens

    let delta_user1_diff = (delta_user1 - expected_delta_user1).abs();
    assert!(
        delta_user1_diff <= tolerance,
        "delta_user1 ({:.7}) is not approximately -436 (difference: {:.7})",
        delta_user1 as f64 / SCALAR_7 as f64,
        delta_user1_diff as f64 / SCALAR_7 as f64
    );

    let delta_user2_diff = (delta_user2 - expected_delta_user2).abs();
    assert!(
        delta_user2_diff <= tolerance,
        "delta_user2 ({:.7}) is not approximately -218 (difference: {:.7})",
        delta_user2 as f64 / SCALAR_7 as f64,
        delta_user2_diff as f64 / SCALAR_7 as f64
    );

    println!(
        "User1 (long) balance: {:.7} (Δ {:.7})\nUser2 (short) balance: {:.7} (Δ {:.7})\nVault balance: {:.7} (Δ {:.7})",
        user1_balance as f64 / SCALAR_7 as f64,
        delta_user1 as f64 / SCALAR_7 as f64,
        user2_balance as f64 / SCALAR_7 as f64,
        delta_user2 as f64 / SCALAR_7 as f64,
        vault_balance as f64 / SCALAR_7 as f64,
        delta_vault as f64 / SCALAR_7 as f64
    );
}

#[test]
fn test_long_short_sequential_closes() {
    // This test tests whether user2 still pays interest and base fee after the other side is closed. 
    let fixture = setup_fixture();
    let user1 = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);

    // Fund users sufficiently for collateral, fees and funding
    fixture.token.mint(&user1, &(500_000 * SCALAR_7));
    fixture.token.mint(&user2, &(500_000 * SCALAR_7));

    // Record initial balances
    let initial_user1_balance = fixture.token.balance(&user1);
    let initial_user2_balance = fixture.token.balance(&user2);
    let initial_vault_balance = fixture.token.balance(&fixture.vault.address);

    // Ensure BTC price is 100K before opening positions
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // User1 opens a long position with 10k collateral and 10x leverage (100k notional)
    let user1_position_id = fixture.trading.open_position(
        &user1,
        &fixture.assets[AssetIndex::BTC],
        &(10_000 * SCALAR_7), // 10k collateral
        &(100_000 * SCALAR_7), // 100k notional (10x leverage)
        &true, // long
        &0, // market order
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // User2 opens a short position with 10k collateral and 10x leverage (100k notional)
    let user2_position_id = fixture.trading.open_position(
        &user2,
        &fixture.assets[AssetIndex::BTC],
        &(10_000 * SCALAR_7), // 10k collateral
        &(100_000 * SCALAR_7), // 100k notional (10x leverage)
        &false, // short
        &0, // market order
        &0, // take profit: 0 (not set)
        &0, // stop loss: 0 (not set)
    );

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Ensure price is still at 100K (no price movement)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // User1 closes the long position
    let result1 = fixture.trading.submit(
        &user1,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user1_position_id,
                data: None
            }
        ],
    );

    // Another week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Ensure price is still at 100K (no price movement)
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // User2 closes the short position
    let result2 = fixture.trading.submit(
        &user2,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Close,
                position: user2_position_id,
                data: None
            }
        ],
    );

    // Ensure positions are closed
    assert_eq!(
        fixture.read_position(user1_position_id).status,
        PositionStatus::Closed
    );
    assert_eq!(
        fixture.read_position(user2_position_id).status,
        PositionStatus::Closed
    );

    // Print transfers for visibility
    fixture.print_transfers(&result1);
    fixture.print_transfers(&result2);

    // Calculate deltas
    let user1_balance = fixture.token.balance(&user1);
    let user2_balance = fixture.token.balance(&user2);
    let vault_balance = fixture.token.balance(&fixture.vault.address);

    let delta_user1 = user1_balance - initial_user1_balance;
    let delta_user2 = user2_balance - initial_user2_balance;
    let delta_vault = vault_balance - initial_vault_balance;

    // Assert user deltas are close to the expected values
    let expected_delta_user1 = -(268 * SCALAR_7); // -268 tokens = -100 - 168
    let expected_delta_user2 = -(386 * SCALAR_7); // -386 tokens = -50 - 336
    let tolerance = 5 * (SCALAR_7 / 10); // 0.5 tokens

    let delta_user1_diff = (delta_user1 - expected_delta_user1).abs();
    assert!(
        delta_user1_diff <= tolerance,
        "delta_user1 ({:.7}) is not approximately -268 (difference: {:.7})",
        delta_user1 as f64 / SCALAR_7 as f64,
        delta_user1_diff as f64 / SCALAR_7 as f64
    );

    let delta_user2_diff = (delta_user2 - expected_delta_user2).abs();
    assert!(
        delta_user2_diff <= tolerance,
        "delta_user2 ({:.7}) is not approximately -436 (difference: {:.7})",
        delta_user2 as f64 / SCALAR_7 as f64,
        delta_user2_diff as f64 / SCALAR_7 as f64
    );

    println!(
        "User1 (long) balance: {:.7} (Δ {:.7})\nUser2 (short) balance: {:.7} (Δ {:.7})\nVault balance: {:.7} (Δ {:.7})",
        user1_balance as f64 / SCALAR_7 as f64,
        delta_user1 as f64 / SCALAR_7 as f64,
        user2_balance as f64 / SCALAR_7 as f64,
        delta_user2 as f64 / SCALAR_7 as f64,
        vault_balance as f64 / SCALAR_7 as f64,
        delta_vault as f64 / SCALAR_7 as f64
    );
}
