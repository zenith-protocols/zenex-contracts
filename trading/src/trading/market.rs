use crate::trading::funding;
use crate::types::MarketData;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::Env;

impl MarketData {
    pub fn accrue(&mut self, e: &Env) {
        funding::accrue_funding(e, self);
    }

    pub fn update_funding_rate(&mut self, e: &Env, base_hourly_rate: i128) {
        self.funding_rate = funding::calc_funding_rate(
            e,
            self.long_notional_size,
            self.short_notional_size,
            base_hourly_rate,
        );
    }

    /// Updates open interest and entry-weighted aggregate stats.
    /// notional_size: positive for open, negative for close/reduce.
    /// entry_price: the position's entry price (price_decimals).
    pub fn update_stats(&mut self, e: &Env, notional_size: i128, is_long: bool, entry_price: i128, price_scalar: i128) {
        let abs_notional = notional_size.abs();
        let ew_delta = abs_notional.fixed_div_floor(e, &entry_price, &price_scalar);

        if is_long {
            self.long_notional_size += notional_size;
            if notional_size > 0 {
                self.long_entry_weighted += ew_delta;
            } else {
                self.long_entry_weighted -= ew_delta;
            }
        } else {
            self.short_notional_size += notional_size;
            if notional_size > 0 {
                self.short_entry_weighted += ew_delta;
            } else {
                self.short_entry_weighted -= ew_delta;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::constants::SCALAR_18;
    use crate::testutils::{create_trading, default_market, default_market_data, BTC_FEED_ID};
    use crate::storage;
    use soroban_sdk::Env;

    #[test]
    fn test_market_data_update_stats() {
        let e = Env::default();
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let scalar_7: i128 = 10_000_000;
            let price_scalar = scalar_7;
            let entry_price: i128 = 100_000 * scalar_7; // $100,000 in 7 decimals

            let mut data = default_market_data();

            let notional_long = 10_000 * scalar_7;
            let notional_short = 5_000 * scalar_7;

            // Add long position
            data.update_stats(&e, notional_long, true, entry_price, price_scalar);
            assert_eq!(data.long_notional_size, notional_long);
            assert_eq!(data.short_notional_size, 0);
            assert!(data.long_entry_weighted > 0);

            // Add short position
            data.update_stats(&e, notional_short, false, entry_price, price_scalar);
            assert_eq!(data.long_notional_size, notional_long);
            assert_eq!(data.short_notional_size, notional_short);
            assert!(data.short_entry_weighted > 0);

            // Remove long position (negative values)
            data.update_stats(&e, -notional_short, true, entry_price, price_scalar);
            assert_eq!(data.long_notional_size, notional_long - notional_short);
        });
    }

    #[test]
    fn test_market_data_load_and_store() {
        let e = Env::default();
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let config = default_market(&e);
            let mut data = default_market_data();
            data.long_notional_size = 1000 * SCALAR_18;
            data.short_notional_size = 500 * SCALAR_18;

            storage::set_market_config(&e, BTC_FEED_ID, &config);
            storage::set_market_data(&e, BTC_FEED_ID, &data);

            // Load and verify
            let loaded = storage::get_market_data(&e, BTC_FEED_ID);
            assert_eq!(loaded.long_notional_size, 1000 * SCALAR_18);
            assert_eq!(loaded.short_notional_size, 500 * SCALAR_18);

            // Modify and store
            let mut loaded = loaded;
            loaded.long_notional_size = 2000 * SCALAR_18;
            storage::set_market_data(&e, BTC_FEED_ID, &loaded);

            // Verify stored correctly
            let reloaded = storage::get_market_data(&e, BTC_FEED_ID);
            assert_eq!(reloaded.long_notional_size, 2000 * SCALAR_18);
        });
    }
}
