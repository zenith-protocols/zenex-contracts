use crate::constants::{ONE_HOUR_SECONDS, SCALAR_18};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{log, Env};

/// Calculate the adjusted hourly interest rates for long and short positions
/// This combines the base rate, leverage multiplier, and long/short corrections
/// Returns (long_hourly_rate, short_hourly_rate) in SCALAR_18 format
pub fn calculate_long_short_hourly_rates(
    e: &Env,
    base_hourly_rate: i128, // Base hourly rate (in SCALAR_18)
    long_notional: i128,    // Total long notional size
    short_notional: i128,   // Total short notional size
) -> (i128, i128) {
    // If no positions, return zero rates
    if long_notional == 0 && short_notional == 0 {
        return (0, 0);
    }

    let short_ratio = short_notional.fixed_div_floor(e, &long_notional, &SCALAR_18);
    let long_ratio = long_notional.fixed_div_floor(e, &short_notional, &SCALAR_18);
    let squared_long_ratio = long_ratio.fixed_mul_floor(e, &long_ratio, &SCALAR_18);
    let squared_short_ratio = short_ratio.fixed_mul_floor(e, &short_ratio, &SCALAR_18);

    // Calculate the 0.8 multiplier in SCALAR_18 format
    let discount_multiplier = 800000000000000000; // 0.8 * 10^18

    // Edge cases: one side empty → dominant pays base (optionally discounted), empty side "receives" base.
    if short_notional == 0 && long_notional > 0 {
        // Longs dominate; make longs pay the base (discounted) and shorts receive base.
        let short_rate = -base_hourly_rate.fixed_mul_floor(e, &discount_multiplier, &SCALAR_18);
        let long_rate = base_hourly_rate;
        return (long_rate, short_rate);
    }

    if long_notional == 0 && short_notional > 0 {
        // Shorts dominate; make shorts pay the base (discounted) and longs receive base.
        let long_rate = -base_hourly_rate.fixed_mul_floor(e, &discount_multiplier, &SCALAR_18);
        let short_rate = base_hourly_rate;
        return (long_rate, short_rate);
    }

    // Calculate interest rates based on long/short dominance
    let (long_rate, short_rate) = if long_notional >= short_notional {
        // When longs ≥ shorts:
        // hourlyRateLong = hourlyRate * (notionalLongs / notionalShorts)
        // hourlyRateShort = -0.8 * hourlyRate * (notionalLongs / notionalShorts)^2
        let long_rate = base_hourly_rate.fixed_mul_floor(e, &long_ratio, &SCALAR_18);
        let short_rate = -base_hourly_rate
            .fixed_mul_floor(e, &discount_multiplier, &SCALAR_18)
            .fixed_mul_floor(e, &squared_long_ratio, &SCALAR_18);
        (long_rate, short_rate)
    } else {
        // When longs < shorts:
        // hourlyRateLong = -0.8 * hourlyRate * (notionalShorts / notionalLongs)^2
        // hourlyRateShort = hourlyRate * (notionalShorts / notionalLongs)
        let long_rate = -base_hourly_rate
            .fixed_mul_floor(e, &discount_multiplier, &SCALAR_18)
            .fixed_mul_floor(e, &squared_short_ratio, &SCALAR_18);
        let short_rate = base_hourly_rate.fixed_mul_floor(e, &short_ratio, &SCALAR_18);
        (long_rate, short_rate)
    };

    (long_rate, short_rate)
}

/// Update a single borrowing index with compound interest
/// Uses per-second compound growth for precision
/// Takes hourly rate and converts it internally to per-second rate
pub fn update_index_with_interest(
    e: &Env,
    current_index: i128,   // Current index value (18 decimal precision)
    hourly_rate: i128,     // Hourly interest rate (18 decimal precision)
    seconds_elapsed: i128, // Time elapsed in seconds
) -> i128 {
    if seconds_elapsed <= 0 {
        return current_index;
    }

    // Convert hourly rate to per-second rate
    let rate_per_second = hourly_rate.fixed_div_floor(e, &(ONE_HOUR_SECONDS as i128), &SCALAR_18);

    // Calculate total growth over the period
    let period_rate = rate_per_second.fixed_mul_floor(e, &seconds_elapsed, &SCALAR_18);

    // Growth factor = 1 + period_rate (both already in 18 decimal precision)
    let growth_factor = SCALAR_18 + period_rate;

    // Apply compound growth to index

    current_index.fixed_mul_floor(e, &growth_factor, &SCALAR_18)
}
