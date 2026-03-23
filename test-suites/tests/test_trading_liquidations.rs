use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::{BTC_FEED_ID, BTC_PRICE, PRICE_SCALAR};
use trading::ExecuteRequest;

const SECONDS_IN_WEEK: u64 = 604800; // 7 days in seconds

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data()
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
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // Open long position: 10k collateral, 100k notional (10x leverage) and fill
    let position_id = fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        10_000 * SCALAR_7,  // 10k collateral
        100_000 * SCALAR_7, // 100k notional (10x leverage)
        true,                // long
        BTC_PRICE,
        0,
        0,
    );

    // Set funding rate: one-sided long → rate = base_rate, longs pay
    fixture.jump(3600);
    fixture.trading.apply_funding();

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Price drops to make position liquidatable
    let current_price = 90_710 * PRICE_SCALAR;
    fixture.set_price(BTC_FEED_ID, current_price);

    // Attempt liquidation
    fixture.trading.execute(
        &liquidator,
        &svec![
            &fixture.env,
            ExecuteRequest {
                request_type: 3, // Liquidate
                position_id,
            },
        ],
        &fixture.dummy_price(),
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
#[should_panic(expected = "Error(Contract, #746)")]
fn test_long_position_not_liquidatable_at_threshold() {
    // This test verifies that a position is NOT liquidatable when equity is just above maintenance margin
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let liquidator = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Set BTC price to 100K
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // Open long position: 10k collateral, 100k notional (10x leverage) and fill
    let position_id = fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        10_000 * SCALAR_7,  // 10k collateral
        100_000 * SCALAR_7, // 100k notional (10x leverage)
        true,                // long
        BTC_PRICE,
        0,
        0,
    );

    // Set funding rate: one-sided long → rate = base_rate, longs pay
    fixture.jump(3600);
    fixture.trading.apply_funding();

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Price drops to just above liquidation threshold (accounting for funding + borrowing fees)
    let current_price = 91_000 * PRICE_SCALAR;
    fixture.set_price(BTC_FEED_ID, current_price);

    // Attempt liquidation — should panic with PositionNotLiquidatable
    fixture.trading.execute(
        &liquidator,
        &svec![
            &fixture.env,
            ExecuteRequest {
                request_type: 3, // Liquidate
                position_id,
            },
        ],
        &fixture.dummy_price(),
    );
}
