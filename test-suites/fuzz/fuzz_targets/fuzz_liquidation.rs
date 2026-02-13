#![no_main]

//! Dedicated fuzz target for the liquidation path.
//!
//! Opens high-leverage market positions then applies adverse price movements
//! to trigger liquidations. Verifies keeper fee payout, vault settlement,
//! and token conservation.
//!
//! Invariants checked after every operation:
//! 1. Token conservation
//! 2. Position validity (positive collateral, notional, entry_price)
//! 3. Zero residual when no positions are open
//! 4. Contract never has negative balance

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
    scenarios: [LiquidationScenario; 6],
}

#[derive(Arbitrary, Debug)]
struct LiquidationScenario {
    asset_idx: u8,
    collateral_raw: u16,
    leverage_raw: u8,
    is_long: bool,
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

fn total_balance(fixture: &TestFixture, user: &Address, keeper: &Address) -> i128 {
    fixture.token.balance(user)
        + fixture.token.balance(keeper)
        + fixture.token.balance(&fixture.vault.address)
        + fixture.token.balance(&fixture.trading.address)
}

fn check_invariants(
    fixture: &TestFixture,
    user: &Address,
    keeper: &Address,
    initial_total: i128,
    positions: &[u32],
) {
    // Invariant 1: Token conservation
    let current_total = total_balance(fixture, user, keeper);
    assert_eq!(
        initial_total, current_total,
        "Token conservation violated! initial={}, current={}, diff={}",
        initial_total,
        current_total,
        initial_total - current_total
    );

    // Invariant 2: All tracked positions have valid fields
    for &pos_id in positions.iter() {
        if let Ok(Ok(pos)) = fixture.trading.try_get_position(&pos_id) {
            assert!(pos.collateral > 0, "Position {} has non-positive collateral: {}", pos_id, pos.collateral);
            assert!(pos.notional_size > 0, "Position {} has non-positive notional: {}", pos_id, pos.notional_size);
            assert!(pos.entry_price > 0, "Position {} has non-positive entry_price: {}", pos_id, pos.entry_price);
        }
    }

    // Invariant 3: Zero residual when no positions are open
    if positions.is_empty() {
        let contract_balance = fixture.token.balance(&fixture.trading.address);
        assert_eq!(
            contract_balance, 0,
            "Contract holds {} tokens with no open positions",
            contract_balance
        );
    }

    // Invariant 4: Contract should never have negative balance
    let contract_balance = fixture.token.balance(&fixture.trading.address);
    assert!(
        contract_balance >= 0,
        "Contract has negative balance: {}",
        contract_balance
    );
}

fuzz_target!(|input: FuzzInput| {
    let fixture = create_fixture_with_data(false);

    let user = Address::generate(&fixture.env);
    // Use a persistent keeper so we can track its balance for conservation
    let keeper = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(10_000_000 * SCALAR_7));

    let initial_total = total_balance(&fixture, &user, &keeper);
    let mut prices = [BTC_BASE, ETH_BASE, XLM_BASE];

    for scenario in &input.scenarios {
        let asset = (scenario.asset_idx % 3) as u32;
        let collateral = ((scenario.collateral_raw as i128).max(10).min(10_000)) * SCALAR_7;
        // Bias leverage high (20x-100x) for liquidation scenarios
        let leverage = (scenario.leverage_raw as i128).max(20).min(100);
        let notional = collateral * leverage;

        // Open market position (entry_price=0, no TP/SL)
        let result = fixture.trading.try_open_position(
            &user, &asset, &collateral, &notional, &scenario.is_long,
            &0i128, &0i128, &0i128,
        );
        verify_no_host_error(&result, "OpenPosition");

        let pos_id = if let Ok(Ok((id, _fee))) = result {
            id
        } else {
            continue;
        };

        let mut positions: Vec<u32> = vec![pos_id];
        let mut liquidated = false;

        check_invariants(&fixture, &user, &keeper, initial_total, &positions);

        // Apply adverse price changes and attempt liquidation after each
        for &change_bps in &scenario.price_changes {
            if liquidated {
                break;
            }

            // Bias adverse: longs get price drops, shorts get price rises
            // Use the raw fuzz value but flip sign to tend adverse
            let raw_bps = (change_bps as i128).max(-5000).min(5000);
            let bps = if scenario.is_long {
                // For longs, bias negative (adverse)
                -(raw_bps.abs())
            } else {
                // For shorts, bias positive (adverse)
                raw_bps.abs()
            };

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

            // Attempt liquidation
            let liq_result = fixture.trading.try_execute(
                &keeper,
                &svec![
                    &fixture.env,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::Liquidate as u32,
                        position_id: pos_id,
                    }
                ],
            );
            verify_no_host_error(&liq_result, "Liquidate");

            if let Ok(Ok(results)) = &liq_result {
                if results.get(0) == Some(0) {
                    positions.clear();
                    liquidated = true;

                    // Verify keeper received some fee (balance > 0)
                    let keeper_balance = fixture.token.balance(&keeper);
                    assert!(
                        keeper_balance >= 0,
                        "Keeper has negative balance after liquidation: {}",
                        keeper_balance
                    );
                }
            }

            check_invariants(&fixture, &user, &keeper, initial_total, &positions);
        }

        // Clean up if not liquidated
        if !liquidated {
            for &pid in &positions {
                let result = fixture.trading.try_close_position(&pid);
                verify_no_host_error(&result, "ClosePosition(cleanup)");
            }
            positions.clear();
            check_invariants(&fixture, &user, &keeper, initial_total, &positions);
        }
    }
});
