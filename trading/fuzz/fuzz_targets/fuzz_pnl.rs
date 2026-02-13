#![no_main]

use std::panic::{catch_unwind, AssertUnwindSafe};

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Env};
use trading::Position;

const SCALAR_7: i128 = 10_000_000;

#[derive(Arbitrary, Debug)]
struct PnlInput {
    is_long: bool,
    /// Entry price in range [1, 1_000_000] * SCALAR_7
    entry_price_raw: u32,
    /// Current price in range [1, 1_000_000] * SCALAR_7
    current_price_raw: u32,
    /// Notional size in range [1, 10_000_000] * SCALAR_7
    notional_raw: u32,
}

fuzz_target!(|input: PnlInput| {
    // Clamp to valid ranges
    let entry_price = ((input.entry_price_raw % 1_000_000).max(1) as i128) * SCALAR_7;
    let current_price = ((input.current_price_raw % 1_000_000).max(1) as i128) * SCALAR_7;
    let notional_size = ((input.notional_raw % 10_000_000).max(1) as i128) * SCALAR_7;

    let e = Env::default();
    let user = Address::generate(&e);

    let position = Position {
        id: 0,
        user,
        filled: true,
        asset_index: 0,
        is_long: input.is_long,
        stop_loss: 0,
        take_profit: 0,
        entry_price,
        collateral: notional_size / 10, // 10x leverage
        notional_size,
        interest_index: 0,
        created_at: 0,
    };

    // Catch panics and distinguish contract errors from unexpected host errors.
    // Contract errors (panic_with_error!) contain "Error(Contract, #" in the message.
    // Anything else (host math overflow, budget) is a bug — the contract should validate first.
    let pnl = match catch_unwind(AssertUnwindSafe(|| {
        position.calculate_pnl(&e, current_price, SCALAR_7)
    })) {
        Ok(pnl) => pnl,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("");
            if msg.contains("Error(Contract, #") {
                return; // Known contract error — validation working
            }
            std::panic::resume_unwind(payload);
        }
    };

    // Invariant 1: PnL = 0 when entry == current
    if entry_price == current_price {
        assert_eq!(pnl, 0, "PnL must be 0 when entry == current price");
    }

    // Invariant 2: Long profit when price up, loss when down
    if input.is_long {
        if current_price > entry_price {
            assert!(pnl > 0, "Long should profit when price goes up");
        } else if current_price < entry_price {
            assert!(pnl < 0, "Long should lose when price goes down");
        }
    } else {
        // Short: profit when price down, loss when up
        if current_price < entry_price {
            assert!(pnl > 0, "Short should profit when price goes down");
        } else if current_price > entry_price {
            assert!(pnl < 0, "Short should lose when price goes up");
        }
    }
});
