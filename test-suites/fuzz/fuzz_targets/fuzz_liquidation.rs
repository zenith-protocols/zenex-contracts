#![no_main]

//! Dedicated fuzz target for the liquidation path.
//!
//! Focuses on edge cases that the general fuzzer is unlikely to hit:
//! - Interest-only liquidation (no price movement)
//! - Near-boundary oscillation (price recovers then interest pushes underwater)
//! - Multiple positions on the same market affecting shared interest indices
//!
//! Invariants checked after every operation:
//! 1. **Zero residual** — contract holds 0 tokens when no positions are open
//! 2. **Known errors** — contract errors must be valid TradingError codes

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ScErrorType;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::SCALAR_7;

const BTC_BASE: i128 = 100_000_0000000;
const ETH_BASE: i128 = 2_000_0000000;
const XLM_BASE: i128 = 0_1000000;

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    scenarios: [LiquidationScenario; 4],
}

#[derive(Arbitrary, Debug)]
struct LiquidationScenario {
    asset_idx: u8,
    collateral_raw: u16,
    leverage_raw: u8,
    is_long: bool,
    /// Sequence of actions: time jumps interleaved with price changes and liquidation attempts.
    /// Each step is either a time jump, a price change, or both.
    steps: [LiquidationStep; 12],
}

#[derive(Arbitrary, Debug)]
struct LiquidationStep {
    /// Hours to advance (0-168, capped to 1 week). Each unit = 3600 seconds.
    hours: u8,
    /// Price change in bps (-5000 to 5000). Not biased — allows recovery and oscillation.
    price_change_bps: i16,
    /// Whether to attempt liquidation after this step
    try_liquidate: bool,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// All valid TradingError codes from errors.rs.
const VALID_ERROR_CODES: &[u32] = &[
    1,                                                          // Unauthorized
    700, 701, 702, 703, 704,                                    // Configuration
    710, 711, 712,                                              // Market
    720, 721,                                                   // Oracle/Price
    730, 731, 732, 733, 734, 735, 736, 737, 738, 739, 740,     // Position
    741, 742, 743, 744, 745, 746, 747, 748,                     // Position (cont)
    750, 751,                                                   // Action/Request
    760, 761, 762,                                              // Status
    770,                                                        // Utilization
];

/// Verify that a try_* result is not a host error, raw panic, or unknown error code.
///
/// In WASM mode, contract errors from `panic_with_error!` arrive as
/// `Err(Ok(Error))` with `ScErrorType::Contract` — these are expected validation.
/// Only non-contract error types (WasmVm, Budget, Storage, etc.) indicate a bug.
fn verify_no_host_error<T, E: core::fmt::Debug>(
    result: &Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
    context: &str,
) {
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => {}
        Err(Ok(e)) if e.is_type(ScErrorType::Contract) => {
            let code = e.get_code();
            assert!(
                VALID_ERROR_CODES.contains(&code),
                "[{}] Unknown contract error code: {} — not a valid TradingError",
                context, code
            );
        }
        Err(Ok(e)) => panic!(
            "[{}] Host error: {:?} — not a contract error, likely a VM/budget/storage issue",
            context, e
        ),
        Err(Err(e)) => panic!(
            "[{}] InvokeError: {:?} — contract should use panic_with_error!, not raw panic",
            context, e
        ),
    }
}

fn check_invariants(fixture: &test_suites::test_fixture::TestFixture, positions: &[u32]) {
    // Zero residual: contract must hold 0 tokens when no positions are open
    if positions.is_empty() {
        let contract_balance = fixture.token.balance(&fixture.trading.address);
        assert_eq!(
            contract_balance, 0,
            "Contract holds {} tokens with no open positions",
            contract_balance
        );
    }
}

fuzz_target!(|input: FuzzInput| {
    let fixture = create_fixture_with_data(true);

    let user = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    let mut prices = [BTC_BASE, ETH_BASE, XLM_BASE];

    for scenario in &input.scenarios {
        let asset = (scenario.asset_idx % 3) as u32;
        let collateral = ((scenario.collateral_raw as i128).max(10).min(10_000)) * SCALAR_7;
        // Range from low leverage (where interest slowly erodes margin)
        // to high leverage (where small price moves liquidate)
        let leverage = (scenario.leverage_raw as i128).max(2).min(100);
        let notional = collateral * leverage;

        // Open limit order at current oracle price, then fill to simulate market order
        let entry_price = prices[asset as usize];
        let result = fixture.trading.try_open_position(
            &user, &asset, &collateral, &notional, &scenario.is_long,
            &entry_price, &0i128, &0i128,
        );
        verify_no_host_error(&result, "OpenPosition");

        let pos_id = if let Ok(Ok((id, _fee))) = result {
            id
        } else {
            continue;
        };

        // Fill the pending limit order
        let fill_result = fixture.trading.try_execute(
            &user,
            &svec![&fixture.env, pos_id],
        );
        verify_no_host_error(&fill_result, "FillPosition");

        // If fill failed, clean up and continue
        if let Ok(Ok(results)) = &fill_result {
            if results.get(0) != Some(0) {
                let _ = fixture.trading.try_close_position(&pos_id);
                continue;
            }
        } else {
            let _ = fixture.trading.try_close_position(&pos_id);
            continue;
        }

        let mut positions: Vec<u32> = vec![pos_id];
        let mut liquidated = false;

        check_invariants(&fixture, &positions);

        for step in &scenario.steps {
            if liquidated {
                break;
            }

            // Time jump in 1-hour intervals, up to 1 week (168 hours)
            let hours = (step.hours as u64).min(168).max(1);
            fixture.jump(hours * 3600);
            fixture.oracle.set_price_stable(&svec![
                &fixture.env,
                1_0000000,
                prices[0],
                prices[1],
                prices[2],
            ]);

            // Price change: unbiased — allows recovery and oscillation
            let bps = (step.price_change_bps as i128).max(-5000).min(5000);
            if bps != 0 {
                let base = prices[asset as usize];
                let new_price = base + base * bps / 10_000;

                if new_price > 0 {
                    prices[asset as usize] = new_price;
                    fixture.oracle.set_price_stable(&svec![
                        &fixture.env,
                        1_0000000,
                        prices[0],
                        prices[1],
                        prices[2],
                    ]);
                }
            }

            // Conditionally attempt liquidation
            if step.try_liquidate {
                let liq_result = fixture.trading.try_execute(
                    &keeper,
                    &svec![&fixture.env, pos_id],
                );
                verify_no_host_error(&liq_result, "Liquidate");

                if let Ok(Ok(results)) = &liq_result {
                    if results.get(0) == Some(0) {
                        positions.clear();
                        liquidated = true;
                    }
                }
            }

            check_invariants(&fixture, &positions);
        }

        // Clean up if not liquidated
        if !liquidated {
            for &pid in &positions {
                let result = fixture.trading.try_close_position(&pid);
                verify_no_host_error(&result, "ClosePosition(cleanup)");
            }
            positions.clear();
            check_invariants(&fixture, &positions);
        }
    }
});
