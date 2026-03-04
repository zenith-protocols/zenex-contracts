use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::BTC_PRICE;
use trading::ExecuteRequest;

const SECONDS_IN_WEEK: u64 = 604800; // 7 days in seconds

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data(false)
}

#[test]
fn test_long_position_liquidation_after_week() {
    // User opens a long on BTC with 10k collateral and 10x leverage.
    // The price of BTC is 100k. A week passes, the price of bitcoin is now 90900.
    // Check whether the position liquidates, if it does, the test passes.
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let liquidator = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Set BTC price to 100K
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // Open long position: 10k collateral, 100k notional (10x leverage) and fill
    let (position_id, _) = fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        10_000 * SCALAR_7,  // 10k collateral
        100_000 * SCALAR_7, // 100k notional (10x leverage)
        true,                // long
        BTC_PRICE,
        0,
        0,
    );

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Price drops to make position liquidatable
    // New funding model: one-sided → rate = base_rate (naturally bounded in [0, baseRate])
    // Over 1 week (168h): funding = 100k × base_rate × 168 = 168 tokens
    // equity = 10000 + PnL - (50 close_fee + 168 funding) < 500
    // → PnL < -9282 → price < 90,718
    let current_price = 90_710_0000000;
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,      // USD
        current_price,  // BTC = 90,710
        2000_0000000,   // ETH
        0_1000000,      // XLM
    ]);

    // Attempt liquidation
    let result = fixture.trading.execute(
        &liquidator,
        &svec![
            &fixture.env,
            ExecuteRequest {
                request_type: 3, // Liquidate
                position_id,
            },
        ],
    );

    // Check if position liquidated
    let result_code = result.get(0).unwrap();

    // Debug output
    println!("\nTest at price: {}", current_price);
    println!("Result code: {} (0=success, 20=BadRequest/not eligible)", result_code);

    // Test passes if liquidation was successful (result code 0) and position is deleted
    assert_eq!(
        result_code,
        0u32,
        "Liquidation should succeed when equity < maintenance margin (after accounting for interest). Result code {} indicates failure.",
        result_code
    );
    assert!(
        !fixture.position_exists(position_id),
        "Position should be deleted after successful liquidation"
    );

    // Verify contract balance is 0 (all funds transferred)
    let contract_balance = fixture.token.balance(&fixture.trading.address);
    assert_eq!(contract_balance, 0);
}

#[test]
fn test_long_position_not_liquidatable_at_threshold() {
    // This test verifies that a position is NOT liquidatable when equity is just above maintenance margin
    // At price 90,730, with interest accumulation over 1 week, equity is above the 500 maintenance margin
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let liquidator = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Set BTC price to 100K
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        100_000_0000000, // BTC = 100K
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // Open long position: 10k collateral, 100k notional (10x leverage) and fill
    let (position_id, _) = fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        10_000 * SCALAR_7,  // 10k collateral
        100_000 * SCALAR_7, // 100k notional (10x leverage)
        true,                // long
        BTC_PRICE,
        0,
        0,
    );

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Price drops to just above liquidation threshold
    // equity = 10000 + PnL - (50 close_fee + 168 funding) >= 500
    // → PnL >= -9282 → price >= 90,718
    // Use 90,730 to be safely above threshold
    let current_price = 90_730_0000000;
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,      // USD
        current_price,  // BTC = 90,730
        2000_0000000,   // ETH
        0_1000000,      // XLM
    ]);

    // Attempt liquidation
    let result = fixture.trading.execute(
        &liquidator,
        &svec![
            &fixture.env,
            ExecuteRequest {
                request_type: 3, // Liquidate
                position_id,
            },
        ],
    );

    // Check that liquidation failed
    let position_after = fixture.trading.get_position(&position_id);
    let result_code = result.get(0).unwrap();

    // Debug output
    println!("\nTest at price: {}", current_price);
    println!("Result code: {} (0=success, 20=BadRequest/not eligible)", result_code);
    println!("Position filled: {}", position_after.filled);

    // Test passes if liquidation FAILED (result code 746 = PositionNotLiquidatable) and position still exists
    assert_eq!(
        result_code,
        746u32,  // TradingError::PositionNotLiquidatable
        "Liquidation should fail when equity >= maintenance margin. Result code {} indicates it succeeded when it shouldn't.",
        result_code
    );
    assert!(
        position_after.filled,
        "Position should remain open (filled=true) when equity >= maintenance margin"
    );
}