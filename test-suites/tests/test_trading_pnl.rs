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

    fixture.jump(SECONDS_IN_HOUR);

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

    let expected_balance = initial_balance + expected_profit;

    //print the difference in expected and actual balance
    println!(
        "Difference: {}",
        (final_balance - expected_balance) as f64 / SCALAR_7 as f64,
    );

    assert_eq!(final_balance, initial_balance + expected_profit);
}

#[test]
fn test_mixed_positions_different_leverage() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user with enough collateral for both positions
    fixture.token.mint(&user, &(300_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position at 100K with 2x leverage (1000 collateral, 2000 notional)
    let long_position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_000 * SCALAR_7), // 1000 tokens collateral
        &(2_000 * SCALAR_7), // 2000 tokens notional (2x leverage)
        &true,               // long position
        &0,                  // market order at 100K
    );

    // Open short position at 100K with 3x leverage (1500 collateral, 4500 notional)
    let short_position_id = fixture.trading.create_position(
        &user,
        &fixture.assets[AssetIndex::BTC],
        &(1_500 * SCALAR_7), // 1500 tokens collateral
        &(4_500 * SCALAR_7), // 4500 tokens notional (3x leverage)
        &false,              // short position
        &0,                  // market order at 100K
    );

    let balance_after_open = fixture.token.balance(&user);
    let market = default_market();

    // Calculate fees for both positions
    let long_base_fee = (1_000 * SCALAR_7)
        .fixed_mul_ceil(market.base_fee, SCALAR_7)
        .unwrap();
    let long_price_impact = (2_000 * SCALAR_7)
        .fixed_div_ceil(market.price_impact_scalar, SCALAR_7)
        .unwrap();

    let short_base_fee = (1_500 * SCALAR_7)
        .fixed_mul_ceil(market.base_fee, SCALAR_7)
        .unwrap();
    let short_price_impact = (4_500 * SCALAR_7)
        .fixed_div_ceil(market.price_impact_scalar, SCALAR_7)
        .unwrap();

    let total_fees = long_base_fee + long_price_impact + short_base_fee + short_price_impact;
    let total_collateral = (1_000 + 1_500) * SCALAR_7;

    assert_eq!(
        balance_after_open,
        initial_balance - total_collateral - total_fees
    );

    fixture.jump(SECONDS_IN_HOUR);

    // Price goes up 10% to 110K
    // Long position should profit, short position should lose
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,       // USD
        110_000_0000000, // BTC = 110K (+10%)
        2000_0000000,    // ETH
        0_1000000,       // XLM
    ]);

    // Close both positions
    let requests = svec![
        &fixture.env,
        Request {
            action: RequestType::Close,
            position: long_position_id,
            data: None,
        },
        Request {
            action: RequestType::Close,
            position: short_position_id,
            data: None,
        }
    ];

    let result = fixture.trading.submit(&user, &requests);

    // Verify both positions are closed
    let long_position = fixture.read_position(long_position_id);
    let short_position = fixture.read_position(short_position_id);
    assert_eq!(long_position.status, PositionStatus::Closed);
    assert_eq!(short_position.status, PositionStatus::Closed);

    fixture.print_transfers(&result);

    // Calculate expected PnL:
    // Long position: 10% price increase * 2x leverage = 20% gain on collateral
    // Profit = 1000 * 0.20 = 200 tokens
    // Short position: 10% price increase * 3x leverage = 30% loss on collateral
    // Loss = 1500 * 0.30 = 450 tokens
    // Net result: 200 - 450 = -250 tokens loss

    let final_balance = fixture.token.balance(&user);
    let expected_long_profit = (200 * SCALAR_7) - long_base_fee - long_price_impact;
    let expected_short_loss = (450 * SCALAR_7) + short_base_fee + short_price_impact;
    let expected_net_result = expected_long_profit - expected_short_loss;

    let expected_balance = initial_balance + expected_net_result;

    // Print the difference in expected and actual balance
    println!(
        "Long Position - 2x Leverage: +{:.6} tokens profit",
        expected_long_profit as f64 / SCALAR_7 as f64,
    );
    println!(
        "Short Position - 3x Leverage: -{:.6} tokens loss",
        expected_short_loss as f64 / SCALAR_7 as f64,
    );
    println!(
        "Net Result: {:.6} tokens",
        expected_net_result as f64 / SCALAR_7 as f64,
    );
    println!(
        "Difference: {:.6}",
        (final_balance - expected_balance) as f64 / SCALAR_7 as f64,
    );

    assert_eq!(final_balance, expected_balance);
}
