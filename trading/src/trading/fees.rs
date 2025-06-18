// contracts/trading/src/trading/fees.rs
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::Env;
use crate::constants::{SCALAR_7, SCALAR_18};

// ===== MAIN INDEX UPDATE FUNCTION =====

/// Updates both long and short borrowing indices for a market
/// This is the main entry point that should be called before any position operation
///
/// # Parameters
/// - `e`: Environment
/// - `time_delta_seconds`: Time elapsed since last update in seconds
/// - `utilization`: Current market utilization (0 to SCALAR_7)
/// - `long_collateral`: Total collateral in long positions
/// - `long_borrowed`: Total borrowed funds in long positions
/// - `short_collateral`: Total collateral in short positions
/// - `short_borrowed`: Total borrowed funds in short positions
/// - `long_index`: Current cumulative interest index for longs
/// - `short_index`: Current cumulative interest index for shorts
/// - `min_rate`: Minimum hourly borrowing rate
/// - `max_rate`: Maximum hourly borrowing rate
/// - `target_rate`: Target hourly borrowing rate at target utilization
/// - `target_utilization`: Target utilization threshold (kink point)
///
/// # Returns
/// Tuple of (new_long_index, new_short_index)
pub fn update_borrowing_indices(
    e: &Env,
    time_delta_seconds: u64,
    utilization: i128,
    long_collateral: i128,
    long_borrowed: i128,
    short_collateral: i128,
    short_borrowed: i128,
    long_index: i128,
    short_index: i128,
    min_rate: i128,
    max_rate: i128,
    target_rate: i128,
    target_utilization: i128,
) -> (i128, i128) {
    // No update needed if no time has passed
    if time_delta_seconds == 0 {
        return (long_index, short_index);
    }

    // Step 1: Calculate base hourly rate from utilization using Jump Rate Model
    let base_hourly_rate = calculate_hourly_borrowing_rate(
        e,
        utilization,
        min_rate,
        target_rate,
        max_rate,
        target_utilization,
    );

    // Step 2: Convert hourly rate to per-second rate for precise calculation
    let base_second_rate = base_hourly_rate.fixed_div_floor(e, &3600, &SCALAR_7);

    // Step 3: Calculate leverage multiplier based on average market leverage
    let leverage_multiplier = calculate_leverage_multiplier(
        e,
        long_collateral,
        long_borrowed,
        short_collateral,
        short_borrowed,
    );

    // Step 4: Calculate long and short notional values
    let long_notional = long_collateral + long_borrowed;
    let short_notional = short_collateral + short_borrowed;

    // Step 5: Get adjusted rates for longs and shorts based on market imbalance
    let (long_rate, short_rate) = calculate_long_short_adjusted_rates(
        e,
        base_second_rate,
        long_notional,
        short_notional,
    );

    // Step 6: Apply leverage multiplier to both rates
    let long_rate_final = long_rate.fixed_mul_floor(e, &leverage_multiplier, &SCALAR_7);
    let short_rate_final = short_rate.fixed_mul_floor(e, &leverage_multiplier, &SCALAR_7);

    // Step 7: Update indices with per-second compound growth
    let new_long_index = update_index_with_rate(
        e,
        long_index,
        long_rate_final,
        time_delta_seconds as i128,
    );

    let new_short_index = update_index_with_rate(
        e,
        short_index,
        short_rate_final,
        time_delta_seconds as i128,
    );

    (new_long_index, new_short_index)
}

// ===== STEP 1: JUMP RATE MODEL CALCULATION =====

/// Calculate base hourly borrowing rate using Jump Rate Model
/// Below target: rate = min + (target - min) × (utilization / target_util)
/// Above target: rate = target + (max - target) × ((utilization - target_util) / (1 - target_util))
fn calculate_hourly_borrowing_rate(
    e: &Env,
    utilization: i128,           // Current utilization (0 to SCALAR_7)
    min_rate: i128,             // Minimum rate when utilization = 0%
    target_rate: i128,          // Rate at target utilization
    max_rate: i128,             // Maximum rate when utilization = 100%
    target_utilization: i128,    // Kink point (e.g., 80% = 8_000_000)
) -> i128 {
    if utilization <= target_utilization {
        // Below kink: gradual increase from min to target
        let rate_range = target_rate - min_rate;
        let utilization_ratio = utilization.fixed_div_floor(e, &target_utilization, &SCALAR_7);
        let additional_rate = rate_range.fixed_mul_floor(e, &utilization_ratio, &SCALAR_7);
        min_rate + additional_rate
    } else {
        // Above kink: sharp increase from target to max
        let rate_range = max_rate - target_rate;
        let excess_utilization = utilization - target_utilization;
        let remaining_capacity = SCALAR_7 - target_utilization;

        if remaining_capacity == 0 {
            return max_rate;
        }

        let utilization_ratio = excess_utilization.fixed_div_floor(e, &remaining_capacity, &SCALAR_7);
        let additional_rate = rate_range.fixed_mul_floor(e, &utilization_ratio, &SCALAR_7);
        target_rate + additional_rate
    }
}

// ===== STEP 3: LEVERAGE MULTIPLIER CALCULATION =====

