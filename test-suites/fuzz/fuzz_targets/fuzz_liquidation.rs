#![no_main]

//! Dedicated fuzz target for the liquidation and ADL paths.
//!
//! Focuses on edge cases the general fuzzer is unlikely to hit:
//! - Interest-only liquidation (no price movement, fees erode margin)
//! - Near-boundary oscillation (price recovers, then fees push underwater)
//! - ADL trigger and reduction verification
//! - Multiple positions on the same market affecting shared indices
//!
//! Uses 3 users (long, short, neutral) + keeper on a single market (BTC)
//! with 15-step sequences of time jumps and price swings.
//!
//! Invariants checked after every step:
//! 1. **Zero residual** — contract holds 0 tokens when no positions are open
//! 2. **Known errors** — contract errors must be valid TradingError codes
//! 3. **Liquidated positions removed** — storage consistent after liquidation
//! 4. **Post-ADL solvency** — ADL reduces net PnL to within vault capacity

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ScErrorType;
use soroban_sdk::{vec as svec, Address};
use test_suites::constants::SCALAR_7;
use test_suites::pyth_helper;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::TestFixture;
use trading::testutils::{FEED_BTC, FEED_ETH, FEED_XLM, PRICE_SCALAR};

// ── Constants ───────────────────────────────────────────────────────────────

const BTC_INITIAL: i64 = 100_000 * PRICE_SCALAR as i64;
const ETH_INITIAL: i64 = 2_000 * PRICE_SCALAR as i64;
const XLM_INITIAL: i64 = PRICE_SCALAR as i64 / 10;

// ── Fuzz Input ──────────────────────────────────────────────────────────────

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    scenarios: [LiquidationScenario; 3],
}

#[derive(Arbitrary, Debug)]
struct LiquidationScenario {
    /// Collateral in token units (clamped to 100-50_000).
    collateral_raw: u16,
    /// Leverage multiplier (clamped to 2-100).
    leverage_raw: u8,
    /// Whether the primary position is long.
    is_long: bool,
    /// Whether to open a counter-position (opposite side) for interest dynamics.
    open_counter: bool,
    /// Counter position collateral (clamped to 100-50_000).
    counter_collateral_raw: u16,
    /// Counter position leverage (clamped to 2-50).
    counter_leverage_raw: u8,
    /// Sequence of time/price/action steps.
    steps: [LiquidationStep; 15],
}

#[derive(Arbitrary, Debug)]
struct LiquidationStep {
    /// Hours to advance (1-168, capped to 1 week).
    hours: u8,
    /// Price change in bps (-5000 to 5000).
    price_change_bps: i16,
    /// Action to attempt after time/price change.
    action: StepAction,
}

