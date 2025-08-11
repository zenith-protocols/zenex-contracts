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

    fixture.jump(604800);

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
    let expected_profit = (100 * SCALAR_7) - base_fee - base_fee - price_impact - price_impact; // 10% gain on collateral

    assert_eq!(final_balance, initial_balance + expected_profit);
}
