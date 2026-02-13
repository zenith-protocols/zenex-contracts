#![no_main]

//! Stateful fuzz target for the Zenex trading contract.
//!
//! Generates random sequences of 15 operations against a fully initialized
//! trading fixture (oracle, token, vault, trading contract with 3 markets)
//! and asserts protocol invariants after every operation:
//!
//! 1. **Token conservation** — total tokens across all accounts never changes
//! 2. **Position validity** — open positions have positive collateral, notional, entry_price
//! 3. **Zero residual** — contract holds 0 tokens when no positions are open

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::TestFixture;
use test_suites::SCALAR_7;
use trading::{ExecuteRequest, ExecuteRequestType};

// Base oracle prices matching the test fixture setup
const BTC_BASE: i128 = 100_000_0000000; // $100,000
const ETH_BASE: i128 = 2_000_0000000; // $2,000
const XLM_BASE: i128 = 0_1000000; // $0.10

/// Top-level fuzz input: a fixed sequence of 15 random commands.
#[derive(Arbitrary, Debug)]
struct FuzzInput {
    commands: [FuzzCommand; 15],
}

/// A single trading operation to execute against the contract.
#[derive(Arbitrary, Debug, Clone)]
enum FuzzCommand {
    /// Open a new position (market order)
    OpenPosition {
        user_idx: bool,
        asset_idx: u8,
        collateral_raw: u16,
        leverage_raw: u8,
        is_long: bool,
    },
    /// Open a limit order (pending, not immediately filled)
    OpenLimitOrder {
        user_idx: bool,
        asset_idx: u8,
        collateral_raw: u16,
        leverage_raw: u8,
        is_long: bool,
        offset_bps: u16,
    },
    /// Close an existing position (filled or pending)
    ClosePosition {
        position_idx: u8,
    },
    /// Add or remove collateral from an existing position
    ModifyCollateral {
        position_idx: u8,
        amount_raw: u16,
        is_increase: bool,
    },
    /// Set take profit and/or stop loss on a filled position
    SetTriggers {
        position_idx: u8,
        tp_offset_bps: u16,
        sl_offset_bps: u16,
    },
    /// Advance the ledger clock and refresh oracle prices
    PassTime {
        seconds: u16,
    },
    /// Change an asset's oracle price
    UpdatePrice {
        asset_idx: u8,
        change_bps: i16,
    },
    /// Attempt to fill a pending limit order
    ExecuteFill {
        position_idx: u8,
    },
    /// Attempt to execute stop loss on a filled position
    ExecuteStopLoss {
        position_idx: u8,
    },
    /// Attempt to execute take profit on a filled position
    ExecuteTakeProfit {
        position_idx: u8,
    },
    /// Attempt to liquidate a position via the keeper execute() path
    Liquidate {
        position_idx: u8,
    },
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Verify that a try_* result is NOT a host error or raw panic.
///
/// - `Ok(Ok(_))` — success
/// - `Ok(Err(_))` — contract error via `panic_with_error!` → expected validation
/// - `Err(Ok(_))` — Soroban host error → BUG (contract should have caught this)
/// - `Err(Err(_))` — InvokeError / raw panic → BUG (contract should use `panic_with_error!`)
fn verify_no_host_error<T, E: core::fmt::Debug>(
    result: &Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
    context: &str,
) {
    match result {
        Ok(Ok(_)) => {}                     // success
        Ok(Err(_)) => {}                    // known contract error — validation working
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

/// Returns true if the result is a success (Ok(Ok(_))).
fn is_ok<T, E>(result: &Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>) -> bool {
    matches!(result, Ok(Ok(_)))
}

/// Sum of all token balances across every relevant account.
fn total_balance(fixture: &TestFixture, user0: &Address, user1: &Address) -> i128 {
    fixture.token.balance(user0)
        + fixture.token.balance(user1)
        + fixture.token.balance(&fixture.vault.address)
        + fixture.token.balance(&fixture.trading.address)
}

fuzz_target!(|input: FuzzInput| {
    // ===== SETUP =====
    let fixture = create_fixture_with_data(false);

    let user0 = Address::generate(&fixture.env);
    let user1 = Address::generate(&fixture.env);
    let users = [&user0, &user1];

    fixture.token.mint(&user0, &(10_000_000 * SCALAR_7));
    fixture.token.mint(&user1, &(10_000_000 * SCALAR_7));

    let initial_total = total_balance(&fixture, &user0, &user1);

    let mut filled_positions: Vec<u32> = Vec::new();
    let mut pending_positions: Vec<u32> = Vec::new();
    let mut prices = [BTC_BASE, ETH_BASE, XLM_BASE];

    // ===== EXECUTE COMMAND SEQUENCE =====
    for cmd in &input.commands {
        match cmd {
            FuzzCommand::OpenPosition {
                user_idx,
                asset_idx,
                collateral_raw,
                leverage_raw,
                is_long,
            } => {
                let user = users[*user_idx as usize];
                let asset = (*asset_idx % 3) as u32;
                let collateral = ((*collateral_raw as i128).max(10).min(10_000)) * SCALAR_7;
                let leverage = (*leverage_raw as i128).max(2).min(100);
                let notional = collateral * leverage;

                let result = fixture.trading.try_open_position(
                    user, &asset, &collateral, &notional, is_long, &0i128, &0i128, &0i128,
                );
                verify_no_host_error(&result, "OpenPosition");

                if let Ok(Ok((pos_id, _fee))) = result {
                    filled_positions.push(pos_id);
                }
            }

            FuzzCommand::OpenLimitOrder {
                user_idx,
                asset_idx,
                collateral_raw,
                leverage_raw,
                is_long,
                offset_bps,
            } => {
                let user = users[*user_idx as usize];
                let asset = (*asset_idx % 3) as u32;
                let collateral = ((*collateral_raw as i128).max(10).min(10_000)) * SCALAR_7;
                let leverage = (*leverage_raw as i128).max(2).min(100);
                let notional = collateral * leverage;
                let current_price = prices[asset as usize];
                let offset = (*offset_bps as i128).max(1).min(5000);

                // Limit orders: entry_price away from current price
                // Long limit: entry_price >= current (contract requirement)
                // Short limit: entry_price <= current (contract requirement)
                let entry_price = if *is_long {
                    current_price + current_price * offset / 10_000
                } else {
                    current_price - current_price * offset / 10_000
                };

                if entry_price <= 0 {
                    continue;
                }

                let result = fixture.trading.try_open_position(
                    user, &asset, &collateral, &notional, is_long, &entry_price, &0i128, &0i128,
                );
                verify_no_host_error(&result, "OpenLimitOrder");

                if let Ok(Ok((pos_id, _fee))) = result {
                    pending_positions.push(pos_id);
                }
            }

            FuzzCommand::ClosePosition { position_idx } => {
                let all_positions: Vec<u32> = filled_positions.iter()
                    .chain(pending_positions.iter())
                    .copied().collect();

                if all_positions.is_empty() {
                    continue;
                }
                let idx = (*position_idx as usize) % all_positions.len();
                let pos_id = all_positions[idx];

                let result = fixture.trading.try_close_position(&pos_id);
                verify_no_host_error(&result, "ClosePosition");

                if is_ok(&result) {
                    if let Some(i) = filled_positions.iter().position(|&p| p == pos_id) {
                        filled_positions.remove(i);
                    } else if let Some(i) = pending_positions.iter().position(|&p| p == pos_id) {
                        pending_positions.remove(i);
                    }
                }
            }

            FuzzCommand::ModifyCollateral {
                position_idx,
                amount_raw,
                is_increase,
            } => {
                let all_positions: Vec<u32> = filled_positions.iter()
                    .chain(pending_positions.iter())
                    .copied().collect();

                if all_positions.is_empty() {
                    continue;
                }
                let idx = (*position_idx as usize) % all_positions.len();
                let pos_id = all_positions[idx];

                let pos_result = fixture.trading.try_get_position(&pos_id);
                if let Ok(Ok(pos)) = pos_result {
                    let delta = ((*amount_raw as i128).max(1).min(5000)) * SCALAR_7;
                    let new_collateral = if *is_increase {
                        pos.collateral + delta
                    } else {
                        (pos.collateral - delta).max(SCALAR_7)
                    };

                    let result = fixture.trading.try_modify_collateral(&pos_id, &new_collateral);
                    verify_no_host_error(&result, "ModifyCollateral");
                }
            }

            FuzzCommand::SetTriggers {
                position_idx,
                tp_offset_bps,
                sl_offset_bps,
            } => {
                if filled_positions.is_empty() {
                    continue;
                }
                let idx = (*position_idx as usize) % filled_positions.len();
                let pos_id = filled_positions[idx];

                let pos_result = fixture.trading.try_get_position(&pos_id);
                if let Ok(Ok(pos)) = pos_result {
                    let current_price = prices[pos.asset_index as usize];
                    let tp_bps = (*tp_offset_bps as i128).min(5000);
                    let sl_bps = (*sl_offset_bps as i128).min(5000);

                    // Compute trigger prices based on direction
                    // tp_bps == 0 → clear TP (pass 0), sl_bps == 0 → clear SL (pass 0)
                    let take_profit = if tp_bps == 0 {
                        0i128
                    } else if pos.is_long {
                        // Long TP: above current price
                        current_price + current_price * tp_bps / 10_000
                    } else {
                        // Short TP: below current price
                        let tp = current_price - current_price * tp_bps / 10_000;
                        if tp <= 0 { continue; }
                        tp
                    };

                    let stop_loss = if sl_bps == 0 {
                        0i128
                    } else if pos.is_long {
                        // Long SL: below current price
                        let sl = current_price - current_price * sl_bps / 10_000;
                        if sl <= 0 { continue; }
                        sl
                    } else {
                        // Short SL: above current price
                        current_price + current_price * sl_bps / 10_000
                    };

                    let result = fixture.trading.try_set_triggers(&pos_id, &take_profit, &stop_loss);
                    verify_no_host_error(&result, "SetTriggers");
                }
            }

            FuzzCommand::PassTime { seconds } => {
                let secs = (*seconds as u64).max(1).min(86400);
                fixture.jump(secs);

                // Refresh oracle prices to prevent PriceStale errors
                fixture.oracle.set_price_stable(&svec![
                    &fixture.env,
                    1_0000000,
                    prices[0],
                    prices[1],
                    prices[2],
                ]);
            }

            FuzzCommand::UpdatePrice {
                asset_idx,
                change_bps,
            } => {
                let idx = (*asset_idx % 3) as usize;
                let bps = (*change_bps as i128).max(-5000).min(5000);
                let base = prices[idx];
                let new_price = base + base * bps / 10_000;

                if new_price <= 0 {
                    continue;
                }

                prices[idx] = new_price;

                fixture.oracle.set_price_stable(&svec![
                    &fixture.env,
                    1_0000000,
                    prices[0],
                    prices[1],
                    prices[2],
                ]);
            }

            FuzzCommand::ExecuteFill { position_idx } => {
                if pending_positions.is_empty() {
                    continue;
                }
                let idx = (*position_idx as usize) % pending_positions.len();
                let pos_id = pending_positions[idx];

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
                        pending_positions.remove(idx);
                        filled_positions.push(pos_id);
                    }
                }
            }

            FuzzCommand::ExecuteStopLoss { position_idx } => {
                if filled_positions.is_empty() {
                    continue;
                }
                let idx = (*position_idx as usize) % filled_positions.len();
                let pos_id = filled_positions[idx];

                let keeper = Address::generate(&fixture.env);
                let result = fixture.trading.try_execute(
                    &keeper,
                    &svec![
                        &fixture.env,
                        ExecuteRequest {
                            request_type: ExecuteRequestType::StopLoss as u32,
                            position_id: pos_id,
                        }
                    ],
                );
                verify_no_host_error(&result, "ExecuteStopLoss");

                if let Ok(Ok(results)) = result {
                    if results.get(0) == Some(0) {
                        filled_positions.remove(idx);
                    }
                }
            }

            FuzzCommand::ExecuteTakeProfit { position_idx } => {
                if filled_positions.is_empty() {
                    continue;
                }
                let idx = (*position_idx as usize) % filled_positions.len();
                let pos_id = filled_positions[idx];

                let keeper = Address::generate(&fixture.env);
                let result = fixture.trading.try_execute(
                    &keeper,
                    &svec![
                        &fixture.env,
                        ExecuteRequest {
                            request_type: ExecuteRequestType::TakeProfit as u32,
                            position_id: pos_id,
                        }
                    ],
                );
                verify_no_host_error(&result, "ExecuteTakeProfit");

                if let Ok(Ok(results)) = result {
                    if results.get(0) == Some(0) {
                        filled_positions.remove(idx);
                    }
                }
            }

            FuzzCommand::Liquidate { position_idx } => {
                if filled_positions.is_empty() {
                    continue;
                }
                let idx = (*position_idx as usize) % filled_positions.len();
                let pos_id = filled_positions[idx];

                let keeper = Address::generate(&fixture.env);
                let result = fixture.trading.try_execute(
                    &keeper,
                    &svec![
                        &fixture.env,
                        ExecuteRequest {
                            request_type: ExecuteRequestType::Liquidate as u32,
                            position_id: pos_id,
                        }
                    ],
                );
                verify_no_host_error(&result, "Liquidate");

                if let Ok(Ok(results)) = result {
                    if results.get(0) == Some(0) {
                        filled_positions.remove(idx);
                    }
                }
            }
        }

        // ===== INVARIANT CHECKS (after every command) =====

        // Invariant 1: Token conservation
        let current_total = total_balance(&fixture, &user0, &user1);
        assert_eq!(
            initial_total, current_total,
            "Token conservation violated! initial={}, current={}, diff={}",
            initial_total,
            current_total,
            initial_total - current_total
        );

        // Invariant 2: All tracked positions have valid fields
        for &pos_id in filled_positions.iter().chain(pending_positions.iter()) {
            if let Ok(Ok(pos)) = fixture.trading.try_get_position(&pos_id) {
                assert!(
                    pos.collateral > 0,
                    "Position {} has non-positive collateral: {}",
                    pos_id,
                    pos.collateral
                );
                assert!(
                    pos.notional_size > 0,
                    "Position {} has non-positive notional: {}",
                    pos_id,
                    pos.notional_size
                );
                assert!(
                    pos.entry_price > 0,
                    "Position {} has non-positive entry_price: {}",
                    pos_id,
                    pos.entry_price
                );
            }
        }

        // Invariant 3: Zero residual when no positions are open
        if filled_positions.is_empty() && pending_positions.is_empty() {
            let contract_balance = fixture.token.balance(&fixture.trading.address);
            assert_eq!(
                contract_balance, 0,
                "Contract holds {} tokens with no open positions",
                contract_balance
            );
        }
    }
});
