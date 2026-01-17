use soroban_fixed_point_math::FixedPoint;
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

    // Price drops to test threshold - use 90,720 to check exact equity
    let current_price = 90_720_0000000; // 90,720
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,      // USD
        current_price,  // BTC = 90,720
        2000_0000000,   // ETH
        0_1000000,      // XLM
    ]);

    // Calculate and print equity details before liquidation attempt
    let position = fixture.read_position(position_id);
    let market_data = fixture.read_market_data(fixture.assets[AssetIndex::BTC].clone());
    let market_config = fixture.read_market_config(fixture.assets[AssetIndex::BTC].clone());
    
    // Calculate PnL (same logic as position.calculate_pnl)
    let price_diff = if position.is_long {
        current_price - position.entry_price
    } else {
        position.entry_price - current_price
    };
    let pnl = if price_diff == 0 {
        0
    } else {
        let price_change_ratio = price_diff
            .fixed_div_floor(position.entry_price, SCALAR_7)
            .unwrap();
        position.notional_size
            .fixed_mul_floor(price_change_ratio, SCALAR_7)
            .unwrap()
    };
    
    // Calculate fee (same logic as position.calculate_fee)
    let is_long_dominant = market_data.long_notional_size > market_data.short_notional_size;
    let is_short_dominant = market_data.short_notional_size > market_data.long_notional_size;
    let is_balanced = market_data.long_notional_size == market_data.short_notional_size;
    let should_pay_base_fee = is_balanced || (is_long_dominant && position.is_long) || (is_short_dominant && !position.is_long);
    
    let base_fee = if should_pay_base_fee {
        position.notional_size
            .fixed_mul_ceil(market_config.base_fee, SCALAR_7)
            .unwrap()
    } else {
        0
    };
    
    let price_impact_scalar = position.notional_size
        .fixed_div_ceil(market_config.price_impact_scalar, SCALAR_7)
        .unwrap();
    
    let index_difference = if position.is_long {
        market_data.long_interest_index - position.interest_index
    } else {
        market_data.short_interest_index - position.interest_index
    };
    
    const SCALAR_18: i128 = 1_000_000_000_000_000_000; // 18 decimal places
    let interest_fee = position.notional_size
        .fixed_mul_floor(index_difference, SCALAR_18)
        .unwrap();
    
    let total_fee = base_fee + price_impact_scalar + interest_fee;
    
    // Calculate equity
    let equity = position.collateral + pnl - total_fee;
    
    // Calculate maintenance margin
    let required_margin = position.notional_size
        .fixed_mul_floor(market_config.maintenance_margin, SCALAR_7)
        .unwrap();
    
    println!("\n=== Equity Calculation Debug ===");
    println!("Collateral: {:.7}", position.collateral as f64 / SCALAR_7 as f64);
    println!("PnL: {:.7} (price change: {:.2}%)", 
        pnl as f64 / SCALAR_7 as f64,
        (price_diff as f64 / position.entry_price as f64) * 100.0
    );
    println!("Base fee: {:.7}", base_fee as f64 / SCALAR_7 as f64);
    println!("Price impact: {:.7}", price_impact_scalar as f64 / SCALAR_7 as f64);
    println!("Interest fee: {:.7} (index diff: {})", 
        interest_fee as f64 / SCALAR_7 as f64,
        index_difference
    );
    println!("Total fee: {:.7}", total_fee as f64 / SCALAR_7 as f64);
    println!("Equity: {:.7} = collateral ({:.7}) + PnL ({:.7}) - fee ({:.7})", 
        equity as f64 / SCALAR_7 as f64,
        position.collateral as f64 / SCALAR_7 as f64,
        pnl as f64 / SCALAR_7 as f64,
        total_fee as f64 / SCALAR_7 as f64
    );
    println!("Required margin (maintenance): {:.7}", required_margin as f64 / SCALAR_7 as f64);
    println!("Equity >= Required margin? {} (Liquidation possible if false)", equity >= required_margin);

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
    
    // Debug: Print the result code to understand what happened
    let result_code = result.results.get(0).unwrap();
    println!("\n=== Liquidation Debug ===");
    println!("Result code: {}", result_code);
    println!("Position status: {:?}", position_after.status);
    
    // Print transfers for visibility
    fixture.print_transfers(&result);

    // Test passes if liquidation was successful (result code 0) and position is closed
    assert_eq!(
        result_code,
        0u32,
        "Liquidation should succeed when equity < maintenance margin. Result code {} indicates it didn't liquidate (20 = BadRequest = not eligible).",
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
