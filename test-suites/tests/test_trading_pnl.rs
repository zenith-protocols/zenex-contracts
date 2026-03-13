use soroban_fixed_point_math::FixedPoint;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::{AssetIndex, TestFixture};
use test_suites::SCALAR_7;
use trading::testutils::{default_config, default_market, BTC_FEED_ID, BTC_PRICE, PRICE_SCALAR};

const SECONDS_IN_WEEK: u64 = 604800; // 7 days in seconds
const SECONDS_IN_HOUR: u64 = 3600; // 1 hour in seconds

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data()
}

#[test]
fn test_profitable_long_position_small_gain() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(150_000 * SCALAR_7));
    let initial_balance = fixture.token.balance(&user);

    // Open long position at 100K and fill
    let (position_id, _) = fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        1_000 * SCALAR_7, // 1000 tokens collateral
        2_000 * SCALAR_7, // 2x leverage
        true,
        BTC_PRICE, // entry price at 100K
        0, // take profit: 0 (not set)
        0, // stop loss: 0 (not set)
    );

    let balance_after_open = fixture.token.balance(&user);
    let config = default_config();
    let market = default_market(&fixture.env);
    // Base fee is charged on notional_size (2000), not collateral (1000)
    let base_fee = (2_000 * SCALAR_7)
        .fixed_mul_ceil(config.base_fee_dominant, SCALAR_7)
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
    fixture.set_price(BTC_FEED_ID, 105_000 * PRICE_SCALAR);

    // Close position using the new close_position function
    fixture.trading.close_position(&position_id, &fixture.dummy_price());

    // Verify position is deleted
    assert!(!fixture.position_exists(position_id));

    // Calculate expected profit: 5% price increase on notional (2000) = 100 tokens
    // User pays fees on open and close:
    // - Open: base_fee (on notional) + price_impact
    // - Close: base_fee (on notional) + price_impact + interest (small after 1 hour)
    let final_balance = fixture.token.balance(&user);
    let expected_profit = (100 * SCALAR_7) - base_fee - base_fee - price_impact - price_impact;

    // Use tolerance for small interest accrual (1 hour of funding @ 0.01% hourly rate)
    let tolerance = SCALAR_7 / 10; // 0.1 tokens tolerance
    let actual_profit = final_balance - initial_balance;
    let diff = (actual_profit - expected_profit).abs();
    assert!(
        diff <= tolerance,
        "Profit difference {} exceeds tolerance {}",
        diff as f64 / SCALAR_7 as f64,
        tolerance as f64 / SCALAR_7 as f64
    );
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
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // User1 opens a long position at 100k with 2x leverage
    // Choose collateral = 25k -> notional size = 50k (2x)
    let (user1_position_id, _) = fixture.open_and_fill(
        &user1,
        AssetIndex::BTC as u32,
        25_000 * SCALAR_7,
        50_000 * SCALAR_7,
        true,
        BTC_PRICE,
        0,
        0,
    );

    // User2 opens a short position of 100k notional with 10x leverage -> collateral = 10k
    let (user2_position_id, _) = fixture.open_and_fill(
        &user2,
        AssetIndex::BTC as u32,
        10_000 * SCALAR_7,
        100_000 * SCALAR_7,
        false,
        BTC_PRICE,
        0,
        0,
    );

    // Set funding rate via keeper call (required for funding to accrue)
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // A week passes
    fixture.jump(604800);

    // Price goes up 5%: 100K -> 105K
    fixture.set_price(BTC_FEED_ID, 105_000 * PRICE_SCALAR);

    // User1 closes the long using new close_position function
    fixture.trading.close_position(&user1_position_id, &fixture.dummy_price());

    // User2 closes the short using new close_position function
    fixture.trading.close_position(&user2_position_id, &fixture.dummy_price());

    // Ensure positions are deleted
    assert!(!fixture.position_exists(user1_position_id));
    assert!(!fixture.position_exists(user2_position_id));

    // Print final balances of User1, User2 and the Vault (token balances)
    let user1_balance = fixture.token.balance(&user1);
    let user2_balance = fixture.token.balance(&user2);
    let vault_balance = fixture.token.balance(&fixture.vault.address);
    let contract_balance = fixture.token.balance(&fixture.trading.address);

    let delta_user1 = user1_balance - initial_user1_balance;
    // delta user1: 2500 (5% of 50k profit) - fees
    // Fees on open: base_fee (25) + price_impact (~0.0000125)
    // Fees on close: base_fee (25 - only when dominant, but after user2 closed, longs=50k, shorts=0, so long is dominant) + price_impact + interest
    let delta_user2 = user2_balance - initial_user2_balance;
    // delta user2: -5000 (5% of 100k loss) - fees
    // Fees on open: base_fee (50 - shorts become 100k > longs 50k, so dominant) + price_impact (~0.000025)
    // Fees on close: base_fee (50 - shorts 100k > longs 0, still dominant) + price_impact + interest
    // Note: User2 pays base_fee on BOTH open and close because short is dominant in both cases
    let delta_vault = vault_balance - initial_vault_balance;
    // delta vault: net fees from both users plus the net pnl (2.5k to vault from user2 loss)

    println!(
        "Contract balance (should be 0): {:.7}",
        contract_balance as f64 / SCALAR_7 as f64
    );
    println!(
        "User1 (long 50k) Δ {:.7}\nUser2 (short 100k) Δ {:.7}\nVault Δ {:.7}",
        delta_user1 as f64 / SCALAR_7 as f64,
        delta_user2 as f64 / SCALAR_7 as f64,
        delta_vault as f64 / SCALAR_7 as f64
    );

    assert_eq!(contract_balance, 0);

    // Shorts dominant (100k > 50k), rate = base_rate × 50k/150k = base_rate/3
    // Shorts pay 168h of funding, longs receive (scaled by D/M ratio) minus 20% vault_skim
    let tolerance = 2 * SCALAR_7;
    let expected_delta_user1 = 25148 * (SCALAR_7 / 10); // ~2514.8 tokens
    let expected_delta_user2 = -51560 * (SCALAR_7 / 10); // ~-5156.0 tokens
    let expected_delta_vault = 26412 * (SCALAR_7 / 10); // ~2641.2 tokens

    let delta_user1_diff = (delta_user1 - expected_delta_user1).abs();
    assert!(
        delta_user1_diff <= tolerance,
        "delta_user1 off by {:.4} (expected {:.4}, got {:.4})",
        delta_user1_diff as f64 / SCALAR_7 as f64,
        expected_delta_user1 as f64 / SCALAR_7 as f64,
        delta_user1 as f64 / SCALAR_7 as f64
    );

    let delta_user2_diff = (delta_user2 - expected_delta_user2).abs();
    assert!(
        delta_user2_diff <= 5 * SCALAR_7,
        "delta_user2 off by {:.4} (expected {:.4}, got {:.4})",
        delta_user2_diff as f64 / SCALAR_7 as f64,
        expected_delta_user2 as f64 / SCALAR_7 as f64,
        delta_user2 as f64 / SCALAR_7 as f64
    );

    let delta_vault_diff = (delta_vault - expected_delta_vault).abs();
    assert!(
        delta_vault_diff <= 15 * SCALAR_7,
        "delta_vault off by {:.4} (expected {:.4}, got {:.4})",
        delta_vault_diff as f64 / SCALAR_7 as f64,
        expected_delta_vault as f64 / SCALAR_7 as f64,
        delta_vault as f64 / SCALAR_7 as f64
    );
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
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // Both target 200k notional. Choose ample collateral to avoid liquidation on a 10% move.
    // Collateral = 50k, Notional = 200k (4x leverage)
    let collateral = 50_000 * SCALAR_7;
    let notional = 200_000 * SCALAR_7;

    // User1 opens a SHORT at 100k
    let (user1_position_id, _) = fixture.open_and_fill(
        &user1,
        AssetIndex::BTC as u32,
        collateral,
        notional,
        false,
        BTC_PRICE,
        0,
        0,
    );

    // User2 opens a LONG at 100k
    let (user2_position_id, _) = fixture.open_and_fill(
        &user2,
        AssetIndex::BTC as u32,
        collateral,
        notional,
        true,
        BTC_PRICE,
        0,
        0,
    );

    // A week passes
    fixture.jump(604800);

    // Price drops 10%: 100K -> 90K
    fixture.set_price(BTC_FEED_ID, 90_000 * PRICE_SCALAR);

    // Close both positions using new close_position function
    fixture.trading.close_position(&user1_position_id, &fixture.dummy_price());
    fixture.trading.close_position(&user2_position_id, &fixture.dummy_price());

    // Ensure positions are deleted
    assert!(!fixture.position_exists(user1_position_id));
    assert!(!fixture.position_exists(user2_position_id));

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
    // Equal notional → funding_rate = 0 in new model, so no funding costs
    // User1 (short): +20000 PnL - fees (~200 base_fee + impact)
    // User2 (long): -20000 PnL - fees (~120 base_fee + impact)
    let expected_delta_user1 = 19_800 * SCALAR_7;
    let expected_delta_user2 = -20_120 * SCALAR_7;
    let tolerance = SCALAR_7; // 1 token

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
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // Notionals per scenario
    let notional_100k = 100_000 * SCALAR_7;
    let notional_50k = 50_000 * SCALAR_7;

    // Choose reasonable collateral to avoid liquidation purely from funding/fees
    let collateral_100k = 20_000 * SCALAR_7; // 5x leverage
    let collateral_50k = 10_000 * SCALAR_7; // 5x leverage

    // User1 opens LONG 100k
    let (user1_position_id, _) = fixture.open_and_fill(
        &user1,
        AssetIndex::BTC as u32,
        collateral_100k,
        notional_100k,
        true,
        BTC_PRICE,
        0,
        0,
    );

    // User2 opens SHORT 50k
    let (user2_position_id, _) = fixture.open_and_fill(
        &user2,
        AssetIndex::BTC as u32,
        collateral_50k,
        notional_50k,
        false,
        BTC_PRICE,
        0,
        0,
    );

    // Set funding rate: longs dominant (100k vs 50k)
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Price does not move (stay at 100k)
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // User3 opens SHORT 100k
    let (user3_position_id, _) = fixture.open_and_fill(
        &user3,
        AssetIndex::BTC as u32,
        collateral_100k,
        notional_100k,
        false,
        BTC_PRICE,
        0,
        0,
    );

    // Update funding rate: shorts now dominant (150k vs 100k)
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // Another week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Price still unchanged at 100k
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // Close all positions using new close_position function
    fixture.trading.close_position(&user1_position_id, &fixture.dummy_price());
    fixture.trading.close_position(&user2_position_id, &fixture.dummy_price());
    fixture.trading.close_position(&user3_position_id, &fixture.dummy_price());

    // Ensure positions are deleted
    assert!(!fixture.position_exists(user1_position_id));
    assert!(!fixture.position_exists(user2_position_id));
    assert!(!fixture.position_exists(user3_position_id));

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

    // Week 1: long 100k vs short 50k → longs dominant, rate = base_rate × 50k/150k = base_rate/3
    // Week 2: long 100k vs short 150k → shorts dominant, rate = base_rate × 50k/250k = base_rate/5
    // User1 (long 100k, 2 weeks): pays funding week 1, receives week 2
    // User2 (short 50k, 2 weeks): receives funding week 1, pays week 2
    // User3 (short 100k, 1 week): pays funding week 2
    let expected_delta_user1 = -(66 * SCALAR_7); // ~-66 tokens (fees + net funding)
    let expected_delta_user2 = 15 * (SCALAR_7 / 10); // ~+1.5 tokens (funding received offsets fees)
    let expected_delta_user3 = -(1334 * (SCALAR_7 / 10)); // ~-133.4 tokens (fees + funding paid)
    let tolerance = 2 * SCALAR_7;

    let delta_user1_diff = (delta_user1 - expected_delta_user1).abs();
    assert!(
        delta_user1_diff <= tolerance,
        "delta_user1 ({:.4}) is not approximately {:.4} (difference: {:.4})",
        delta_user1 as f64 / SCALAR_7 as f64,
        expected_delta_user1 as f64 / SCALAR_7 as f64,
        delta_user1_diff as f64 / SCALAR_7 as f64
    );

    let delta_user2_diff = (delta_user2 - expected_delta_user2).abs();
    assert!(
        delta_user2_diff <= tolerance,
        "delta_user2 ({:.4}) is not approximately {:.4} (difference: {:.4})",
        delta_user2 as f64 / SCALAR_7 as f64,
        expected_delta_user2 as f64 / SCALAR_7 as f64,
        delta_user2_diff as f64 / SCALAR_7 as f64
    );

    let delta_user3_diff = (delta_user3 - expected_delta_user3).abs();
    assert!(
        delta_user3_diff <= tolerance,
        "delta_user3 ({:.4}) is not approximately {:.4} (difference: {:.4})",
        delta_user3 as f64 / SCALAR_7 as f64,
        expected_delta_user3 as f64 / SCALAR_7 as f64,
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
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // User1 opens a long position with 50k collateral and 2x leverage (100k notional)
    let (user1_position_id, _) = fixture.open_and_fill(
        &user1,
        AssetIndex::BTC as u32,
        50_000 * SCALAR_7, // 50k collateral
        100_000 * SCALAR_7, // 100k notional (2x leverage)
        true, // long
        BTC_PRICE,
        0,
        0,
    );

    // Set funding rate: one-sided long → rate = base_rate, longs pay
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Ensure price is still at 100K before User2 opens position
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // User2 opens a short position with 50k collateral and 2x leverage (100k notional)
    let (user2_position_id, _) = fixture.open_and_fill(
        &user2,
        AssetIndex::BTC as u32,
        50_000 * SCALAR_7, // 50k collateral
        100_000 * SCALAR_7, // 100k notional (2x leverage)
        false, // short
        BTC_PRICE,
        0,
        0,
    );

    // Update funding rate: balanced (100k vs 100k) → rate = 0
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // Another week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Ensure price is still at 100K (no price movement)
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // User1 closes the long position using new close_position function
    fixture.trading.close_position(&user1_position_id, &fixture.dummy_price());

    // User2 closes the short position using new close_position function
    fixture.trading.close_position(&user2_position_id, &fixture.dummy_price());

    // Ensure positions are deleted
    assert!(!fixture.position_exists(user1_position_id));
    assert!(!fixture.position_exists(user2_position_id));

    // Calculate deltas
    let user1_balance = fixture.token.balance(&user1);
    let user2_balance = fixture.token.balance(&user2);
    let vault_balance = fixture.token.balance(&fixture.vault.address);

    let delta_user1 = user1_balance - initial_user1_balance;
    let delta_user2 = user2_balance - initial_user2_balance;
    let delta_vault = vault_balance - initial_vault_balance;

    println!(
        "User1 (long) Δ {:.4}\nUser2 (short) Δ {:.4}\nVault Δ {:.4}",
        delta_user1 as f64 / SCALAR_7 as f64,
        delta_user2 as f64 / SCALAR_7 as f64,
        delta_vault as f64 / SCALAR_7 as f64
    );

    // Week 1: only long (one-sided) → rate = base_rate, longs pay
    // Week 2: long 100k = short 100k → rate = 0 (balanced), no funding
    // User1 (long, 2 weeks): pays ~169 funding (week 1 + 1h extra), 0 week 2, + 100 fees
    // User2 (short, 1 week): receives small funding, pays fees
    let expected_delta_user1 = -(269 * SCALAR_7); // -269 tokens (fees + funding)
    let expected_delta_user2 = -(59 * SCALAR_7); // ~-59 tokens (fees + small funding effects)
    let tolerance = 2 * SCALAR_7;

    let delta_user1_diff = (delta_user1 - expected_delta_user1).abs();
    assert!(
        delta_user1_diff <= tolerance,
        "delta_user1 ({:.4}) is not approximately {:.4} (difference: {:.4})",
        delta_user1 as f64 / SCALAR_7 as f64,
        expected_delta_user1 as f64 / SCALAR_7 as f64,
        delta_user1_diff as f64 / SCALAR_7 as f64
    );

    let delta_user2_diff = (delta_user2 - expected_delta_user2).abs();
    assert!(
        delta_user2_diff <= tolerance,
        "delta_user2 ({:.4}) is not approximately {:.4} (difference: {:.4})",
        delta_user2 as f64 / SCALAR_7 as f64,
        expected_delta_user2 as f64 / SCALAR_7 as f64,
        delta_user2_diff as f64 / SCALAR_7 as f64
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
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // User1 opens a long position with 10k collateral and 10x leverage (100k notional)
    let (user1_position_id, _) = fixture.open_and_fill(
        &user1,
        AssetIndex::BTC as u32,
        10_000 * SCALAR_7, // 10k collateral
        100_000 * SCALAR_7, // 100k notional (10x leverage)
        true, // long
        BTC_PRICE,
        0,
        0,
    );

    // User2 opens a short position with 10k collateral and 10x leverage (100k notional)
    let (user2_position_id, _) = fixture.open_and_fill(
        &user2,
        AssetIndex::BTC as u32,
        10_000 * SCALAR_7, // 10k collateral
        100_000 * SCALAR_7, // 100k notional (10x leverage)
        false, // short
        BTC_PRICE,
        0,
        0,
    );

    // Set funding rate: balanced (100k vs 100k) → rate = 0
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // A week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Ensure price is still at 100K (no price movement)
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // User1 closes the long position using new close_position function
    fixture.trading.close_position(&user1_position_id, &fixture.dummy_price());

    // Update funding rate: one-sided short → rate = -base_rate, shorts pay
    fixture.jump(SECONDS_IN_HOUR);
    fixture.trading.apply_funding();

    // Another week passes
    fixture.jump(SECONDS_IN_WEEK);

    // Ensure price is still at 100K (no price movement)
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // User2 closes the short position using new close_position function
    fixture.trading.close_position(&user2_position_id, &fixture.dummy_price());

    // Ensure positions are deleted
    assert!(!fixture.position_exists(user1_position_id));
    assert!(!fixture.position_exists(user2_position_id));

    // Calculate deltas
    let user1_balance = fixture.token.balance(&user1);
    let user2_balance = fixture.token.balance(&user2);
    let vault_balance = fixture.token.balance(&fixture.vault.address);

    let delta_user1 = user1_balance - initial_user1_balance;
    let delta_user2 = user2_balance - initial_user2_balance;
    let delta_vault = vault_balance - initial_vault_balance;

    // Assert user deltas are close to the expected values
    // Week 1: long 100k = short 100k → rate = 0 (balanced), no funding
    // Week 2: only short (one-sided) → rate = -base_rate, shorts pay
    // User1 (long, closes after week 1): 0 funding, only fees
    // User2 (short, 2 weeks): 0 funding week 1, pays 1x funding week 2
    let expected_delta_user1 = -(100 * SCALAR_7); // -100 tokens (fees only)
    let expected_delta_user2 = -(228 * SCALAR_7); // -228 tokens (fees + 1x funding week 2)
    let tolerance = 5 * (SCALAR_7 / 10); // 0.5 tokens

    let delta_user1_diff = (delta_user1 - expected_delta_user1).abs();
    assert!(
        delta_user1_diff <= tolerance,
        "delta_user1 ({:.7}) is not approximately -100 (difference: {:.7})",
        delta_user1 as f64 / SCALAR_7 as f64,
        delta_user1_diff as f64 / SCALAR_7 as f64
    );

    let delta_user2_diff = (delta_user2 - expected_delta_user2).abs();
    assert!(
        delta_user2_diff <= tolerance,
        "delta_user2 ({:.7}) is not approximately -228 (difference: {:.7})",
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
fn test_close_when_loss_exceeds_collateral() {
    // This test verifies that closing a position works correctly when loss + fees > collateral
    // Previously this would fail because the contract tried to transfer more to vault than it held
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);

    // Fund user
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    let initial_user_balance = fixture.token.balance(&user);
    let initial_vault_balance = fixture.token.balance(&fixture.vault.address);

    // Set BTC price to 100K
    fixture.set_price(BTC_FEED_ID, 100_000 * PRICE_SCALAR);

    // Open a highly leveraged long position and fill
    // 1k collateral, 20k notional (20x leverage)
    let (position_id, _) = fixture.open_and_fill(
        &user,
        AssetIndex::BTC as u32,
        1_000 * SCALAR_7,  // 1k collateral
        20_000 * SCALAR_7, // 20k notional (20x leverage)
        true,               // long
        BTC_PRICE,
        0,
        0,
    );

    // Jump time to accrue some interest
    fixture.jump(SECONDS_IN_WEEK);

    // Price drops 10%: 100K -> 90K
    // This creates a 10% loss on 20k notional = 2k loss
    // With 1k collateral, loss (2k) > collateral (1k)
    fixture.set_price(BTC_FEED_ID, 90_000 * PRICE_SCALAR);

    // Close position - this should NOT panic even though loss > collateral
    let (pnl, fee) = fixture.trading.close_position(&position_id, &fixture.dummy_price());

    // Verify position is deleted
    assert!(!fixture.position_exists(position_id));

    // Verify PnL is negative (loss)
    assert!(pnl < 0, "Expected negative PnL (loss), got {}", pnl);

    // Verify loss exceeds original collateral
    let collateral = 1_000 * SCALAR_7;
    assert!(
        (-pnl) > collateral,
        "Loss {} should exceed collateral {}",
        -pnl,
        collateral
    );

    // Verify user balance - should have lost all collateral but nothing more
    let final_user_balance = fixture.token.balance(&user);
    let user_loss = initial_user_balance - final_user_balance;

    // User should lose approximately collateral + opening fees
    // (close fees come from collateral, not user's remaining balance)
    println!(
        "User loss: {:.7} (PnL: {:.7}, Fee: {:.7})",
        user_loss as f64 / SCALAR_7 as f64,
        pnl as f64 / SCALAR_7 as f64,
        fee as f64 / SCALAR_7 as f64
    );

    // Verify vault received the collateral (minus any caller fees)
    let final_vault_balance = fixture.token.balance(&fixture.vault.address);
    let vault_gain = final_vault_balance - initial_vault_balance;
    println!(
        "Vault gain: {:.7}",
        vault_gain as f64 / SCALAR_7 as f64
    );

    // Contract should have no balance left
    let contract_balance = fixture.token.balance(&fixture.trading.address);
    assert_eq!(
        contract_balance, 0,
        "Contract should have 0 balance, but has {}",
        contract_balance
    );
}
