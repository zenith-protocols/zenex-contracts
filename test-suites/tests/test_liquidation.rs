use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::pyth_helper;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::TestFixture;
use test_suites::constants::{BTC_PRICE_I64, SCALAR_7, SECONDS_PER_WEEK};
use trading::testutils::{FEED_BTC, PRICE_SCALAR};

// ==========================================
// Helper Functions
// ==========================================

fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data()
}

fn trigger_vecs(env: &soroban_sdk::Env, user: &Address, seq: u32) -> (soroban_sdk::Vec<Address>, soroban_sdk::Vec<u32>) {
    (svec![env, user.clone()], svec![env, seq])
}

// ==========================================
// 1. Core Liquidation Tests
// ==========================================

#[test]
fn test_liquidation_underwater_position() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open long with high leverage: 110 collateral, 10000 notional (~91x)
    let position_id = fixture.open_long(&user, FEED_BTC, 110, 10_000, BTC_PRICE_I64);

    // Price drops 2% -- underwater at this leverage
    fixture.jump(31); // past MIN_OPEN_TIME for the position to be closable
    let crash_price = fixture.btc_price(98_000 * PRICE_SCALAR as i64);

    let (users, seqs) = trigger_vecs(&fixture.env, &user, position_id);
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &crash_price);

    assert!(
        !fixture.position_exists(&user, position_id),
        "Position should be removed after liquidation"
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #731)")]
fn test_liquidation_healthy_position_rejected() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open long with moderate leverage: 1000 collateral, 10000 notional (10x)
    let position_id = fixture.open_long(&user, FEED_BTC, 1_000, 10_000, BTC_PRICE_I64);

    // Price drops only 5% -- with 10x leverage and 1% margin, position is still healthy
    // Equity = col + pnl - fees = ~1000 + (-500) - fees > liq_threshold (0.5% of 10k = 50)
    let mild_drop = fixture.btc_price(95_000 * PRICE_SCALAR as i64);

    let (users, seqs) = trigger_vecs(&fixture.env, &user, position_id);
    fixture
        .trading
        .execute(&keeper, &FEED_BTC, &users, &seqs, &mild_drop); // should panic #751
}

#[test]
fn test_liquidation_keeper_receives_fee() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open highly leveraged long for liquidation
    let position_id = fixture.open_long(&user, FEED_BTC, 110, 10_000, BTC_PRICE_I64);

    let keeper_balance_before = fixture.token.balance(&keeper);

    // Price drops 3% -- underwater
    fixture.jump(31);
    let crash_price = fixture.btc_price(97_000 * PRICE_SCALAR as i64);

    let (users, seqs) = trigger_vecs(&fixture.env, &user, position_id);
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &crash_price);

    let keeper_balance_after = fixture.token.balance(&keeper);
    assert!(
        keeper_balance_after > keeper_balance_before,
        "Keeper should receive liquidation fee"
    );
}

// ==========================================
// 2. Stale Price Guard (T-DOS-03)
// ==========================================

#[test]
#[should_panic(expected = "Error(Contract, #711)")]
fn test_liquidation_stale_price_rejected() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Jump forward to establish a base timestamp (t=100)
    fixture.jump(100);

    // Build stale price bytes BEFORE opening position (timestamp = t=99)
    let stale_price = pyth_helper::build_price_update(
        &fixture.env,
        &fixture.signing_key,
        &[pyth_helper::FeedInput {
            feed_id: FEED_BTC,
            price: 98_000 * PRICE_SCALAR as i64,
            exponent: -8,
            confidence: 0,
        }],
        99, // publish_time BEFORE position creation
    );

    // Open position at t=100 (created_at = 100)
    let position_id = fixture.open_long(&user, FEED_BTC, 110, 10_000, BTC_PRICE_I64);

    // Try liquidation with stale price (publish_time=99 < created_at=100)
    // Price verifier passes (abs_diff(100, 99) = 1 < max_staleness=60)
    // But trading contract's require_liquidatable rejects: StalePrice (749)
    let (users, seqs) = trigger_vecs(&fixture.env, &user, position_id);
    fixture
        .trading
        .execute(&keeper, &FEED_BTC, &users, &seqs, &stale_price); // should panic #749
}