/// Calculate leverage multiplier for borrowing fees
/// Formula: Leverage Multiplier = 1.01 ^ Average Leverage
/// Average Leverage = (Total Notional Long + Total Notional Short) / Total Collateral
/// Higher leverage = higher risk = higher fees
fn calculate_leverage_multiplier(
    e: &Env,
    long_collateral: i128,
    long_borrowed: i128,
    short_collateral: i128,
    short_borrowed: i128,
) -> i128 {
    let total_collateral = long_collateral + short_collateral;

    // If no positions, return 1x leverage
    if total_collateral == 0 {
        return SCALAR_7; // 1.0x
    }

    // Calculate total notional value
    // Notional = Collateral + Borrowed
    let long_notional = long_collateral + long_borrowed;
    let short_notional = short_collateral + short_borrowed;
    let total_notional = long_notional + short_notional;

    // Average leverage = Total Notional / Total Collateral
    let average_leverage = total_notional.fixed_div_floor(e, &total_collateral, &SCALAR_7);

    // Convert from SCALAR_7 to integer
    let leverage_int = average_leverage / SCALAR_7;

    if leverage_int <= 0 {
        return SCALAR_7; // 1.0x multiplier
    }

    // 1.01 in SCALAR_7 format
    let base = SCALAR_7 + (SCALAR_7 / 100); // 1_0100000

    // Power calculation for reasonable leverage values (1-100x)
    let mut result = SCALAR_7;
    for _ in 0..leverage_int {
        result = result.fixed_mul_floor(e, &base, &SCALAR_7);
    }

    // For fractional part, use linear approximation
    let fractional_leverage = average_leverage % SCALAR_7;
    if fractional_leverage > 0 {
        let fractional_growth = (base - SCALAR_7).fixed_mul_floor(e, &fractional_leverage, &SCALAR_7);
        result = result + result.fixed_mul_floor(e, &fractional_growth, &SCALAR_7);
    }

    result
}

// ===== STEP 5: LONG/SHORT BALANCE ADJUSTMENTS =====

/// Calculate separate rates for longs and shorts to incentivize balance
/// Implements the direct ratio as specified in the docs:
/// Long multiplier = Long Notional / Short Notional
/// Short multiplier = Short Notional / Long Notional
fn calculate_long_short_adjusted_rates(
    e: &Env,
    base_rate: i128,             // Base rate (per second)
    total_long_notional: i128,   // Total value of all long positions
    total_short_notional: i128,  // Total value of all short positions
) -> (i128, i128) {  // Returns (long_rate, short_rate)

    // If either side has no positions, use base rate for both
    if total_long_notional == 0 || total_short_notional == 0 {
        return (base_rate, base_rate);
    }

    // Calculate direct ratios as specified in docs
    let long_multiplier = total_long_notional.fixed_div_floor(e, &total_short_notional, &SCALAR_7);
    let short_multiplier = total_short_notional.fixed_div_floor(e, &total_long_notional, &SCALAR_7);

    // Apply multipliers to base rate
    let long_rate = base_rate.fixed_mul_floor(e, &long_multiplier, &SCALAR_7);
    let short_rate = base_rate.fixed_mul_floor(e, &short_multiplier, &SCALAR_7);

    (long_rate, short_rate)
}

// ===== STEP 7: INDEX UPDATE CALCULATION =====

/// Update a single borrowing index with compound interest
/// Uses per-second compound growth for precision
fn update_index_with_rate(
    e: &Env,
    current_index: i128,
    rate_per_second: i128,
    seconds_elapsed: i128,
) -> i128 {
    if seconds_elapsed <= 0 {
        return current_index;
    }

    // Calculate total growth over the period
    let period_rate = rate_per_second.fixed_mul_floor(e, &seconds_elapsed, &SCALAR_7);

    // Convert to 18 decimal precision for index math
    let period_rate_18 = period_rate * (SCALAR_18 / SCALAR_7);

    // Growth factor = 1 + period_rate
    let growth_factor = SCALAR_18 + period_rate_18;

    // Apply compound growth to index
    current_index.fixed_mul_floor(e, &growth_factor, &SCALAR_18)
}

// ===== FEE CALCULATION FUNCTIONS (Used when closing positions) =====

/// Calculate the base fee for a position operation
/// Formula: Base Fee = Collateral × Base Fee Rate
pub fn calculate_base_fee(e: &Env, collateral: i128, base_fee_rate: i128) -> i128 {
    collateral.fixed_mul_floor(e, &base_fee_rate, &SCALAR_7)
}

/// Calculate price impact fee based on notional size
/// Formula: Price Impact Fee = Notional Size / Price Impact Scalar
pub fn calculate_price_impact_fee(e: &Env, notional_size: i128, price_impact_scalar: i128) -> i128 {
    if price_impact_scalar == 0 {
        return 0;
    }
    notional_size.fixed_div_floor(e, &price_impact_scalar, &SCALAR_7)
}

/// Calculate borrowing fee using index growth (compound interest)
/// Formula: Fee = Borrowed Amount × (Current Index / Position Index - 1)
pub fn calculate_borrowing_fee_from_index(
    e: &Env,
    borrowed_amount: i128,
    position_index: i128,        // Index when position was opened
    current_index: i128,         // Current long_index or short_index
) -> i128 {
    if borrowed_amount <= 0 || current_index <= position_index {
        return 0;
    }

    // Calculate growth factor (how much the index has grown)
    let index_ratio = current_index.fixed_div_floor(e, &position_index, &SCALAR_18);
    let growth_factor = index_ratio - SCALAR_18; // Subtract 1.0

    // Apply growth to borrowed amount
    borrowed_amount.fixed_mul_floor(e, &growth_factor, &SCALAR_18)
}