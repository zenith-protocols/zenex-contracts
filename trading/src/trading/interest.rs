use soroban_sdk::{Env, I256};
use soroban_fixed_point_math::SorobanFixedPoint;
use crate::constants::{SCALAR_18, SCALAR_7};

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
    long_collateral: i128,
    long_borrowed: i128,
    short_collateral: i128,
    short_borrowed: i128,
) -> i128 {
    // Calculate total notional values
    let total_notional_longs = long_collateral + long_borrowed;
    let total_notional_shorts = short_collateral + short_borrowed;
    let total_notional = total_notional_longs + total_notional_shorts;

    // Calculate total collateral
    let total_collateral = long_collateral + short_collateral;

    // If no positions or no collateral, return 1.0
    if total_notional == 0 || total_collateral == 0 {
        return SCALAR_7;
    }

    // Calculate average leverage: total_notional / total_collateral
    let average_leverage = total_notional.fixed_div_floor(e, &total_collateral, &SCALAR_7);

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

pub fn calculate_ratios(
    e: &Env,
    long_collateral: i128,
    long_borrowed: i128,
    short_collateral: i128,
    short_borrowed: i128,
) -> (i128, i128) {
    let total_notional_longs = long_collateral + long_borrowed;
    let total_notional_shorts = short_collateral + short_borrowed;

    if total_notional_longs == 0 && total_notional_shorts == 0 {
        return (SCALAR_7, SCALAR_7); // Both ratios are 1.0 in SCALAR_7
    }

    // Handle division by zero cases
    let long_ratio = if total_notional_shorts == 0 {
        SCALAR_7 // Return 1.0 if no shorts
    } else {
        total_notional_longs.fixed_div_floor(e, &total_notional_shorts, &SCALAR_7)
    };

    let short_ratio = if total_notional_longs == 0 {
        SCALAR_7 // Return 1.0 if no longs
    } else {
        total_notional_shorts.fixed_div_floor(e, &total_notional_longs, &SCALAR_7)
    };

    (long_ratio, short_ratio)
}