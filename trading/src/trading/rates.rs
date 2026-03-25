use crate::constants::{SCALAR_7, SCALAR_18};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::Env;

/// Calculate the signed funding rate based on open interest imbalance.
///
/// # Formula
/// ```text
/// rate = r_funding × |L - S| / (L + S)
/// ```
///
/// Returns a single i128 rate in SCALAR_18 precision.
/// Positive = longs pay shorts, negative = shorts pay longs.
///
/// # Bounds
/// - Naturally bounded in `[-r_funding, +r_funding]` by the `|L-S|/(L+S)` fraction.
/// - One-sided market: `|L-0|/(L+0) = 1` yields full `r_funding`.
/// - Balanced or empty: rate is 0.
///
/// # Parameters
/// - `long_notional` - Total long notional across all positions (token_decimals)
/// - `short_notional` - Total short notional across all positions (token_decimals)
/// - `base_rate` - `r_funding` from TradingConfig (SCALAR_18, hourly)
///
/// # Design (WHY)
/// WHY: Funding is pure peer-to-peer. The dominant (heavier) side pays the
/// non-dominant side directly. The protocol collects zero funding revenue --
/// it only facilitates the transfer. This ensures the cost of holding a
/// position scales with how imbalanced the market is, incentivizing
/// arbitrageurs to restore balance.
pub fn calc_funding_rate(
    e: &Env,
    long_notional: i128,
    short_notional: i128,
    base_rate: i128,
) -> i128 {
    match (long_notional > 0, short_notional > 0) {
        // No positions on either side
        (false, false) => 0,
        // Only longs exist — |L-0|/(L+0) = 1 → full base_rate
        (true, false) => base_rate,
        // Only shorts exist — |0-S|/(0+S) = 1 → -full base_rate
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

            // WHY: ceil rounding on funding rate -- rounds in favor of the receiving
            // (non-dominant) side, ensuring they never receive less than owed.
            let fraction = imbalance.fixed_div_ceil(e, &total, &SCALAR_18);
            let rate = base_rate.fixed_mul_ceil(e, &fraction, &SCALAR_18);

            if is_long_dominant { rate } else { -rate }
        }
    }
}

