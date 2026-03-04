#![no_main]

//! Stateful fuzz target for the Zenex trading contract core flow.
//!
//! Focuses on the operations that matter for the general path:
//! open → modify collateral / price change / time jump → close
//!
//! Trigger-based paths (TP/SL, fill, liquidation) require specific price
//! conditions that random movements almost never hit — those are covered
//! by the dedicated fuzz_liquidation target instead.
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

// Base oracle prices matching the test fixture setup
const BTC_BASE: i128 = 100_000_0000000; // $100,000
const ETH_BASE: i128 = 2_000_0000000; // $2,000
const XLM_BASE: i128 = 0_1000000; // $0.10

/// Top-level fuzz input: a fixed sequence of 20 random commands.
#[derive(Arbitrary, Debug)]
struct FuzzInput {
    commands: [FuzzCommand; 20],
}

/// A single trading operation to execute against the contract.
#[derive(Arbitrary, Debug, Clone)]
enum FuzzCommand {
    /// Open a new market position
    OpenPosition {
        user_idx: bool,
        asset_idx: u8,
        collateral_raw: u16,
        leverage_raw: u8,
        is_long: bool,
    },
    /// Close an existing position
    ClosePosition {
        position_idx: u8,
    },
    /// Add or remove collateral from an existing position
    ModifyCollateral {
        position_idx: u8,
        amount_raw: u16,
        is_increase: bool,
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

/// Returns true if the result is a success.
fn is_ok<T, E>(
    result: &Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> bool {
    matches!(result, Ok(Ok(_)))
}

fuzz_target!(|input: FuzzInput| {
    // ===== SETUP =====
    let fixture = create_fixture_with_data(true);

    let user0 = Address::generate(&fixture.env);
    let user1 = Address::generate(&fixture.env);
    let users = [&user0, &user1];

    fixture.token.mint(&user0, &(10_000_000 * SCALAR_7));
    fixture.token.mint(&user1, &(10_000_000 * SCALAR_7));

    let mut positions: Vec<u32> = Vec::new();
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

                // Use current oracle price as entry_price so the limit order is immediately fillable
                let entry_price = prices[asset as usize];
                let result = fixture.trading.try_open_position(
                    user, &asset, &collateral, &notional, is_long, &entry_price, &0i128, &0i128,
                );
                verify_no_host_error(&result, "OpenPosition");

                if let Ok(Ok((pos_id, _fee))) = result {
                    // Fill the pending limit order immediately
                    let fill_result = fixture.trading.try_execute(
                        user,
                        &svec![
                            &fixture.env,
                            trading::ExecuteRequest {
                                request_type: 0, // Fill
                                position_id: pos_id,
                            },
                        ],
                    );
                    verify_no_host_error(&fill_result, "FillPosition");

                    let filled = if let Ok(Ok(results)) = &fill_result {
                        results.get(0) == Some(0)
                    } else {
                        false
                    };

                    if filled {
                        positions.push(pos_id);
                    } else {
                        // Cancel the unfilled limit order to avoid residual balance
                        let _ = fixture.trading.try_close_position(&pos_id);
                    }
                }
            }

            FuzzCommand::ClosePosition { position_idx } => {
                if positions.is_empty() {
                    continue;
                }
                let idx = (*position_idx as usize) % positions.len();
                let pos_id = positions[idx];

                let result = fixture.trading.try_close_position(&pos_id);
                verify_no_host_error(&result, "ClosePosition");

                if is_ok(&result) {
                    positions.remove(idx);
                }
            }

            FuzzCommand::ModifyCollateral {
                position_idx,
                amount_raw,
                is_increase,
            } => {
                if positions.is_empty() {
                    continue;
                }
                let idx = (*position_idx as usize) % positions.len();
                let pos_id = positions[idx];

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
        }

        // ===== INVARIANT CHECK =====
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
});
