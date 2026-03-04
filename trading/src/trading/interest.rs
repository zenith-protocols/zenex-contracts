use crate::constants::SCALAR_18;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::Env;

/// Calculate the signed funding rate based on market imbalance.
///
/// Returns a single i128 rate in SCALAR_18 precision.
/// Positive = longs pay, negative = shorts pay.
///
/// Formula: baseRate × |L - S| / (L + S)
/// - Naturally bounded in [0, baseRate]
/// - One-sided market: ±baseRate
/// - Balanced or no positions: 0
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

            let fraction = imbalance.fixed_div_ceil(e, &total, &SCALAR_18);
            let rate = base_rate.fixed_mul_ceil(e, &fraction, &SCALAR_18);

            if is_long_dominant { rate } else { -rate }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
