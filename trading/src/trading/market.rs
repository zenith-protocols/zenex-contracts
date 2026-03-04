use crate::constants::{ONE_HOUR_SECONDS, SCALAR_18};
use crate::storage;
use crate::trading::interest::calc_funding_rate;
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

    /// Accrues funding using the stored signed rate.
    /// Payers pay full delta per unit. Receivers get: pay_delta × 0.8 × (dominant/minority) per unit.
    /// Total received is always exactly 80% of total paid (self-balancing).
    pub fn accrue(&mut self, e: &Env, vault_skim: i128, token_scalar: i128) {
        let current_time = e.ledger().timestamp();
        let seconds_elapsed = (current_time - self.data.last_update) as i128;

        if seconds_elapsed <= 0 {
            return;
        }

        let hour = ONE_HOUR_SECONDS as i128;
        let hours_scaled = seconds_elapsed.fixed_mul_floor(e, &SCALAR_18, &hour);
        let discount_factor = (token_scalar - vault_skim)
            .fixed_mul_floor(e, &SCALAR_18, &token_scalar);

        let pay_delta = self
            .data
            .funding_rate
            .abs()
            .fixed_mul_ceil(e, &hours_scaled, &SCALAR_18);

        if self.data.funding_rate > 0 {
            // Longs pay
            self.data.long_funding_index += pay_delta;
            // Shorts receive (scaled by L/S ratio)
            if self.data.short_notional_size > 0 {
                let ratio = self.data.long_notional_size
                    .fixed_div_floor(e, &self.data.short_notional_size, &SCALAR_18);
                let receive_delta = pay_delta
                    .fixed_mul_floor(e, &discount_factor, &SCALAR_18)
                    .fixed_mul_floor(e, &ratio, &SCALAR_18);
                self.data.short_funding_index -= receive_delta;
            }
        } else if self.data.funding_rate < 0 {
            // Shorts pay
            self.data.short_funding_index += pay_delta;
            // Longs receive (scaled by S/L ratio)
            if self.data.long_notional_size > 0 {
                let ratio = self.data.short_notional_size
                    .fixed_div_floor(e, &self.data.long_notional_size, &SCALAR_18);
                let receive_delta = pay_delta
                    .fixed_mul_floor(e, &discount_factor, &SCALAR_18)
                    .fixed_mul_floor(e, &ratio, &SCALAR_18);
                self.data.long_funding_index -= receive_delta;
            }
        }
        self.data.last_update = current_time;
    }

    /// Recalculate the funding rate based on current market state.
    pub fn update_funding_rate(&mut self, e: &Env) {
        self.data.funding_rate = calc_funding_rate(
            e,
            self.data.long_notional_size,
            self.data.short_notional_size,
            self.config.base_hourly_rate,
        );
    }

    /// Updates open interest and entry-weighted aggregate stats.
    /// notional_size: positive for open, negative for close/reduce.
    /// entry_price: the position's entry price (price_decimals).
    pub fn update_stats(&mut self, e: &Env, notional_size: i128, is_long: bool, entry_price: i128, price_scalar: i128) {
        let abs_notional = notional_size.abs();
        let ew_delta = abs_notional.fixed_div_floor(e, &entry_price, &price_scalar);

        if is_long {
            self.data.long_notional_size += notional_size;
            if notional_size > 0 {
                self.data.long_entry_weighted += ew_delta;
            } else {
                self.data.long_entry_weighted -= ew_delta;
            }
        } else {
            self.data.short_notional_size += notional_size;
            if notional_size > 0 {
                self.data.short_entry_weighted += ew_delta;
            } else {
                self.data.short_entry_weighted -= ew_delta;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils::{create_trading, default_market_data};
    use soroban_sdk::Env;

    // Base rate: 0.001% per hour = 10^13 in SCALAR_18
    const BASE_RATE: i128 = 10_000_000_000_000;

    #[test]
    fn test_market_update_stats() {
        let e = Env::default();
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let scalar_7: i128 = 10_000_000;
            let price_scalar = scalar_7;
            let entry_price: i128 = 100_000 * scalar_7; // $100,000 in 7 decimals

            let mut market = Market {
                asset_index: 0,
                config: crate::types::MarketConfig {
                    asset: sep_40_oracle::Asset::Other(soroban_sdk::Symbol::new(&e, "BTC")),
                    enabled: true,
                    init_margin: 0_0100000,
                    base_hourly_rate: BASE_RATE,
                    price_impact_scalar: 8_000_000_000 * scalar_7,
                },
                data: default_market_data(),
            };

            let notional_long = 10_000 * scalar_7;
            let notional_short = 5_000 * scalar_7;

            // Add long position
            market.update_stats(&e, notional_long, true, entry_price, price_scalar);
            assert_eq!(market.data.long_notional_size, notional_long);
            assert_eq!(market.data.short_notional_size, 0);
            assert!(market.data.long_entry_weighted > 0);

            // Add short position
            market.update_stats(&e, notional_short, false, entry_price, price_scalar);
            assert_eq!(market.data.long_notional_size, notional_long);
            assert_eq!(market.data.short_notional_size, notional_short);
            assert!(market.data.short_entry_weighted > 0);

            // Remove long position (negative values)
            market.update_stats(&e, -notional_short, true, entry_price, price_scalar);
            assert_eq!(market.data.long_notional_size, notional_long - notional_short);
        });
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
    fn test_market_accrue() {
        use crate::testutils::{create_trading, default_market, default_market_data, jump};

        let e = Env::default();
        jump(&e, 0);

        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let config = default_market(&e);
            let mut data = default_market_data();
            data.long_notional_size = 2000 * SCALAR_18;
            data.short_notional_size = 1000 * SCALAR_18;
            data.funding_rate = 10_000_000_000_000; // positive = longs pay
            data.last_update = 0;

            storage::set_market_config(&e, 0, &config);
            storage::set_market_data(&e, 0, &data);

            // Advance time by 1 hour
            jump(&e, 3600);

            let mut market = Market::load(&e, 0);
            let vault_skim = 0_2000000; // 20%
            let token_scalar = 10_000_000i128;
            market.accrue(&e, vault_skim, token_scalar);

            // Funding should have accrued — longs pay, index increases
            assert!(market.data.long_funding_index > 0);
            assert_eq!(market.data.last_update, 3600);
        });
    }
}