// ==========================================
// 3. Interest Accrual Impact on Liquidation
// ==========================================

#[test]
fn test_liquidation_after_interest_accrual() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));

    // Open long with 10k collateral, 100k notional (10x leverage)
    let position_id = fixture.open_long(&user, FEED_BTC, 10_000, 100_000, BTC_PRICE_I64);

    // Set funding rate: one-sided long -> rate = base_rate, longs pay
    fixture.jump(3600);
    fixture.trading.apply_funding();

    // A week passes -- significant interest accrual
    fixture.jump(SECONDS_PER_WEEK);

    // Price drops moderately: ~9.07%
    // Without interest: equity = 10000 - 9070 - fees = ~930 > liq_threshold (500)
    // With a week of interest: accrued fees reduce equity below threshold
    let moderate_drop = fixture.btc_price(90_710 * PRICE_SCALAR as i64);

    let (users, seqs) = trigger_vecs(&fixture.env, &user, position_id);
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &moderate_drop);

    assert!(
        !fixture.position_exists(&user, position_id),
        "Position should be liquidatable after interest accrual ate into equity"
    );
}

// ==========================================
// 4. Liquidation After ADL
// ==========================================

#[test]
fn test_liquidation_after_adl() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    let user2 = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    fixture.token.mint(&user2, &(100_000 * SCALAR_7));

    // Open long positions to create a deficit scenario (user2 provides the mass)
    // 5 × 1M notional = 5M total. Price 3x → PnL 10M > vault 10M → ADL
    for _ in 0..5 {
        fixture.open_long(&user2, FEED_BTC, 11_000, 1_000_000, BTC_PRICE_I64);
    }

    // User opens a highly leveraged long: 120 col, 10k notional (~83x)
    let position_id = fixture.open_long(&user, FEED_BTC, 120, 10_000, BTC_PRICE_I64);

    // Price 4x ($400k) -> triggers ADL (longs are winning, vault can't cover)
    let pump_ts = fixture.env.ledger().timestamp() + 31;
    let pump_price = pyth_helper::build_price_update(
        &fixture.env,
        &fixture.signing_key,
        &[
            pyth_helper::FeedInput { feed_id: 1, price: 400_000 * PRICE_SCALAR as i64, exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: 2, price: 200_000_000_000, exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: 3, price: 10_000_000, exponent: -8, confidence: 0 },
        ],
        pump_ts,
    );
    fixture.jump(31);
    fixture.trading.update_status(&pump_price);

    // ADL reduced long notional -- verify the index changed
    let market = fixture.trading.get_market_data(&(FEED_BTC));
    assert!(market.l_adl_idx < 1_000_000_000_000_000_000i128, "ADL should have reduced long index");

    // Now price crashes back below entry -- the ADL-reduced position has less
    // notional to absorb the loss, making it easier to liquidate
    let crash_ts = fixture.env.ledger().timestamp() + 31;
    let crash_price = pyth_helper::build_price_update(
        &fixture.env,
        &fixture.signing_key,
        &[
            pyth_helper::FeedInput { feed_id: 1, price: 97_000 * PRICE_SCALAR as i64, exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: 2, price: 200_000_000_000, exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: 3, price: 10_000_000, exponent: -8, confidence: 0 },
        ],
        crash_ts,
    );
    fixture.jump(31);

    let (users, seqs) = trigger_vecs(&fixture.env, &user, position_id);
    fixture.trading.execute(&keeper, &FEED_BTC, &users, &seqs, &crash_price);

    assert!(
        !fixture.position_exists(&user, position_id),
        "Position should be liquidated after ADL + price reversal"
    );
}