/// Calculate the per-market borrowing rate using a steep utilization curve.
///
/// # Formula
/// ```text
/// rate = r_base × (1 + r_var × util^5) × r_borrow
/// ```
///
/// # Parameters
/// - `r_base` - Global base borrowing rate (SCALAR_18, hourly)
/// - `r_var` - Variable multiplier at full utilization (SCALAR_7; e.g. 1e7 = rate doubles at 100%)
/// - `r_borrow` - Per-market weight (SCALAR_7; 1e7 = 1x, 2e7 = 2x for volatile markets)
/// - `util` - Current market utilization, `total_notional / vault_balance` (SCALAR_7)
///
/// # Design (WHY)
/// WHY: util^5 exponent creates a steep, convex curve that keeps borrowing
/// cheap at low utilization but ramps aggressively near capacity. This
/// disincentivizes concentrated positions that threaten vault solvency,
/// while keeping costs negligible for normal usage. At 50% util the
/// multiplier is only ~3%, but at 90% it's ~59%.
///
/// WHY: Only the dominant (heavier) side pays borrowing. The non-dominant
/// side does not incur borrowing costs because their positions actually
/// reduce systemic risk by providing counterparty balance.
pub fn calc_borrowing_rate(
    e: &Env,
    r_base: i128,
    r_var: i128,      // SCALAR_7 multiplier
    r_borrow: i128,   // SCALAR_7 per-market weight
    util: i128,        // SCALAR_7 precision, 0..SCALAR_7
) -> i128 {
    if util <= 0 || r_var <= 0 {
        return r_base.fixed_mul_ceil(e, &r_borrow, &SCALAR_7);
    }

    // util^5 in SCALAR_7 precision (computed via repeated squaring + one multiply)
    let u2 = util.fixed_mul_ceil(e, &util, &SCALAR_7);
    let u4 = u2.fixed_mul_ceil(e, &u2, &SCALAR_7);
    let u5 = u4.fixed_mul_ceil(e, &util, &SCALAR_7);

    // multiplier = 1 + r_var × util^5 (in SCALAR_7)
    let util_factor = r_var.fixed_mul_ceil(e, &u5, &SCALAR_7);
    let multiplier = SCALAR_7 + util_factor;

    // WHY: ceil rounding throughout -- borrowing rate rounds up to protect the vault
    // from under-collecting fees in high-utilization scenarios.
    // rate = base × multiplier × r_borrow (SCALAR_18 result)
    let global_rate = r_base.fixed_mul_ceil(e, &multiplier, &SCALAR_7);
    global_rate.fixed_mul_ceil(e, &r_borrow, &SCALAR_7)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::SCALAR_7;
    use soroban_sdk::Env;

    // Base rate: 0.001% per hour = 10^13 in SCALAR_18
    const BASE_RATE: i128 = 10_000_000_000_000;

    #[test]
    fn test_no_positions() {
        let e = Env::default();
        let rate = calc_funding_rate(&e, 0, 0, BASE_RATE);
        assert_eq!(rate, 0);
    }

    #[test]
    fn test_only_longs() {
        let e = Env::default();
        let rate = calc_funding_rate(&e, 1000 * SCALAR_18, 0, BASE_RATE);
        // |L-0|/(L+0) = 1 → base_rate
        assert_eq!(rate, BASE_RATE);
    }

    #[test]
    fn test_only_shorts() {
        let e = Env::default();
        let rate = calc_funding_rate(&e, 0, 1000 * SCALAR_18, BASE_RATE);
        // |0-S|/(0+S) = 1 → -base_rate
        assert_eq!(rate, -BASE_RATE);
    }

    #[test]
    fn test_equal_positions() {
        let e = Env::default();
        let rate = calc_funding_rate(
            &e,
            1000 * SCALAR_18,
            1000 * SCALAR_18,
            BASE_RATE,
        );
        // Balanced — rate is 0
        assert_eq!(rate, 0);
    }

    #[test]
    fn test_long_dominant_2x() {
        let e = Env::default();
        // 2000 long vs 1000 short: |2000-1000|/(2000+1000) = 1/3
        let rate = calc_funding_rate(
            &e,
            2000 * SCALAR_18,
            1000 * SCALAR_18,
            BASE_RATE,
        );
        // base_rate * 1/3 — use ceil division so expect ceiling
        let expected = BASE_RATE.fixed_mul_ceil(&Env::default(), &SCALAR_18, &(3 * SCALAR_18));
        assert_eq!(rate, expected);
    }

    #[test]
    fn test_short_dominant_2x() {
        let e = Env::default();
        // 1000 long vs 2000 short: |1000-2000|/(1000+2000) = 1/3
        let rate = calc_funding_rate(
            &e,
            1000 * SCALAR_18,
            2000 * SCALAR_18,
            BASE_RATE,
        );
        let expected = BASE_RATE.fixed_mul_ceil(&Env::default(), &SCALAR_18, &(3 * SCALAR_18));
        assert_eq!(rate, -expected);
    }

    #[test]
    fn test_long_dominant_high_ratio() {
        let e = Env::default();
        // 10000 long vs 1000 short: |10000-1000|/(10000+1000) = 9/11
        let rate = calc_funding_rate(
            &e,
            10000 * SCALAR_18,
            1000 * SCALAR_18,
            BASE_RATE,
        );
        // base_rate * 9/11
        let fraction = (9 * SCALAR_18).fixed_div_ceil(&e, &(11 * SCALAR_18), &SCALAR_18);
        let expected = BASE_RATE.fixed_mul_ceil(&e, &fraction, &SCALAR_18);
        assert_eq!(rate, expected);
    }

    #[test]
    fn test_short_dominant_high_ratio() {
        let e = Env::default();
        // 1000 long vs 10000 short: |1000-10000|/(1000+10000) = 9/11
        let rate = calc_funding_rate(
            &e,
            1000 * SCALAR_18,
            10000 * SCALAR_18,
            BASE_RATE,
        );
        let fraction = (9 * SCALAR_18).fixed_div_ceil(&e, &(11 * SCALAR_18), &SCALAR_18);
        let expected = BASE_RATE.fixed_mul_ceil(&e, &fraction, &SCALAR_18);
        assert_eq!(rate, -expected);
    }

    #[test]
    fn test_extreme_ratio() {
        let e = Env::default();
        // 1000000 long vs 1 short: rate ≈ base_rate (naturally bounded)
        let rate = calc_funding_rate(
            &e,
            1000000 * SCALAR_18,
            SCALAR_18,
            BASE_RATE,
        );
        // (1000000-1)/(1000000+1) ≈ 0.999998 → rate ≈ base_rate
        assert!(rate <= BASE_RATE);
        assert!(rate > BASE_RATE * 999 / 1000); // within 0.1% of base_rate
    }

    // ==========================================
    // Borrowing Rate Tests
    // ==========================================

    #[test]
    fn test_borrowing_rate_zero_utilization() {
        let e = Env::default();
        let rate = calc_borrowing_rate(&e, BASE_RATE, SCALAR_7, SCALAR_7, 0);
        assert_eq!(rate, BASE_RATE);
    }

    #[test]
    fn test_borrowing_rate_full_utilization() {
        let e = Env::default();
        // util=100% → util^5=1 → rate = base × (1 + 1) × 1x = 2 × base
        let rate = calc_borrowing_rate(&e, BASE_RATE, SCALAR_7, SCALAR_7, SCALAR_7);
        assert_eq!(rate, 2 * BASE_RATE);
    }

    #[test]
    fn test_borrowing_rate_half_utilization() {
        let e = Env::default();
        let half = SCALAR_7 / 2;
        // util=50% → util^5 ≈ 0.03125 → rate ≈ 1.03× base
        let rate = calc_borrowing_rate(&e, BASE_RATE, SCALAR_7, SCALAR_7, half);
        assert!(rate > BASE_RATE);
        assert!(rate < BASE_RATE + BASE_RATE / 10); // less than 1.1× base
    }

    #[test]
    fn test_borrowing_rate_no_variable() {
        let e = Env::default();
        let rate = calc_borrowing_rate(&e, BASE_RATE, 0, SCALAR_7, SCALAR_7);
        assert_eq!(rate, BASE_RATE);
    }

    #[test]
    fn test_borrowing_rate_high_multiplier() {
        let e = Env::default();
        // variable = 10× → at full util: rate = base × (1 + 10) × 1x = 11 × base
        let rate = calc_borrowing_rate(&e, BASE_RATE, 10 * SCALAR_7, SCALAR_7, SCALAR_7);
        assert_eq!(rate, 11 * BASE_RATE);
    }

    #[test]
    fn test_borrowing_rate_market_weight() {
        let e = Env::default();
        // r_borrow = 2x weight, zero util → rate = base × 2
        let rate = calc_borrowing_rate(&e, BASE_RATE, SCALAR_7, 2 * SCALAR_7, 0);
        assert_eq!(rate, 2 * BASE_RATE);
    }
}
