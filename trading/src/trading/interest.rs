use crate::constants::SCALAR_18;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::Env;

/// Receiving side discount (0.8) - ensure vault remains profitable
const DISCOUNT_FACTOR: i128 = 800_000_000_000_000_000;

/// Calculate funding rates based on market imbalance.
///
/// Returns (long_rate, short_rate) as hourly rates in SCALAR_18 precision.
///
/// Formula:
/// - Dominant side pays: `base_rate × ratio` (capped at ratio_cap)
/// - Minority side receives: `-0.8 × base_rate × ratio²`
/// - Equal positions: both pay base_rate
/// - One-sided market: existing side pays base_rate, empty side would receive 0.8×base_rate
pub fn calc_interest(
    e: &Env,
    long_notional: i128,
    short_notional: i128,
    base_rate: i128,
    ratio_cap: i128,
) -> (i128, i128) {
    match (long_notional > 0, short_notional > 0) {
        // No positions on either side
        (false, false) => (0, 0),
        // Only longs exist
        (true, false) => (
            base_rate,
            -base_rate.fixed_mul_floor(e, &DISCOUNT_FACTOR, &SCALAR_18),
        ),
        // Only shorts exist
        (false, true) => (
            -base_rate.fixed_mul_floor(e, &DISCOUNT_FACTOR, &SCALAR_18),
            base_rate,
        ),
        // Both sides equal
        (true, true) if long_notional == short_notional => (base_rate, base_rate),
        // Imbalanced market
        (true, true) => {
            let (dominant, minority, is_long_dominant) = if long_notional > short_notional {
                (long_notional, short_notional, true)
            } else {
                (short_notional, long_notional, false)
            };

            // Use ceil for ratio to ensure payers pay more (protocol-safe)
            let ratio = dominant
                .fixed_div_ceil(e, &minority, &SCALAR_18)
                .min(ratio_cap);
            let squared = ratio.fixed_mul_ceil(e, &ratio, &SCALAR_18);

            // Ceil for pay amount (payers pay more)
            let pay = base_rate.fixed_mul_ceil(e, &ratio, &SCALAR_18);
            // Floor for receive amount (receivers get less)
            let receive = -base_rate
                .fixed_mul_floor(e, &DISCOUNT_FACTOR, &SCALAR_18)
                .fixed_mul_floor(e, &squared, &SCALAR_18);

            if is_long_dominant {
                (pay, receive)
            } else {
                (receive, pay)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    // Base rate: 0.001% per hour = 10^13 in SCALAR_18
    const BASE_RATE: i128 = 10_000_000_000_000;
    // 5x ratio cap
    const RATIO_CAP: i128 = 5 * SCALAR_18;

    #[test]
    fn test_no_positions() {
        let e = Env::default();
        let (long_rate, short_rate) = calc_interest(&e, 0, 0, BASE_RATE, RATIO_CAP);

        assert_eq!(long_rate, 0);
        assert_eq!(short_rate, 0);
    }

    #[test]
    fn test_only_longs() {
        let e = Env::default();
        let (long_rate, short_rate) =
            calc_interest(&e, 1000 * SCALAR_18, 0, BASE_RATE, RATIO_CAP);

        // Longs pay base rate
        assert_eq!(long_rate, BASE_RATE);
        // Shorts would receive 0.8x base rate (negative = receiving)
        assert_eq!(short_rate, -BASE_RATE * 8 / 10);
    }

    #[test]
    fn test_only_shorts() {
        let e = Env::default();
        let (long_rate, short_rate) =
            calc_interest(&e, 0, 1000 * SCALAR_18, BASE_RATE, RATIO_CAP);

        // Longs would receive 0.8x base rate
        assert_eq!(long_rate, -BASE_RATE * 8 / 10);
        // Shorts pay base rate
        assert_eq!(short_rate, BASE_RATE);
    }

    #[test]
    fn test_equal_positions() {
        let e = Env::default();
        let (long_rate, short_rate) =
            calc_interest(&e, 1000 * SCALAR_18, 1000 * SCALAR_18, BASE_RATE, RATIO_CAP);

        // Both sides pay base rate when balanced
        assert_eq!(long_rate, BASE_RATE);
        assert_eq!(short_rate, BASE_RATE);
    }

    #[test]
    fn test_long_dominant_2x() {
        let e = Env::default();
        // 2000 long vs 1000 short = 2x ratio
        let (long_rate, short_rate) =
            calc_interest(&e, 2000 * SCALAR_18, 1000 * SCALAR_18, BASE_RATE, RATIO_CAP);

        // Longs pay: base_rate * 2 = 2x base rate
        assert_eq!(long_rate, BASE_RATE * 2);
        // Shorts receive: -0.8 * base_rate * 4 = -3.2x base rate
        assert_eq!(short_rate, -BASE_RATE * 8 * 4 / 10);
    }

    #[test]
    fn test_short_dominant_2x() {
        let e = Env::default();
        // 1000 long vs 2000 short = 2x ratio (short dominant)
        let (long_rate, short_rate) =
            calc_interest(&e, 1000 * SCALAR_18, 2000 * SCALAR_18, BASE_RATE, RATIO_CAP);

        // Longs receive: -0.8 * base_rate * 4 = -3.2x base rate
        assert_eq!(long_rate, -BASE_RATE * 8 * 4 / 10);
        // Shorts pay: base_rate * 2 = 2x base rate
        assert_eq!(short_rate, BASE_RATE * 2);
    }

    #[test]
    fn test_long_dominant_at_cap() {
        let e = Env::default();
        // 10000 long vs 1000 short = 10x ratio, but capped at 5x
        let (long_rate, short_rate) =
            calc_interest(&e, 10000 * SCALAR_18, 1000 * SCALAR_18, BASE_RATE, RATIO_CAP);

        // Longs pay: base_rate * 5 (capped)
        assert_eq!(long_rate, BASE_RATE * 5);
        // Shorts receive: -0.8 * base_rate * 25 = -20x base rate
        assert_eq!(short_rate, -BASE_RATE * 8 * 25 / 10);
    }

    #[test]
    fn test_short_dominant_at_cap() {
        let e = Env::default();
        // 1000 long vs 10000 short = 10x ratio, but capped at 5x
        let (long_rate, short_rate) =
            calc_interest(&e, 1000 * SCALAR_18, 10000 * SCALAR_18, BASE_RATE, RATIO_CAP);

        // Longs receive: -0.8 * base_rate * 25 = -20x base rate
        assert_eq!(long_rate, -BASE_RATE * 8 * 25 / 10);
        // Shorts pay: base_rate * 5 (capped)
        assert_eq!(short_rate, BASE_RATE * 5);
    }

    #[test]
    fn test_ratio_cap_prevents_extreme_rates() {
        let e = Env::default();
        // 1000000 long vs 1 short = 1,000,000x ratio, but capped at 5x
        let (long_rate, short_rate) =
            calc_interest(&e, 1000000 * SCALAR_18, SCALAR_18, BASE_RATE, RATIO_CAP);

        // Should be same as 5x cap
        assert_eq!(long_rate, BASE_RATE * 5);
        assert_eq!(short_rate, -BASE_RATE * 8 * 25 / 10);
    }

    #[test]
    fn test_vault_profit_margin() {
        let e = Env::default();
        // With 2x imbalance:
        // - Dominant pays: base_rate * 2
        // - Minority receives: -0.8 * base_rate * 4 = -3.2 * base_rate
        // Vault keeps: 2 - 3.2 * (minority_size / dominant_size)
        //            = 2 - 3.2 * 0.5 = 2 - 1.6 = 0.4 per unit of dominant notional
        let (long_rate, short_rate) =
            calc_interest(&e, 2000 * SCALAR_18, 1000 * SCALAR_18, BASE_RATE, RATIO_CAP);

        // Verify the 0.8 discount ensures vault profit
        // Total collected from longs: 2000 * 2 * BASE_RATE = 4000 * BASE_RATE
        // Total paid to shorts: 1000 * 3.2 * BASE_RATE = 3200 * BASE_RATE
        // Vault profit: 800 * BASE_RATE (20% of what longs pay)
        let long_payment = 2000 * long_rate;
        let short_receipt = 1000 * short_rate.abs();
        assert!(long_payment > short_receipt);
        assert_eq!(long_payment - short_receipt, 800 * BASE_RATE);
    }
}
