use crate::constants::{SCALAR_7, SCALAR_18};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::Env;

/// Calculate the signed funding rate based on open interest imbalance.
///
/// `rate = r_funding × |L - S| / (L + S)`
///
/// Bounded in [-r_funding, +r_funding]. Empty or balanced markets return 0.
///
/// # Parameters
/// - `long_notional` - Total long notional (token_decimals)
/// - `short_notional` - Total short notional (token_decimals)
/// - `base_rate` - `r_funding` from TradingConfig (SCALAR_18)
///
/// # Returns
/// Signed rate (SCALAR_18). Positive = longs pay shorts. Negative = shorts pay longs.
pub fn calc_funding_rate(
    e: &Env,
    long_notional: i128,
    short_notional: i128,
    base_rate: i128,
) -> i128 {
    match (long_notional > 0, short_notional > 0) {
        // No positions on either side
        (false, false) => 0,
        // Only longs exist |L-0|/(L+0) = 1 → full base_rate
        (true, false) => base_rate,
        // Only shorts exist |0-S|/(0+S) = 1 → -full base_rate
        (false, true) => -base_rate,
        // Both sides equal
        (true, true) if long_notional == short_notional => 0,
        // Imbalanced market
        (true, true) => {
            let total = long_notional + short_notional;
            let (imbalance, is_long_dominant) = if long_notional > short_notional {
                (long_notional - short_notional, true)
            } else {
                (short_notional - long_notional, false)
            };

            // Ceil rounding on the rate: payers pay a slightly higher rate.
            // Combined with settlement rounding (ceil for payers, floor for receivers),
            // the vault retains any rounding difference.
            let fraction = imbalance.fixed_div_ceil(e, &total, &SCALAR_18);
            let rate = base_rate.fixed_mul_ceil(e, &fraction, &SCALAR_18);

            if is_long_dominant { rate } else { -rate }
        }
    }
}

