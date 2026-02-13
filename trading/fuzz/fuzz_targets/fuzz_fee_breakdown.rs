#![no_main]

use std::panic::{catch_unwind, AssertUnwindSafe};

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Env};
use trading::testutils::{create_trading, default_market, default_market_data};
use trading::{storage, Market, Position};

const SCALAR_7: i128 = 10_000_000;
const SCALAR_18: i128 = 1_000_000_000_000_000_000;

#[derive(Arbitrary, Debug)]
struct FeeInput {
    is_long: bool,
    /// Notional size [100, 1_000_000] * SCALAR_7
    notional_raw: u32,
    /// Long notional size [0, 10_000_000] * SCALAR_7
    long_notional_raw: u32,
    /// Short notional size [0, 10_000_000] * SCALAR_7
    short_notional_raw: u32,
    /// Interest index delta [0, 100] * SCALAR_18 / 10000
    interest_delta_raw: u16,
}

fuzz_target!(|input: FeeInput| {
    let notional_size = ((input.notional_raw % 1_000_000).max(100) as i128) * SCALAR_7;
    let long_notional = (input.long_notional_raw as i128) * SCALAR_7;
    let short_notional = (input.short_notional_raw as i128) * SCALAR_7;
    let interest_delta = (input.interest_delta_raw as i128) * SCALAR_18 / 10000;

    let e = Env::default();
    let (address, _) = create_trading(&e);
    let user = Address::generate(&e);

    let current_index = SCALAR_18 + interest_delta;
    let position = Position {
        id: 0,
        user,
        filled: true,
        asset_index: 0,
        is_long: input.is_long,
        stop_loss: 0,
        take_profit: 0,
        entry_price: 100_000 * SCALAR_7,
        collateral: notional_size / 10,
        notional_size,
        interest_index: SCALAR_18,
        created_at: 0,
    };

    // Catch panics and distinguish contract errors from unexpected host errors.
    // Contract errors (panic_with_error!) contain "Error(Contract, #" in the message.
    // Anything else (host math overflow, budget) is a bug — the contract should validate first.
    let result = catch_unwind(AssertUnwindSafe(|| {
        e.as_contract(&address, || {
            storage::set_price_decimals(&e, 7);
            storage::set_token_decimals(&e, 7);

            let mut data = default_market_data();
            data.long_notional_size = long_notional;
            data.short_notional_size = short_notional;
            if input.is_long {
                data.long_interest_index = current_index;
            } else {
                data.short_interest_index = current_index;
            }

            let market = Market {
                asset_index: 0,
                config: default_market(&e),
                data,
            };

            position.calculate_fee_breakdown(&e, &market)
        })
    }));

    let fees = match result {
        Ok(fees) => fees,
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

    // Invariant 1: All fee components >= 0
    assert!(fees.base_fee >= 0, "Base fee must be non-negative");
    assert!(fees.impact_fee >= 0, "Impact fee must be non-negative");
    assert!(fees.interest >= 0, "Interest must be non-negative");

    // Invariant 2: base_fee = 0 when minority side (same_side < other_side)
    let same_side = if input.is_long {
        long_notional
    } else {
        short_notional
    };
    let other_side = if input.is_long {
        short_notional
    } else {
        long_notional
    };
    if same_side < other_side {
        assert_eq!(
            fees.base_fee, 0,
            "Minority side should not pay base fee"
        );
    }

    // Invariant 3: Total fee >= 0
    assert!(
        fees.total_fee() >= 0,
        "Total fee must be non-negative"
    );
});
