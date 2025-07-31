use soroban_sdk::{Env, I256};
use soroban_fixed_point_math::SorobanFixedPoint;
use crate::constants::{ONE_HOUR_SECONDS, SCALAR_18, SCALAR_7};

/// Calculate the base hourly interest rate based on utilization.
/// Below Target Utilization:
/// HourlyBorrowRate = MinRate + ((TargetRate - MinRate) / TargetUtil) * Utilization
/// Above Target Utilization:
/// HourlyBorrowRate = TargetRate + ((MaxRate - TargetRate) / (1 - TargetUtil)) * (Utilization - TargetUtil)
pub fn base_hourly_interest_rate(
    e: &Env,
    utilization: i128,           // Current utilization (0 to SCALAR_7)
    min_rate: i128,              // Minimum rate when utilization = 0%
    target_rate: i128,           // Rate at target utilization
    max_rate: i128,              // Maximum rate when utilization = 100%
    target_utilization: i128,    // Kink point (e.g., 80% = 8_000_000)
) -> i128 {
    if utilization <= target_utilization {
        // Below target utilization
        let rate_range = target_rate - min_rate;
        let utilization_ratio = utilization.fixed_mul_floor(e, &target_utilization, &SCALAR_7);
        min_rate + rate_range.fixed_mul_floor(e, &utilization_ratio, &SCALAR_7)
    } else {
        // Above target utilization
        let rate_range = max_rate - target_rate;
        let excess_utilization = utilization - target_utilization;
        let remaining_capacity = SCALAR_7 - target_utilization;
        let utilization_ratio = excess_utilization.fixed_div_floor(e, &remaining_capacity, &SCALAR_7);
        target_rate + rate_range.fixed_mul_floor(e, &utilization_ratio, &SCALAR_7)
    }
}


/// Calculate a^n where n is an integer using binary exponentiation
/// Uses 18 decimal precision internally for accuracy
pub fn pow_int(e: &Env, a: &I256, n: &u32) -> I256 {
    let scalar_18_i256 = I256::from_i128(e, SCALAR_18);
    let mut z = if n % 2 != 0 { a.clone() } else { scalar_18_i256.clone() };
    let mut a = a.clone();
    let mut n = n / 2;

    while n != 0 {
        a = a.fixed_mul_floor(e, &a, &scalar_18_i256);
        if n % 2 != 0 {
            z = z.fixed_mul_floor(e, &a, &scalar_18_i256);
        }
        n = n / 2;
    }
    z
}

/// Calculate leverage multiplier as 1.01^average_leverage
/// Inputs are in SCALAR_7 format, but internally uses 18 decimals for precision
fn leverage_multiplier(
    e: &Env,
    collateral: i128, // Total collateral in SCALAR_7
    notional_size: i128, // Total notional size in SCALAR_7
) -> i128 {

    // If no positions or no collateral, return 1.0
    if notional_size == 0 || collateral == 0 {
        return SCALAR_7;
    }

    // Calculate average leverage: notional_size / collateral
    let average_leverage = notional_size.fixed_div_floor(e, &collateral, &SCALAR_7);

    // Convert leverage to integer (e.g., 2.5x leverage becomes 2)
    let leverage_int = (average_leverage / SCALAR_7) as u32;

    // If leverage is less than 1, return 1.0
    if leverage_int == 0 {
        return SCALAR_7;
    }

    // Convert 1.01 from SCALAR_7 to 18 decimals for calculation
    let scale_up = SCALAR_18 / SCALAR_7;
    let base_18 = I256::from_i128(e, 1_010_000_000_000_000_000); // 1.01 with 18 decimals

    // Calculate 1.01^leverage_int with 18 decimal precision
    let result_18 = pow_int(e, &base_18, &leverage_int);

    // Convert result back to SCALAR_7
    let result_i128 = result_18.to_i128().unwrap_or(SCALAR_18);
    result_i128 / scale_up
}

