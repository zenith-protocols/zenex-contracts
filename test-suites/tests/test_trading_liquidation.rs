use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::{PositionStatus, Request, RequestType};

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

    // Open long position: 10k collateral, 100k notional (10x leverage)
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(10_000 * SCALAR_7),  // 10k collateral
        &(100_000 * SCALAR_7), // 100k notional (10x leverage)
        &true,                 // long
        &0,                    // market order
        &0,                    // take profit: 0 (not set)
        &0,                    // stop loss: 0 (not set)
    );

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Price drops to make position liquidatable
    // At 90,700: PnL=-9300, interest≈168, base_fee=50 → equity≈482 < 500 (liquidatable)
    // At 90,718: equity≈500 (borderline liquidatable)
    // At 90,719: equity≈501 (not liquidatable)
    let current_price = 90_718_0000000;
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,      // USD
        current_price,  // BTC = 90,720
        2000_0000000,   // ETH
        0_1000000,      // XLM
    ]);

    // Attempt liquidation
    let result = fixture.trading.submit(
        &liquidator,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Liquidation,
                position: position_id,
                data: None,
            },
        ],
    );

    // Check if position liquidated
    let position_after = fixture.read_position(position_id);
    let result_code = result.results.get(0).unwrap();

    // Debug output
    println!("\nTest at price: {}", current_price);
    println!("Result code: {} (0=success, 20=BadRequest/not eligible)", result_code);
    println!("Position status: {:?}", position_after.status);

    // Test passes if liquidation was successful (result code 0) and position is closed
    assert_eq!(
        result_code,
        0u32,
        "Liquidation should succeed when equity < maintenance margin (after accounting for interest). Result code {} indicates failure.",
        result_code
    );
    assert_eq!(
        position_after.status,
        PositionStatus::Closed,
        "Position should be closed after successful liquidation"
    );

    // Verify contract balance is 0 (all funds transferred)
    let contract_balance = fixture.token.balance(&fixture.trading.address);
    assert_eq!(contract_balance, 0);
}

#[test]
fn test_long_position_not_liquidatable_at_threshold() {
    // This test verifies that a position is NOT liquidatable when equity is just above maintenance margin
    // At price 90,719, with interest accumulation over 1 week, equity ≈ 501 which is above the 500 maintenance margin
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

    // Open long position: 10k collateral, 100k notional (10x leverage)
    let position_id = fixture.trading.open_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(10_000 * SCALAR_7),  // 10k collateral
        &(100_000 * SCALAR_7), // 100k notional (10x leverage)
        &true,                 // long
        &0,                    // market order
        &0,                    // take profit: 0 (not set)
        &0,                    // stop loss: 0 (not set)
    );

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Price drops to just above liquidation threshold
    // At 90,719: PnL=-9281, interest≈168, base_fee=50 → equity≈501 > 500 (NOT liquidatable)
    let current_price = 90_719_0000000;
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,      // USD
        current_price,  // BTC = 90,719
        2000_0000000,   // ETH
        0_1000000,      // XLM
    ]);

    // Attempt liquidation
    let result = fixture.trading.submit(
        &liquidator,
        &svec![
            &fixture.env,
            Request {
                action: RequestType::Liquidation,
                position: position_id,
                data: None,
            },
        ],
    );

    // Check that liquidation failed
    let position_after = fixture.read_position(position_id);
    let result_code = result.results.get(0).unwrap();

    // Debug output
    println!("\nTest at price: {}", current_price);
    println!("Result code: {} (0=success, 20=BadRequest/not eligible)", result_code);
    println!("Position status: {:?}", position_after.status);

    // Test passes if liquidation FAILED (result code 20) and position is still open
    assert_eq!(
        result_code,
        20u32,
        "Liquidation should fail when equity >= maintenance margin. Result code {} indicates it succeeded when it shouldn't.",
        result_code
    );
    assert_eq!(
        position_after.status,
        PositionStatus::Open,
        "Position should remain open when equity >= maintenance margin"
    );
}
