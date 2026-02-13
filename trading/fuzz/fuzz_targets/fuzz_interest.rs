#![no_main]

use std::panic::{catch_unwind, AssertUnwindSafe};

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::Env;
use trading::testutils::calc_interest_for_test;

const SCALAR_18: i128 = 1_000_000_000_000_000_000;

#[derive(Arbitrary, Debug)]
struct InterestInput {
    /// Long notional [0, 10_000_000] * SCALAR_18
    long_raw: u32,
    /// Short notional [0, 10_000_000] * SCALAR_18
    short_raw: u32,
    /// Base rate [1, 1_000_000] in raw (maps to reasonable hourly rates)
    base_rate_raw: u32,
    /// Ratio cap [1x, 10x] as raw multiplier
    ratio_cap_raw: u8,
}

fuzz_target!(|input: InterestInput| {
    let long_notional = (input.long_raw as i128) * SCALAR_18;
    let short_notional = (input.short_raw as i128) * SCALAR_18;
    let base_rate = ((input.base_rate_raw % 1_000_000).max(1) as i128) * 10_000_000; // Scale to reasonable rates
    let ratio_cap = ((input.ratio_cap_raw % 10).max(1) as i128) * SCALAR_18;

    let e = Env::default();

    // Catch panics and distinguish contract errors from unexpected host errors.
    // Contract errors (panic_with_error!) contain "Error(Contract, #" in the message.
    // Anything else (host math overflow, budget) is a bug — the contract should validate first.
    let (long_rate, short_rate) = match catch_unwind(AssertUnwindSafe(|| {
        calc_interest_for_test(&e, long_notional, short_notional, base_rate, ratio_cap)
    })) {
        Ok(rates) => rates,
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

    // Invariant 1: Both rates = 0 when no positions
    if long_notional == 0 && short_notional == 0 {
        assert_eq!(long_rate, 0, "Long rate must be 0 with no positions");
        assert_eq!(short_rate, 0, "Short rate must be 0 with no positions");
    }

    // Invariant 2: Balanced market → both pay base_rate
    if long_notional > 0 && long_notional == short_notional {
        assert_eq!(long_rate, base_rate, "Balanced: long should pay base_rate");
        assert_eq!(
            short_rate, base_rate,
            "Balanced: short should pay base_rate"
        );
    }

    // Invariant 3: Dominant side always pays (positive rate), minority receives (negative rate)
    if long_notional > 0 && short_notional > 0 && long_notional != short_notional {
        if long_notional > short_notional {
            assert!(long_rate > 0, "Dominant long side should pay (positive)");
            assert!(short_rate < 0, "Minority short side should receive (negative)");
        } else {
            assert!(long_rate < 0, "Minority long side should receive (negative)");
            assert!(short_rate > 0, "Dominant short side should pay (positive)");
        }
    }

    // Invariant 4: Rates capped - dominant rate <= base_rate * ratio_cap / SCALAR_18
    if long_notional > 0 && short_notional > 0 {
        let max_rate = base_rate * ratio_cap / SCALAR_18;
        if long_notional > short_notional {
            assert!(
                long_rate <= max_rate + 1, // +1 for rounding
                "Long rate {} should be <= max_rate {}",
                long_rate,
                max_rate
            );
        } else if short_notional > long_notional {
            assert!(
                short_rate <= max_rate + 1,
                "Short rate {} should be <= max_rate {}",
                short_rate,
                max_rate
            );
        }
    }
});
