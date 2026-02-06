use crate::constants::{ONE_HOUR_SECONDS, SCALAR_18};
use crate::storage;
use crate::trading::interest::calc_interest;
use crate::types::{MarketConfig, MarketData};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{contracttype, Env};

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

        let (long_rate, short_rate) = calc_interest(
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
    use crate::testutils::default_market_data;

    // Base rate: 0.001% per hour = 10^13 in SCALAR_18
    const BASE_RATE: i128 = 10_000_000_000_000;
    // 5x ratio cap
    const RATIO_CAP: i128 = 5 * SCALAR_18;

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
            data: default_market_data()
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
        use crate::testutils::{create_trading, default_market, default_market_data, jump};

        let e = Env::default();
        jump(&e, 0);

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
            jump(&e, 3600);

            let mut market = Market::load(&e, 0);
            market.accrue_interest(&e);

            // Interest should have accrued
            assert_ne!(market.data.long_interest_index, SCALAR_18);
            assert_eq!(market.data.last_update, 3600);
        });
    }
}
