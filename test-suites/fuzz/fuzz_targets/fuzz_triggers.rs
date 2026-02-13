#![no_main]

//! Dedicated fuzz target for stop loss and take profit execution paths.
//!
//! Opens market positions with TP/SL set inline, then fuzzes price movements
//! biased toward trigger levels to exercise the trigger execution code.
//!
//! Invariants checked after every operation:
//! 1. Token conservation
//! 2. Position validity (positive collateral, notional, entry_price)
//! 3. Zero residual when no positions are open

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::TestFixture;
use test_suites::SCALAR_7;
use trading::{ExecuteRequest, ExecuteRequestType};

const BTC_BASE: i128 = 100_000_0000000;
const ETH_BASE: i128 = 2_000_0000000;
const XLM_BASE: i128 = 0_1000000;

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    scenarios: [TriggerScenario; 6],
}

#[derive(Arbitrary, Debug)]
struct TriggerScenario {
    asset_idx: u8,
    collateral_raw: u16,
    leverage_raw: u8,
    is_long: bool,
    tp_offset_bps: u16,
    sl_offset_bps: u16,
    price_changes: [i16; 10],
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn verify_no_host_error<T, E: core::fmt::Debug>(
    result: &Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
    context: &str,
) {
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => {}
        Err(Ok(e)) => panic!(
            "[{}] Host error (Err(Ok)): {:?} — contract validation missed bad input",
            context, e
        ),
        Err(Err(e)) => panic!(
            "[{}] InvokeError (Err(Err)): {:?} — contract should use panic_with_error!, not raw panic",
            context, e
        ),
    }
}

fn total_balance(fixture: &TestFixture, user: &Address) -> i128 {
    fixture.token.balance(user)
        + fixture.token.balance(&fixture.vault.address)
        + fixture.token.balance(&fixture.trading.address)
}

fn check_invariants(
    fixture: &TestFixture,
    user: &Address,
    initial_total: i128,
    positions: &[u32],
) {
    let current_total = total_balance(fixture, user);
    assert_eq!(
        initial_total, current_total,
        "Token conservation violated! initial={}, current={}, diff={}",
        initial_total,
        current_total,
        initial_total - current_total
    );

    for &pos_id in positions.iter() {
        if let Ok(Ok(pos)) = fixture.trading.try_get_position(&pos_id) {
            assert!(pos.collateral > 0, "Position {} has non-positive collateral: {}", pos_id, pos.collateral);
            assert!(pos.notional_size > 0, "Position {} has non-positive notional: {}", pos_id, pos.notional_size);
            assert!(pos.entry_price > 0, "Position {} has non-positive entry_price: {}", pos_id, pos.entry_price);
        }
    }

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
    let fixture = create_fixture_with_data(false);

    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    let initial_total = total_balance(&fixture, &user);
    let mut prices = [BTC_BASE, ETH_BASE, XLM_BASE];

    for scenario in &input.scenarios {
        let asset = (scenario.asset_idx % 3) as u32;
        let collateral = ((scenario.collateral_raw as i128).max(10).min(10_000)) * SCALAR_7;
        let leverage = (scenario.leverage_raw as i128).max(2).min(100);
        let notional = collateral * leverage;
        let current_price = prices[asset as usize];

        let tp_bps = (scenario.tp_offset_bps as i128).max(100).min(5000);
        let sl_bps = (scenario.sl_offset_bps as i128).max(100).min(5000);

        // TP/SL validation:
        // Long TP: must be > current_price; Long SL: must be < current_price
        // Short TP: must be < current_price; Short SL: must be > current_price
        let take_profit = if scenario.is_long {
            current_price + current_price * tp_bps / 10_000
        } else {
            let tp = current_price - current_price * tp_bps / 10_000;
            if tp <= 0 { continue; }
            tp
        };

        let stop_loss = if scenario.is_long {
            let sl = current_price - current_price * sl_bps / 10_000;
            if sl <= 0 { continue; }
            sl
        } else {
            current_price + current_price * sl_bps / 10_000
        };

        // Open market position (entry_price=0) with TP/SL inline
        let result = fixture.trading.try_open_position(
            &user, &asset, &collateral, &notional, &scenario.is_long,
            &0i128, &take_profit, &stop_loss,
        );
        verify_no_host_error(&result, "OpenPosition");

        let pos_id = if let Ok(Ok((id, _fee))) = result {
            id
        } else {
            continue;
        };

        let mut positions: Vec<u32> = vec![pos_id];
        let mut triggered = false;

        check_invariants(&fixture, &user, initial_total, &positions);

        // Apply price changes and attempt both SL and TP after each
        for &change_bps in &scenario.price_changes {
            if triggered {
                break;
            }

            // Bias price changes toward trigger levels:
            // Use the raw fuzz value but allow it to push toward triggers
            let bps = (change_bps as i128).max(-5000).min(5000);
            let base = prices[asset as usize];
            let new_price = base + base * bps / 10_000;

            if new_price <= 0 {
                continue;
            }

            prices[asset as usize] = new_price;

            fixture.jump(60);
            fixture.oracle.set_price_stable(&svec![
                &fixture.env,
                1_0000000,
                prices[0],
                prices[1],
                prices[2],
            ]);

            // Attempt StopLoss
            let keeper = Address::generate(&fixture.env);
            let sl_result = fixture.trading.try_execute(
                &keeper,
                &svec![
                    &fixture.env,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::StopLoss as u32,
                        position_id: pos_id,
                    }
                ],
            );
            verify_no_host_error(&sl_result, "ExecuteStopLoss");

            if let Ok(Ok(results)) = &sl_result {
                if results.get(0) == Some(0) {
                    positions.clear();
                    triggered = true;
                    check_invariants(&fixture, &user, initial_total, &positions);
                    continue;
                }
            }

            // Attempt TakeProfit
            let keeper = Address::generate(&fixture.env);
            let tp_result = fixture.trading.try_execute(
                &keeper,
                &svec![
                    &fixture.env,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::TakeProfit as u32,
                        position_id: pos_id,
                    }
                ],
            );
            verify_no_host_error(&tp_result, "ExecuteTakeProfit");

            if let Ok(Ok(results)) = &tp_result {
                if results.get(0) == Some(0) {
                    positions.clear();
                    triggered = true;
                    check_invariants(&fixture, &user, initial_total, &positions);
                    continue;
                }
            }

            check_invariants(&fixture, &user, initial_total, &positions);
        }

        // Clean up if not triggered
        if !triggered {
            for &pid in &positions {
                let result = fixture.trading.try_close_position(&pid);
                verify_no_host_error(&result, "ClosePosition(cleanup)");
            }
            positions.clear();
            check_invariants(&fixture, &user, initial_total, &positions);
        }
    }
});
