#![no_main]

//! Dedicated fuzz target for limit order fill paths.
//!
//! Guarantees prerequisite state (valid limit order) then fuzzes price
//! movements to trigger fills, followed by post-fill operations.
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
    scenarios: [FillScenario; 6],
}

#[derive(Arbitrary, Debug)]
struct FillScenario {
    asset_idx: u8,
    collateral_raw: u16,
    leverage_raw: u8,
    is_long: bool,
    offset_bps: u16,
    tp_offset_bps: u16,
    sl_offset_bps: u16,
    price_changes: [i16; 8],
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
    filled: &[u32],
    pending: &[u32],
) {
    // Invariant 1: Token conservation
    let current_total = total_balance(fixture, user);
    assert_eq!(
        initial_total, current_total,
        "Token conservation violated! initial={}, current={}, diff={}",
        initial_total,
        current_total,
        initial_total - current_total
    );

    // Invariant 2: All tracked positions have valid fields
    for &pos_id in filled.iter().chain(pending.iter()) {
        if let Ok(Ok(pos)) = fixture.trading.try_get_position(&pos_id) {
            assert!(pos.collateral > 0, "Position {} has non-positive collateral: {}", pos_id, pos.collateral);
            assert!(pos.notional_size > 0, "Position {} has non-positive notional: {}", pos_id, pos.notional_size);
            assert!(pos.entry_price > 0, "Position {} has non-positive entry_price: {}", pos_id, pos.entry_price);
        }
    }

    // Invariant 3: Zero residual when no positions are open
    if filled.is_empty() && pending.is_empty() {
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
        let offset = (scenario.offset_bps as i128).max(1).min(5000);

        // Correct limit order direction:
        // Long limit: entry_price >= current (contract requires this)
        // Short limit: entry_price <= current (contract requires this)
        let entry_price = if scenario.is_long {
            current_price + current_price * offset / 10_000
        } else {
            current_price - current_price * offset / 10_000
        };

        if entry_price <= 0 {
            continue;
        }

        // Compute TP/SL inline with the limit order
        let tp_bps = (scenario.tp_offset_bps as i128).min(5000);
        let sl_bps = (scenario.sl_offset_bps as i128).min(5000);

        let take_profit = if tp_bps == 0 {
            0i128
        } else if scenario.is_long {
            current_price + current_price * tp_bps.max(offset + 1) / 10_000
        } else {
            let tp = current_price - current_price * tp_bps.max(offset + 1) / 10_000;
            if tp <= 0 { continue; }
            tp
        };

        let stop_loss = if sl_bps == 0 {
            0i128
        } else if scenario.is_long {
            let sl = current_price - current_price * sl_bps / 10_000;
            if sl <= 0 { continue; }
            sl
        } else {
            current_price + current_price * sl_bps / 10_000
        };

        // Step 1: Open the limit order
        let mut filled_positions: Vec<u32> = Vec::new();
        let mut pending_positions: Vec<u32> = Vec::new();

        let result = fixture.trading.try_open_position(
            &user, &asset, &collateral, &notional, &scenario.is_long,
            &entry_price, &take_profit, &stop_loss,
        );
        verify_no_host_error(&result, "OpenLimitOrder");

        if let Ok(Ok((pos_id, _fee))) = result {
            pending_positions.push(pos_id);
        } else {
            continue; // Skip this scenario if order rejected
        }

        check_invariants(&fixture, &user, initial_total, &filled_positions, &pending_positions);

        // Step 2: Apply price changes and attempt fill after each
        for &change_bps in &scenario.price_changes {
            let bps = (change_bps as i128).max(-5000).min(5000);
            let base = prices[asset as usize];
            let new_price = base + base * bps / 10_000;

            if new_price <= 0 {
                continue;
            }

            prices[asset as usize] = new_price;

            // Advance time and refresh oracle
            fixture.jump(60);
            fixture.oracle.set_price_stable(&svec![
                &fixture.env,
                1_0000000,
                prices[0],
                prices[1],
                prices[2],
            ]);

            // Attempt fill on all pending positions
            for i in (0..pending_positions.len()).rev() {
                let pos_id = pending_positions[i];
                let keeper = Address::generate(&fixture.env);
                let result = fixture.trading.try_execute(
                    &keeper,
                    &svec![
                        &fixture.env,
                        ExecuteRequest {
                            request_type: ExecuteRequestType::Fill as u32,
                            position_id: pos_id,
                        }
                    ],
                );
                verify_no_host_error(&result, "ExecuteFill");

                if let Ok(Ok(results)) = result {
                    if results.get(0) == Some(0) {
                        pending_positions.remove(i);
                        filled_positions.push(pos_id);
                    }
                }
            }

            check_invariants(&fixture, &user, initial_total, &filled_positions, &pending_positions);

            // If filled, try post-fill operations
            if !filled_positions.is_empty() {
                let pos_id = filled_positions[0];

                // Try ModifyCollateral (add some)
                if let Ok(Ok(pos)) = fixture.trading.try_get_position(&pos_id) {
                    let delta = 100 * SCALAR_7;
                    let new_collateral = pos.collateral + delta;
                    let result = fixture.trading.try_modify_collateral(&pos_id, &new_collateral);
                    verify_no_host_error(&result, "ModifyCollateral(post-fill)");
                }

                check_invariants(&fixture, &user, initial_total, &filled_positions, &pending_positions);
            }
        }

        // Step 3: Clean up — close all remaining positions
        for &pos_id in filled_positions.iter().chain(pending_positions.iter()) {
            let result = fixture.trading.try_close_position(&pos_id);
            verify_no_host_error(&result, "ClosePosition(cleanup)");
        }
        filled_positions.clear();
        pending_positions.clear();

        check_invariants(&fixture, &user, initial_total, &filled_positions, &pending_positions);
    }
});