/// Calculate long/short ratios for fee corrections
/// If notional longs > notional shorts: long ratio = 1.0, short ratio = -0.8
/// If notional longs < notional shorts: long ratio = -0.8, short ratio = 1.0
/// If notional longs = notional shorts: both ratios = 1.0
///
/// This correction is implemented to incentivize users to open positions in such a way
/// that the longs and shorts cancel each other out and the vault carries less risk.
pub fn calculate_long_short_ratios(
    e: &Env,
    total_notional_longs: i128,
    total_notional_shorts: i128,
) -> (i128, i128) {
    // If no positions, both sides pay normal rate
    if total_notional_longs == 0 && total_notional_shorts == 0 {
        return (SCALAR_7, SCALAR_7); // Both ratios are 1.0
    }

    // Calculate which side is dominant
    if total_notional_longs > total_notional_shorts {
        // More longs than shorts: longs pay full rate, shorts get discount (negative rate * 0.8)
        let negative_discount = 8000000; // 0.8 in SCALAR_7 (8_000_000)
        (SCALAR_7, -negative_discount) // long_ratio = 1.0, short_ratio = -0.8
    } else if total_notional_longs < total_notional_shorts {
        // More shorts than longs: shorts pay full rate, longs get discount (negative rate * 0.8)
        let negative_discount = 8000000; // 0.8 in SCALAR_7 (8_000_000)
        (-negative_discount, SCALAR_7) // long_ratio = -0.8, short_ratio = 1.0
    } else {
        // Equal longs and shorts: both pay normal rate
        (SCALAR_7, SCALAR_7) // Both ratios are 1.0
    }
}

/// Calculate the adjusted hourly interest rates for long and short positions
/// This combines the base rate, leverage multiplier, and long/short corrections
/// Returns (long_hourly_rate, short_hourly_rate) in SCALAR_7 format
pub fn calculate_long_short_hourly_rates(
    e: &Env,
    utilization: i128,           // Current utilization (0 to SCALAR_7)
    min_rate: i128,              // Minimum rate when utilization = 0%
    target_rate: i128,           // Rate at target utilization
    max_rate: i128,              // Maximum rate when utilization = 100%
    target_utilization: i128,    // Kink point (e.g., 80% = 8_000_000)
    long_collateral: i128,       // Total long collateral
    long_notional: i128,         // Total long notional size
    short_collateral: i128,      // Total short collateral
    short_notional: i128,        // Total short notional size
) -> (i128, i128) {
    // Step 1: Calculate base hourly rate using Jump Rate Model
    let base_hourly_rate = base_hourly_interest_rate(
        e,
        utilization,
        min_rate,
        target_rate,
        max_rate,
        target_utilization,
    );

    // Step 2: Calculate leverage multiplier based on total market leverage
    let total_collateral = long_collateral + short_collateral;
    let total_notional = long_notional + short_notional;
    let leverage_mult = leverage_multiplier(e, total_collateral, total_notional);

    // Step 3: Apply leverage multiplier to base rate
    let leveraged_hourly_rate = base_hourly_rate.fixed_mul_floor(e, &leverage_mult, &SCALAR_7);

    // Step 4: Calculate long/short ratios for balancing
    let (long_ratio, short_ratio) = calculate_long_short_ratios(e, long_notional, short_notional);

    // Step 5: Apply ratios to get final rates
    let long_hourly_rate = leveraged_hourly_rate.fixed_mul_floor(e, &long_ratio, &SCALAR_7);
    let short_hourly_rate = leveraged_hourly_rate.fixed_mul_floor(e, &short_ratio, &SCALAR_7);

    (long_hourly_rate, short_hourly_rate)
}

/// Update a single borrowing index with compound interest
/// Uses per-second compound growth for precision
/// Takes hourly rate and converts it internally to per-second rate
pub fn update_index_with_interest(
    e: &Env,
    current_index: i128,         // Current index value (18 decimal precision)
    hourly_rate: i128,           // Hourly interest rate (7 decimal precision)
    seconds_elapsed: i128,       // Time elapsed in seconds
) -> i128 {
    if seconds_elapsed <= 0 {
        return current_index;
    }

    // Convert hourly rate to per-second rate
    let rate_per_second = hourly_rate.fixed_div_floor(e, &(ONE_HOUR_SECONDS as i128), &SCALAR_7);

    // Calculate total growth over the period
    let period_rate = rate_per_second.fixed_mul_floor(e, &seconds_elapsed, &SCALAR_7);

    // Convert to 18 decimal precision for index math
    let period_rate_18 = period_rate * (SCALAR_18 / SCALAR_7);

    // Growth factor = 1 + period_rate
    let growth_factor = SCALAR_18 + period_rate_18;

    // Apply compound growth to index
    current_index.fixed_mul_floor(e, &growth_factor, &SCALAR_18)
}