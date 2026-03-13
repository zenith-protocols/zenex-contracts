use crate::constants::{ONE_HOUR_SECONDS, SCALAR_18};
use crate::types::MarketData;
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

/// Accrues funding using the stored signed rate.
/// Payers pay full delta per unit. Receivers get: pay_delta × (dominant/minority) per unit.
/// Total received equals total paid (self-balancing). Vault skim is applied at close time.
pub fn accrue_funding(e: &Env, data: &mut MarketData) {
    let current_time = e.ledger().timestamp();
    let seconds_elapsed = current_time.saturating_sub(data.last_update) as i128;

    if seconds_elapsed <= 0 {
        return;
    }

    let hour = ONE_HOUR_SECONDS as i128;
    let hours_scaled = seconds_elapsed.fixed_mul_floor(e, &SCALAR_18, &hour);

    let pay_delta = data
        .funding_rate
        .abs()
        .fixed_mul_ceil(e, &hours_scaled, &SCALAR_18);

    if data.funding_rate > 0 {
        // Longs pay
        data.long_funding_index += pay_delta;
        // Shorts receive (scaled by L/S ratio)
        if data.short_notional_size > 0 {
            let ratio = data.long_notional_size
                .fixed_div_floor(e, &data.short_notional_size, &SCALAR_18);
            let receive_delta = pay_delta
                .fixed_mul_floor(e, &ratio, &SCALAR_18);
            data.short_funding_index -= receive_delta;
        }
    } else if data.funding_rate < 0 {
        // Shorts pay
        data.short_funding_index += pay_delta;
        // Longs receive (scaled by S/L ratio)
        if data.long_notional_size > 0 {
            let ratio = data.short_notional_size
                .fixed_div_floor(e, &data.long_notional_size, &SCALAR_18);
            let receive_delta = pay_delta
                .fixed_mul_floor(e, &ratio, &SCALAR_18);
            data.long_funding_index -= receive_delta;
        }
    }
    data.last_update = current_time;
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

    #[test]
    fn test_accrue_funding_longs_pay() {
        use crate::testutils::{create_trading, default_market_data, jump};

        let e = Env::default();
        jump(&e, 0);

        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let mut data = default_market_data();
            data.long_notional_size = 2000 * SCALAR_18;
            data.short_notional_size = 1000 * SCALAR_18;
            data.funding_rate = 10_000_000_000_000; // positive = longs pay
            data.last_update = 0;

            // Advance time by 1 hour
            jump(&e, 3600);

            accrue_funding(&e, &mut data);

            // Funding should have accrued — longs pay, index increases
            assert!(data.long_funding_index > 0);
            assert_eq!(data.last_update, 3600);
        });
    }

    #[test]
    fn test_update_funding_rate() {
        use crate::testutils::default_market_data;

        let e = Env::default();

        let mut data = default_market_data();
        data.long_notional_size = 2000 * SCALAR_18;
        data.short_notional_size = 1000 * SCALAR_18;

        let base_hourly_rate = 10_000_000_000_000i128;
        data.update_funding_rate(&e, base_hourly_rate);

        // Longs dominant → positive rate
        assert!(data.funding_rate > 0);
    }

}