/// Calculate the borrowing rate using an additive two-utilization curve.
///
/// `rate = r_base + r_var × util_vault^5 + r_var_market × util_market^3`
///
/// Vault uses ^5 (gentle at low util, aggressive near capacity).
/// Market uses ^3 (reacts faster to per-market congestion).
///
/// # Parameters
/// - `r_base` - Global base borrowing rate (SCALAR_18)
/// - `r_var` - Vault-level variable rate (SCALAR_18)
/// - `r_var_market` - Per-market variable rate (SCALAR_18)
/// - `util_vault` - Vault utilization, clamped [0, SCALAR_7] (SCALAR_7)
/// - `util_market` - Market utilization, clamped [0, SCALAR_7] (SCALAR_7)
///
/// # Returns
/// Borrowing rate (SCALAR_18).
pub fn calc_borrowing_rate(
    e: &Env,
    r_base: i128,
    r_var: i128,
    r_var_market: i128,
    util_vault: i128,
    util_market: i128,
) -> i128 {
    let mut rate = r_base;

    // Vault term: r_var × util_vault^5
    if r_var > 0 && util_vault > 0 {
        let u2 = util_vault.fixed_mul_ceil(e, &util_vault, &SCALAR_7);
        let u4 = u2.fixed_mul_ceil(e, &u2, &SCALAR_7);
        let u5 = u4.fixed_mul_ceil(e, &util_vault, &SCALAR_7);
        rate += r_var.fixed_mul_ceil(e, &u5, &SCALAR_7);
    }

    // Market term: r_var_market × util_market^3
    if r_var_market > 0 && util_market > 0 {
        let u2 = util_market.fixed_mul_ceil(e, &util_market, &SCALAR_7);
        let u3 = u2.fixed_mul_ceil(e, &util_market, &SCALAR_7);
        rate += r_var_market.fixed_mul_ceil(e, &u3, &SCALAR_7);
    }

    rate
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::SCALAR_7;
    use soroban_sdk::Env;

    const BASE_RATE: i128 = 10_000_000_000_000; // 0.001%/hr in SCALAR_18
    const HALF: i128 = SCALAR_7 / 2;            // 50% util
    const FULL: i128 = SCALAR_7;                 // 100% util

    // ── Funding rate tests ──

    #[test]
    fn test_no_positions() {
        let e = Env::default();
        assert_eq!(calc_funding_rate(&e, 0, 0, BASE_RATE), 0);
    }

    #[test]
    fn test_only_longs() {
        let e = Env::default();
        assert_eq!(calc_funding_rate(&e, 1000 * SCALAR_18, 0, BASE_RATE), BASE_RATE);
    }

    #[test]
    fn test_equal_positions() {
        let e = Env::default();
        assert_eq!(calc_funding_rate(&e, 1000 * SCALAR_18, 1000 * SCALAR_18, BASE_RATE), 0);
    }

    #[test]
    fn test_long_dominant_2x() {
        let e = Env::default();
        let rate = calc_funding_rate(&e, 2000 * SCALAR_18, 1000 * SCALAR_18, BASE_RATE);
        // imbalance=1000, total=3000 → fraction=1/3
        // ceil(10_000_000_000_000 / 3) = 3_333_333_333_334
        assert_eq!(rate, 3_333_333_333_334);
    }

    #[test]
    fn test_extreme_ratio() {
        let e = Env::default();
        let rate = calc_funding_rate(&e, 1000000 * SCALAR_18, SCALAR_18, BASE_RATE);
        assert!(rate <= BASE_RATE);
        assert!(rate > BASE_RATE * 999 / 1000);
    }

    //Borrowing rate tests

    #[test]
    fn test_borrowing_zero_utilization() {
        let e = Env::default();
        assert_eq!(calc_borrowing_rate(&e, BASE_RATE, BASE_RATE, BASE_RATE, 0, 0), BASE_RATE);
    }

    #[test]
    fn test_borrowing_full_vault_util_only() {
        let e = Env::default();
        // util_vault=100%, r_var_market=0 → rate = r_base + r_var
        assert_eq!(calc_borrowing_rate(&e, BASE_RATE, BASE_RATE, 0, FULL, 0), 2 * BASE_RATE);
    }

    #[test]
    fn test_borrowing_half_vault_util() {
        let e = Env::default();
        // 0.5^5 = 0.03125 → u5 = 312_500 (in SCALAR_7)
        // vault_term = BASE_RATE × 312_500 / SCALAR_7 = 312_500_000_000
        // total = 10_000_000_000_000 + 312_500_000_000 = 10_312_500_000_000
        let rate = calc_borrowing_rate(&e, BASE_RATE, BASE_RATE, 0, HALF, 0);
        assert_eq!(rate, 10_312_500_000_000);
    }

    #[test]
    fn test_borrowing_no_variable_rates() {
        let e = Env::default();
        assert_eq!(calc_borrowing_rate(&e, BASE_RATE, 0, 0, FULL, FULL), BASE_RATE);
    }

    #[test]
    fn test_borrowing_high_vault_low_market() {
        let e = Env::default();
        let nine = 9 * SCALAR_7 / 10; // 90%
        let one = SCALAR_7 / 10;      // 10%
        let rate = calc_borrowing_rate(&e, BASE_RATE, BASE_RATE, BASE_RATE, nine, one);
        // 0.9^5 = 0.59049 → vault_term = 5_904_900_000_000
        // 0.1^3 = 0.001   → market_term = 10_000_000_000
        // total = 10_000_000_000_000 + 5_904_900_000_000 + 10_000_000_000 = 15_914_900_000_000
        assert_eq!(rate, 15_914_900_000_000);
    }

    #[test]
    fn test_borrowing_additivity() {
        let e = Env::default();
        let uv = HALF;
        let um = SCALAR_7 / 3;
        let both = calc_borrowing_rate(&e, BASE_RATE, BASE_RATE, BASE_RATE, uv, um);
        let vault_only = calc_borrowing_rate(&e, BASE_RATE, BASE_RATE, 0, uv, 0);
        let market_only = calc_borrowing_rate(&e, BASE_RATE, 0, BASE_RATE, 0, um);
        assert_eq!(both, vault_only + market_only - BASE_RATE);
    }

    #[test]
    fn test_borrowing_cubic_vs_quintic() {
        let e = Env::default();
        // Same 50% util, same rate → ^3 > ^5
        let vault_only = calc_borrowing_rate(&e, BASE_RATE, BASE_RATE, 0, HALF, 0);
        let market_only = calc_borrowing_rate(&e, BASE_RATE, 0, BASE_RATE, 0, HALF);
        assert!(market_only > vault_only);
    }

}
