use crate::constants::{ONE_HOUR_SECONDS, SCALAR_18};
use crate::errors::TradingError;
use crate::storage;
use crate::types::{MarketConfig, MarketData};
use sep_40_oracle::{Asset, PriceFeedClient};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{contracttype, panic_with_error, Address, Env};

/// Receiving side discount (0.8) - ensure vault remains profitable
const DISCOUNT_FACTOR: i128 = 800_000_000_000_000_000;

/// Load the current price for an asset from the oracle
pub fn load_price(e: &Env, oracle: &Address, asset: &Asset, max_price_age: u32) -> i128 {
    let price_data = match PriceFeedClient::new(e, oracle).lastprice(asset) {
        Some(price) => price,
        None => panic_with_error!(e, TradingError::PriceNotFound),
    };
    if price_data.timestamp + (max_price_age as u64) < e.ledger().timestamp() {
        panic_with_error!(e, TradingError::PriceStale);
    }
    price_data.price
}

/// Calculate funding rates based on market imbalance.
///
/// Returns (long_rate, short_rate) as hourly rates in SCALAR_18 precision.
///
/// Formula:
/// - Dominant side pays: `base_rate × ratio` (capped at ratio_cap)
/// - Minority side receives: `-0.8 × base_rate × ratio²`
/// - Equal positions: both pay base_rate
/// - One-sided market: existing side pays base_rate, empty side would receive 0.8×base_rate
pub fn calculate_funding_rates(
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

            let ratio = dominant
                .fixed_div_floor(e, &minority, &SCALAR_18)
                .min(ratio_cap);
            let squared = ratio.fixed_mul_floor(e, &ratio, &SCALAR_18);

            let pay = base_rate.fixed_mul_floor(e, &ratio, &SCALAR_18);
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

#[derive(Clone)]
#[contracttype]
pub struct Market {
    pub asset_index: u32,
    pub config: MarketConfig,
    pub data: MarketData,
}

impl Market {
    pub fn load(e: &Env, asset_index: u32) -> Market {
        let market_config = storage::get_market_config(e, asset_index);
        let market_data = storage::get_market_data(e, asset_index);
        Market {
            asset_index,
            config: market_config,
            data: market_data,
        }
    }

    pub fn store(&self, e: &Env) {
        storage::set_market_data(e, self.asset_index, &self.data);
    }

    /// Accrues funding interest based on long/short imbalance.
    ///
    /// The dominant side pays interest proportional to the imbalance ratio.
    /// The minority side receives interest at 0.8x the squared ratio.
    pub fn accrue_interest(&mut self, e: &Env) {
        let current_time = e.ledger().timestamp();
        let seconds_elapsed = (current_time - self.data.last_update) as i128;

        if seconds_elapsed <= 0 {
            return;
        }

        let (long_rate, short_rate) = calculate_funding_rates(
            e,
            self.data.long_notional_size,
            self.data.short_notional_size,
            self.config.base_hourly_rate,
            self.config.ratio_cap,
        );

        // Apply interest to indices (additive model)
        // rate is per hour, so we calculate: index += rate * hours_elapsed
        // First convert seconds to hours in SCALAR_18 precision to avoid truncation:
        // hours_scaled = (seconds * SCALAR_18) / 3600
        let hour = ONE_HOUR_SECONDS as i128;
        let hours_scaled = seconds_elapsed.fixed_mul_floor(e, &SCALAR_18, &hour);

        // Then: index += (rate * hours_scaled) / SCALAR_18 = rate * hours
        self.data.long_interest_index += long_rate.fixed_mul_floor(e, &hours_scaled, &SCALAR_18);
        self.data.short_interest_index += short_rate.fixed_mul_floor(e, &hours_scaled, &SCALAR_18);
        self.data.last_update = current_time;
    }

    /// Updates open interest statistics for an asset.
    /// Use positive values to add, negative values to subtract.
    pub fn update_stats(&mut self, notional_size: i128, is_long: bool) {
        if is_long {
            self.data.long_notional_size += notional_size;
        } else {
            self.data.short_notional_size += notional_size;
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
        let (long_rate, short_rate) = calculate_funding_rates(&e, 0, 0, BASE_RATE, RATIO_CAP);

        assert_eq!(long_rate, 0);
        assert_eq!(short_rate, 0);
    }

    #[test]
    fn test_only_longs() {
        let e = Env::default();
        let (long_rate, short_rate) =
            calculate_funding_rates(&e, 1000 * SCALAR_18, 0, BASE_RATE, RATIO_CAP);

        // Longs pay base rate
        assert_eq!(long_rate, BASE_RATE);
        // Shorts would receive 0.8x base rate (negative = receiving)
        assert_eq!(short_rate, -BASE_RATE * 8 / 10);
    }

    #[test]
    fn test_only_shorts() {
        let e = Env::default();
        let (long_rate, short_rate) =
            calculate_funding_rates(&e, 0, 1000 * SCALAR_18, BASE_RATE, RATIO_CAP);

        // Longs would receive 0.8x base rate
        assert_eq!(long_rate, -BASE_RATE * 8 / 10);
        // Shorts pay base rate
        assert_eq!(short_rate, BASE_RATE);
    }

    #[test]
    fn test_equal_positions() {
        let e = Env::default();
        let (long_rate, short_rate) =
            calculate_funding_rates(&e, 1000 * SCALAR_18, 1000 * SCALAR_18, BASE_RATE, RATIO_CAP);

        // Both sides pay base rate when balanced
        assert_eq!(long_rate, BASE_RATE);
        assert_eq!(short_rate, BASE_RATE);
    }

    #[test]
    fn test_long_dominant_2x() {
        let e = Env::default();
        // 2000 long vs 1000 short = 2x ratio
        let (long_rate, short_rate) =
            calculate_funding_rates(&e, 2000 * SCALAR_18, 1000 * SCALAR_18, BASE_RATE, RATIO_CAP);

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
            calculate_funding_rates(&e, 1000 * SCALAR_18, 2000 * SCALAR_18, BASE_RATE, RATIO_CAP);

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
            calculate_funding_rates(&e, 10000 * SCALAR_18, 1000 * SCALAR_18, BASE_RATE, RATIO_CAP);

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
            calculate_funding_rates(&e, 1000 * SCALAR_18, 10000 * SCALAR_18, BASE_RATE, RATIO_CAP);

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
            calculate_funding_rates(&e, 1000000 * SCALAR_18, SCALAR_18, BASE_RATE, RATIO_CAP);

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
            calculate_funding_rates(&e, 2000 * SCALAR_18, 1000 * SCALAR_18, BASE_RATE, RATIO_CAP);

        // Verify the 0.8 discount ensures vault profit
        // Total collected from longs: 2000 * 2 * BASE_RATE = 4000 * BASE_RATE
        // Total paid to shorts: 1000 * 3.2 * BASE_RATE = 3200 * BASE_RATE
        // Vault profit: 800 * BASE_RATE (20% of what longs pay)
        let long_payment = 2000 * long_rate;
        let short_receipt = 1000 * short_rate.abs();
        assert!(long_payment > short_receipt);
        assert_eq!(long_payment - short_receipt, 800 * BASE_RATE);
    }

    #[test]
    fn test_market_update_stats() {
        let e = Env::default();
        let mut market = Market {
            asset_index: 0,
            config: crate::types::MarketConfig {
                asset: sep_40_oracle::Asset::Other(soroban_sdk::Symbol::new(&e, "BTC")),
                enabled: true,
                max_payout: 10 * SCALAR_18,
                min_collateral: SCALAR_18,
                max_collateral: 1_000_000 * SCALAR_18,
                init_margin: 0_0100000,
                maintenance_margin: 0_0050000,
                base_fee: 0_0005000,
                price_impact_scalar: 8_000_000_000 * SCALAR_18,
                base_hourly_rate: BASE_RATE,
                ratio_cap: RATIO_CAP,
            },
            data: crate::types::MarketData {
                long_notional_size: 0,
                short_notional_size: 0,
                long_interest_index: 0,
                short_interest_index: 0,
                last_update: 0,
            },
        };

        // Add long position
        market.update_stats(10000, true);
        assert_eq!(market.data.long_notional_size, 10000);
        assert_eq!(market.data.short_notional_size, 0);

        // Add short position
        market.update_stats(5000, false);
        assert_eq!(market.data.long_notional_size, 10000);
        assert_eq!(market.data.short_notional_size, 5000);

        // Remove long position (negative values)
        market.update_stats(-5000, true);
        assert_eq!(market.data.long_notional_size, 5000);
    }

    #[test]
    fn test_market_load_and_store() {
        use crate::testutils::{create_trading, default_market, default_market_data};

        let e = Env::default();
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            // Set up market config and data in storage
            let config = default_market(&e);
            let mut data = default_market_data();
            data.long_notional_size = 1000 * SCALAR_18;
            data.short_notional_size = 500 * SCALAR_18;

            storage::set_market_config(&e, 0, &config);
            storage::set_market_data(&e, 0, &data);

            // Test Market::load
            let market = Market::load(&e, 0);
            assert_eq!(market.asset_index, 0);
            assert_eq!(market.data.long_notional_size, 1000 * SCALAR_18);
            assert_eq!(market.data.short_notional_size, 500 * SCALAR_18);

            // Modify and store
            let mut market = market;
            market.data.long_notional_size = 2000 * SCALAR_18;
            market.store(&e);

            // Verify stored correctly
            let loaded_data = storage::get_market_data(&e, 0);
            assert_eq!(loaded_data.long_notional_size, 2000 * SCALAR_18);
        });
    }

    #[test]
    fn test_market_accrue_interest() {
        use crate::testutils::{create_trading, default_market, default_market_data};
        use soroban_sdk::testutils::{Ledger, LedgerInfo};

        let e = Env::default();
        e.ledger().set(LedgerInfo {
            timestamp: 0,
            protocol_version: 25,
            sequence_number: 100,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3110400,
        });

        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let config = default_market(&e);
            let mut data = default_market_data();
            data.long_notional_size = 2000 * SCALAR_18;
            data.short_notional_size = 1000 * SCALAR_18;
            data.last_update = 0;

            storage::set_market_config(&e, 0, &config);
            storage::set_market_data(&e, 0, &data);

            // Advance time by 1 hour
            e.ledger().set(LedgerInfo {
                timestamp: 3600,
                protocol_version: 25,
                sequence_number: 200,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3110400,
            });

            let mut market = Market::load(&e, 0);
            market.accrue_interest(&e);

            // Interest should have accrued
            assert!(market.data.long_interest_index != SCALAR_18);
            assert_eq!(market.data.last_update, 3600);
        });
    }
}