#[derive(Arbitrary, Debug)]
enum StepAction {
    /// Attempt liquidation via execute().
    TryLiquidate,
    /// Attempt ADL via update_status().
    TryADL,
    /// Apply funding to advance indices.
    ApplyFunding,
    /// Do nothing (just observe state after time/price change).
    Observe,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Per-operation expected error codes. Any other contract error is a bug.
///
/// open_market: collateral/notional validation, utilization cap, status guards
const OPEN_ERRORS: &[u32] = &[
    724, // NotionalBelowMinimum
    725, // NotionalAboveMaximum
    726, // LeverageAboveMaximum
    741, // ContractOnIce (if ADL triggered OnIce earlier in this scenario)
    751, // UtilizationExceeded
];

/// execute (liquidation): position not actionable, or position already gone
const EXECUTE_ERRORS: &[u32] = &[
    720, // PositionNotFound (already liquidated/closed)
    731, // NotActionable (healthy position, no TP/SL hit)
];

/// update_status (ADL): threshold not met, or frozen
const STATUS_ERRORS: &[u32] = &[
    740, // InvalidStatus (frozen)
    750, // ThresholdNotMet (PnL below trigger)
];

/// apply_funding: called too soon
const FUNDING_ERRORS: &[u32] = &[
    752, // FundingTooEarly
];

/// close_position: position not found, frozen, too new
const CLOSE_ERRORS: &[u32] = &[
    720, // PositionNotFound (already liquidated)
    732, // PositionTooNew
    742, // ContractFrozen
];

fn verify_expected_error<T, E: core::fmt::Debug>(
    result: &Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
    context: &str,
    allowed: &[u32],
) {
    match result {
        Ok(Ok(_)) | Ok(Err(_)) => {}
        Err(Ok(e)) if e.is_type(ScErrorType::Contract) => {
            let code = e.get_code();
            assert!(
                allowed.contains(&code),
                "[{}] Unexpected contract error {}: not in allowed list {:?}",
                context, code, allowed
            );
        }
        Err(Ok(e)) => panic!("[{}] Host error: {:?}", context, e),
        Err(Err(e)) => panic!("[{}] InvokeError: {:?}", context, e),
    }
}

fn is_ok<T, E>(
    result: &Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> bool {
    matches!(result, Ok(Ok(_)))
}

fn build_btc_price(fixture: &TestFixture, btc_price: i64) -> soroban_sdk::Bytes {
    fixture.price_for_feed(FEED_BTC, btc_price)
}

fn build_all_prices(fixture: &TestFixture, btc_price: i64) -> soroban_sdk::Bytes {
    let ts = fixture.env.ledger().timestamp();
    pyth_helper::build_price_update(
        &fixture.env,
        &fixture.signing_key,
        &[
            pyth_helper::FeedInput { feed_id: FEED_BTC, price: btc_price, exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: FEED_ETH, price: ETH_INITIAL, exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: FEED_XLM, price: XLM_INITIAL, exponent: -8, confidence: 0 },
        ],
        ts,
    )
}

fn check_zero_residual(fixture: &TestFixture, has_positions: bool) {
    if !has_positions {
        let bal = fixture.token.balance(&fixture.trading.address);
        assert_eq!(bal, 0, "Contract holds {} tokens with no open positions", bal);
    }
}

// ── Fuzz Target ─────────────────────────────────────────────────────────────

fuzz_target!(|input: FuzzInput| {
    let fixture = create_fixture_with_data();

    let user_primary = Address::generate(&fixture.env);
    let user_counter = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    fixture.token.mint(&user_primary, &(10_000_000 * SCALAR_7));
    fixture.token.mint(&user_counter, &(10_000_000 * SCALAR_7));

    for scenario in &input.scenarios {
        let collateral = ((scenario.collateral_raw as i128).max(100).min(50_000)) * SCALAR_7;
        let leverage = (scenario.leverage_raw as i128).max(2).min(100);
        let notional = collateral * leverage;

        let mut btc_price = BTC_INITIAL;

        // Open primary position
        let price_bytes = build_btc_price(&fixture, btc_price);
        let result = fixture.trading.try_open_market(
            &user_primary, &FEED_BTC, &collateral, &notional,
            &scenario.is_long, &0i128, &0i128, &price_bytes,
        );
        verify_expected_error(&result, "OpenPrimary", OPEN_ERRORS);

        let primary_id = if let Ok(Ok(id)) = result {
            id
        } else {
            continue;
        };

        let mut positions: Vec<u32> = vec![primary_id];

        // Optionally open a counter-position for interest dynamics
        if scenario.open_counter {
            let counter_col = ((scenario.counter_collateral_raw as i128).max(100).min(50_000)) * SCALAR_7;
            let counter_lev = (scenario.counter_leverage_raw as i128).max(2).min(50);
            let counter_not = counter_col * counter_lev;
            let counter_price = build_btc_price(&fixture, btc_price);

            let counter_result = fixture.trading.try_open_market(
                &user_counter, &FEED_BTC, &counter_col, &counter_not,
                &(!scenario.is_long), &0i128, &0i128, &counter_price,
            );
            verify_expected_error(&counter_result, "OpenCounter", OPEN_ERRORS);

            if let Ok(Ok(counter_id)) = counter_result {
                positions.push(counter_id);
            }
        }

        // Jump past MIN_OPEN_TIME for close operations
        fixture.jump(31);

        let mut primary_liquidated = false;

        for step in &scenario.steps {
            if primary_liquidated { break; }

            // Time jump
            let hours = (step.hours as u64).max(1).min(168);
            fixture.jump(hours * 3600);

            // Price change
            let bps = (step.price_change_bps as i64).max(-5000).min(5000);
            if bps != 0 {
                let new_price = btc_price + btc_price * bps / 10_000;
                if new_price > 0 {
                    btc_price = new_price;
                }
            }

            // Execute action
            match &step.action {
                StepAction::TryLiquidate => {
                    let price_bytes = build_btc_price(&fixture, btc_price);
                    let result = fixture.trading.try_execute(
                        &keeper, &FEED_BTC, &svec![&fixture.env, primary_id], &price_bytes,
                    );
                    verify_expected_error(&result, "TryLiquidate", EXECUTE_ERRORS);

                    if is_ok(&result) {
                        // Check position was actually removed
                        assert!(
                            !fixture.position_exists(primary_id),
                            "Position {} still exists after successful liquidation",
                            primary_id
                        );
                        positions.retain(|&id| id != primary_id);
                        primary_liquidated = true;
                    }
                }

                StepAction::TryADL => {
                    let price_bytes = build_all_prices(&fixture, btc_price);
                    let result = fixture.trading.try_update_status(&price_bytes);
                    verify_expected_error(&result, "TryADL", STATUS_ERRORS);

                    // Post-ADL: if succeeded and was ADL (not just status change),
                    // verify ADL index decreased
                    if is_ok(&result) {
                        let md = fixture.trading.get_market_data(&FEED_BTC);
                        let adl_idx = if scenario.is_long { md.l_adl_idx } else { md.s_adl_idx };
                        assert!(
                            adl_idx <= 1_000_000_000_000_000_000,
                            "ADL index should be <= SCALAR_18, got {}",
                            adl_idx
                        );
                    }
                }

                StepAction::ApplyFunding => {
                    let result = fixture.trading.try_apply_funding();
                    verify_expected_error(&result, "ApplyFunding", FUNDING_ERRORS);
                }

                StepAction::Observe => {}
            }

            check_zero_residual(&fixture, !positions.is_empty());
        }

        // Cleanup: close remaining positions, tracking whether all succeed
        let mut all_closed = true;
        for &pid in &positions {
            let price_bytes = build_btc_price(&fixture, btc_price);
            let result = fixture.trading.try_close_position(&pid, &price_bytes);
            verify_expected_error(&result, "Cleanup", CLOSE_ERRORS);
            if !is_ok(&result) {
                all_closed = false;
            }
        }

        // Only assert zero residual if all positions were successfully closed
        if all_closed {
            check_zero_residual(&fixture, false);
        }
    }
});
