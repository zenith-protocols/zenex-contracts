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

#[test]
fn test_long_short_week_5pct_move_print_balances() {
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

    // Set BTC price to 50K before opening positions
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,        // USD
        100_000_0000000,   // BTC = 100K
        2000_0000000,     // ETH
        0_1000000,        // XLM
    ]);

    // User1 opens a long position at 100k with 2x leverage
    // Choose collateral = 25k tokens -> notional size = 50k tokens (2x)
    let user1_position_id = fixture.trading.create_position(
        &user1,
        &fixture.assets[AssetIndex::BTC],
        &(25_000 * SCALAR_7),
        &(50_000 * SCALAR_7),
        &true,
        &0, // market order at current price (100K)
    );

    // User2 opens a short position of 100k notional with 10x leverage -> collateral = 10k
    let user2_position_id = fixture.trading.create_position(
        &user2,
        &fixture.assets[AssetIndex::BTC],
        &(10_000 * SCALAR_7),
        &(100_000 * SCALAR_7),
        &false,
        &0, // market order at current price (100K)
    );

    // A week passes
    fixture.jump(604800);

    // Price goes up 5%: 100K -> 105K
    fixture.oracle.set_price_stable(&svec![
        &fixture.env,
        1_0000000,        // USD
        105_000_0000000,   // BTC = 105K (+5%)
        2000_0000000,     // ETH
        0_1000000,        // XLM
    ]);

    // User1 closes the long
    let result1 = fixture.trading.submit(&user1, &svec![
        &fixture.env,
        Request { action: RequestType::Close, position: user1_position_id, data: None }
    ]);

    // User2 closes the short
    let result2 = fixture.trading.submit(&user2, &svec![
        &fixture.env,
        Request { action: RequestType::Close, position: user2_position_id, data: None }
    ]);

    // Ensure positions are closed
    assert_eq!(fixture.read_position(user1_position_id).status, PositionStatus::Closed);
    assert_eq!(fixture.read_position(user2_position_id).status, PositionStatus::Closed);

    // Print transfers for visibility
    fixture.print_transfers(&result1);
    fixture.print_transfers(&result2);

    // Print final balances of User1, User2 and the Vault (token balances)
    let user1_balance = fixture.token.balance(&user1);
    let user2_balance = fixture.token.balance(&user2);
    let vault_balance = fixture.token.balance(&fixture.vault.address);

    let delta_user1 = user1_balance - initial_user1_balance;
    let delta_user2 = user2_balance - initial_user2_balance;
    let delta_vault = vault_balance - initial_vault_balance;

    println!(
        "User1 balance: {} (Δ {})\nUser2 balance: {} (Δ {})\nVault balance: {} (Δ {})",
        user1_balance, delta_user1,
        user2_balance, delta_user2,
        vault_balance, delta_vault
    );
}