use crate::constants::{ONE_HOUR_SECONDS, SCALAR_7, SCALAR_18};
use crate::dependencies::{VaultClient, TreasuryClient};
use crate::errors::TradingError;
use crate::storage;
use crate::trading::position::{Position, Settlement};
use crate::types::{MarketConfig, MarketData, TradingConfig};
use crate::dependencies::{PriceData, scalar_from_exponent};
use crate::trading::rates;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{panic_with_error, Address, Env};

/// Full context for any market operation: per-market + global state.
pub struct Market {
    // Per-market
    pub feed_id: u32,
    pub price: i128,
    pub price_scalar: i128,
    pub config: MarketConfig,
    pub data: MarketData,
    // Global
    pub trading_config: TradingConfig,
    pub vault: Address,
    pub vault_balance: i128,
    pub token: Address,
    pub treasury: Address,
    pub total_notional: i128,
    pub publish_time: u64, // from PriceData, used for liquidation stale-price guard
}

impl Market {
    /// Load full market context from storage and accrue to current timestamp.
    pub fn load(e: &Env, price_data: &PriceData) -> Self {
        let feed_id = price_data.feed_id;
        let trading_config = storage::get_config(e);
        let vault = storage::get_vault(e);
        let vault_balance = VaultClient::new(e, &vault).total_assets();
        let token = storage::get_token(e);
        let treasury = storage::get_treasury(e);
        let total_notional = storage::get_total_notional(e);
        let mut data = storage::get_market_data(e, feed_id);
        let config = storage::get_market_config(e, feed_id);
        data.accrue(e, trading_config.r_base, trading_config.r_var, config.r_borrow, vault_balance);
        Market {
            feed_id,
            price: price_data.price,
            price_scalar: scalar_from_exponent(price_data.exponent),
            config,
            data,
            trading_config,
            vault,
            vault_balance,
            token,
            treasury,
            total_notional,
            publish_time: price_data.publish_time,
        }
    }

    /// Panics if per-market or global utilization exceeds caps.
    fn require_within_util(&self, e: &Env) {
        if self.vault_balance <= 0 {
            panic_with_error!(e, TradingError::UtilizationExceeded);
        }
        let market_notional = self.data.l_notional + self.data.s_notional;
        let market_util = market_notional.fixed_div_ceil(e, &self.vault_balance, &SCALAR_7);
        if market_util > self.config.max_util {
            panic_with_error!(e, TradingError::UtilizationExceeded);
        }
        let global_util = self.total_notional.fixed_div_ceil(e, &self.vault_balance, &SCALAR_7);
        if global_util > self.trading_config.max_util {
            panic_with_error!(e, TradingError::UtilizationExceeded);
        }
    }

    pub(crate) fn treasury_fee(&self, e: &Env, revenue: i128) -> i128 {
        if revenue > 0 {
            TreasuryClient::new(e, &self.treasury).get_fee(&revenue)
        } else {
            0
        }
    }

    /// Open a position: compute fees, deduct from collateral, fill, register stats.
    /// Returns (base_fee, impact_fee).
    pub fn open(&mut self, e: &Env, position: &mut Position, position_id: u32) -> (i128, i128) {
        let base_fee = if self.data.is_dominant(position.long, position.notional) {
            position.notional.fixed_mul_ceil(e, &self.trading_config.fee_dom, &SCALAR_7)
        } else {
            position.notional.fixed_mul_ceil(e, &self.trading_config.fee_non_dom, &SCALAR_7)
        };
        let impact_fee = position.notional.fixed_div_ceil(e, &self.config.impact, &SCALAR_7);

        position.col -= base_fee + impact_fee;
        position.validate(e, self.config.enabled, self.trading_config.min_notional, self.trading_config.max_notional, self.config.margin);
        position.fill(e, &self.data);
        storage::set_position(e, position_id, position);

        let ew_delta = position.notional.fixed_div_floor(e, &position.entry_price, &self.price_scalar);
        self.data.update_stats(position.long, position.notional, ew_delta);
        self.total_notional += position.notional;
        self.require_within_util(e);

        (base_fee, impact_fee)
    }

    /// Close a position: settle PnL/fees, update market stats, remove from storage.
    pub fn close(&mut self, e: &Env, position: &mut Position, position_id: u32) -> Settlement {
        let s = position.settle(e, self);
        let ew_delta = position.notional.fixed_div_floor(e, &position.entry_price, &self.price_scalar);
        self.data.update_stats(position.long, -position.notional, ew_delta);
        self.total_notional -= position.notional;
        storage::remove_user_position(e, &position.user, position_id);
        storage::remove_position(e, position_id);
        s
    }

    /// Write mutable state back to storage.
    pub fn store(&self, e: &Env) {
        storage::set_market_data(e, self.feed_id, &self.data);
        storage::set_total_notional(e, self.total_notional);
    }
}

impl Default for MarketData {
    fn default() -> Self {
        Self {
            l_notional: 0,
            s_notional: 0,
            l_fund_idx: 0,
            s_fund_idx: 0,
            l_borr_idx: 0,
            s_borr_idx: 0,
            l_entry_wt: 0,
            s_entry_wt: 0,
            fund_rate: 0,
            last_update: 0,
            l_adl_idx: SCALAR_18,
            s_adl_idx: SCALAR_18,
        }
    }
}

impl MarketData {
    /// Returns (funding_index, borrowing_index, adl_index) for the given side.
    pub fn indices(&self, is_long: bool) -> (i128, i128, i128) {
        if is_long {
            (self.l_fund_idx, self.l_borr_idx, self.l_adl_idx)
        } else {
            (self.s_fund_idx, self.s_borr_idx, self.s_adl_idx)
        }
    }

    /// Returns true if the given side is dominant (has more notional).
    /// `extra` is additional notional being added/removed.
    pub fn is_dominant(&self, is_long: bool, extra: i128) -> bool {
        if is_long {
            self.l_notional + extra > self.s_notional
        } else {
            self.s_notional + extra > self.l_notional
        }
    }

    /// Accrue borrowing then funding indices. Computes borrow rate from global curve params and per-market weight.
    pub fn accrue(&mut self, e: &Env, r_base: i128, r_var: i128, r_borrow: i128, vault_balance: i128) {
        let current_time = e.ledger().timestamp();
        let seconds = current_time.saturating_sub(self.last_update) as i128;
        self.last_update = current_time;

        if seconds == 0 {
            return;
        }

        let hour = ONE_HOUR_SECONDS as i128;

        // Borrowing: dominant side only
        let market_notional = self.l_notional + self.s_notional;
        let util = if market_notional > 0 && vault_balance > 0 {
            market_notional.fixed_div_ceil(e, &vault_balance, &SCALAR_7).min(SCALAR_7)
        } else {
            0
        };
        let borr_rate = rates::calc_borrowing_rate(e, r_base, r_var, r_borrow, util);

        if borr_rate > 0 {
            let borrow_delta = borr_rate.fixed_mul_ceil(e, &seconds, &hour);
            if self.l_notional > self.s_notional {
                self.l_borr_idx += borrow_delta;
            } else if self.s_notional > self.l_notional {
                self.s_borr_idx += borrow_delta;
            } else if self.l_notional > 0 {
                self.l_borr_idx += borrow_delta;
                self.s_borr_idx += borrow_delta;
            }
        }

        // Funding: peer-to-peer — skip if either side is empty (no counterparty)
        if self.fund_rate == 0 || self.l_notional == 0 || self.s_notional == 0 {
            return;
        }

        let pay_delta = self.fund_rate.abs().fixed_mul_ceil(e, &seconds, &hour);

        let (pay_notional, recv_notional) = if self.fund_rate > 0 {
            (self.l_notional, self.s_notional)
        } else {
            (self.s_notional, self.l_notional)
        };

        let recv_delta = if recv_notional > 0 {
            let ratio = pay_notional.fixed_div_floor(e, &recv_notional, &SCALAR_18);
            pay_delta.fixed_mul_floor(e, &ratio, &SCALAR_18)
        } else {
            0
        };

        if self.fund_rate > 0 {
            self.l_fund_idx += pay_delta;
            self.s_fund_idx -= recv_delta;
        } else {
            self.s_fund_idx += pay_delta;
            self.l_fund_idx -= recv_delta;
        }
    }

    pub fn update_funding_rate(&mut self, e: &Env, base_funding_rate: i128) {
        self.fund_rate = rates::calc_funding_rate(
            e,
            self.l_notional,
            self.s_notional,
            base_funding_rate,
        );
    }

    /// Updates open interest and entry-weighted aggregate stats.
    /// notional_size: positive for open, negative for close/reduce.
    /// ew_delta: pre-computed |notional| / entry_price in price_scalar precision.
    pub fn update_stats(&mut self, is_long: bool, notional_size: i128, ew_delta: i128) {
        if is_long {
            self.l_notional += notional_size;
            if notional_size > 0 {
                self.l_entry_wt += ew_delta;
            } else {
                self.l_entry_wt -= ew_delta;
            }
        } else {
            self.s_notional += notional_size;
            if notional_size > 0 {
                self.s_entry_wt += ew_delta;
            } else {
                self.s_entry_wt -= ew_delta;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::constants::{SCALAR_7, SCALAR_18};
    use crate::testutils::{create_trading, default_market, default_market_data, jump, BTC_FEED_ID};
    use crate::storage;
    use soroban_sdk::Env;

    #[test]
    fn test_market_data_update_stats() {
        let e = Env::default();
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            use soroban_fixed_point_math::SorobanFixedPoint;

            let scalar_7: i128 = 10_000_000;
            let price_scalar = scalar_7;
            let entry_price: i128 = 100_000 * scalar_7;

            let mut data = default_market_data();

            let notional_long = 10_000 * scalar_7;
            let notional_short = 5_000 * scalar_7;

            // Add long position
            let ew = notional_long.fixed_div_floor(&e, &entry_price, &price_scalar);
            data.update_stats(true, notional_long, ew);
            assert_eq!(data.l_notional, notional_long);
            assert_eq!(data.s_notional, 0);
            assert!(data.l_entry_wt > 0);

            // Add short position
            let ew = notional_short.fixed_div_floor(&e, &entry_price, &price_scalar);
            data.update_stats(false, notional_short, ew);
            assert_eq!(data.l_notional, notional_long);
            assert_eq!(data.s_notional, notional_short);
            assert!(data.s_entry_wt > 0);

            // Remove long position (negative values)
            let ew = notional_short.fixed_div_floor(&e, &entry_price, &price_scalar);
            data.update_stats(true, -notional_short, ew);
            assert_eq!(data.l_notional, notional_long - notional_short);
        });
    }

    #[test]
    fn test_market_data_load_and_store() {
        let e = Env::default();
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let config = default_market(&e);
            let mut data = default_market_data();
            data.l_notional = 1000 * SCALAR_18;
            data.s_notional = 500 * SCALAR_18;

            storage::set_market_config(&e, BTC_FEED_ID, &config);
            storage::set_market_data(&e, BTC_FEED_ID, &data);

            // Load and verify
            let loaded = storage::get_market_data(&e, BTC_FEED_ID);
            assert_eq!(loaded.l_notional, 1000 * SCALAR_18);
            assert_eq!(loaded.s_notional, 500 * SCALAR_18);

            // Modify and store
            let mut loaded = loaded;
            loaded.l_notional = 2000 * SCALAR_18;
            storage::set_market_data(&e, BTC_FEED_ID, &loaded);

            // Verify stored correctly
            let reloaded = storage::get_market_data(&e, BTC_FEED_ID);
            assert_eq!(reloaded.l_notional, 2000 * SCALAR_18);
        });
    }

    #[test]
    fn test_accrue_funding_longs_pay() {
        let e = Env::default();
        jump(&e, 0);
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let mut data = default_market_data();
            data.l_notional = 2000 * SCALAR_18;
            data.s_notional = 1000 * SCALAR_18;
            data.fund_rate = 10_000_000_000_000; // positive = longs pay
            data.last_update = 0;

            jump(&e, 3600);
            data.accrue(&e, 0, 0, 0, 0); // no borrowing, only funding accrues

            assert!(data.l_fund_idx > 0);
            assert_eq!(data.last_update, 3600);
        });
    }

    #[test]
    fn test_accrue_borrowing_longs_dominant() {
        let e = Env::default();
        jump(&e, 0);
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let mut data = default_market_data();
            data.l_notional = 2000 * SCALAR_7;
            data.s_notional = 1000 * SCALAR_7;
            data.last_update = 0;

            jump(&e, 3600);
            // r_var=0 so base_rate=r_base, r_borrow=SCALAR_7 (1x) so borr_rate=r_base
            let r_base: i128 = 10_000_000_000_000;
            let vault_balance = 100_000 * SCALAR_7;
            data.accrue(&e, r_base, 0, SCALAR_7, vault_balance);

            assert!(data.l_borr_idx > 0, "dominant longs should accrue");
            assert_eq!(data.s_borr_idx, 0, "non-dominant shorts should NOT accrue");
        });
    }

    #[test]
    fn test_accrue_borrowing_shorts_dominant() {
        let e = Env::default();
        jump(&e, 0);
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let mut data = default_market_data();
            data.l_notional = 1000 * SCALAR_7;
            data.s_notional = 2000 * SCALAR_7;
            data.last_update = 0;

            jump(&e, 3600);
            let r_base: i128 = 10_000_000_000_000;
            let vault_balance = 100_000 * SCALAR_7;
            data.accrue(&e, r_base, 0, SCALAR_7, vault_balance);

            assert_eq!(data.l_borr_idx, 0, "non-dominant longs should NOT accrue");
            assert!(data.s_borr_idx > 0, "dominant shorts should accrue");
        });
    }

    #[test]
    fn test_accrue_borrowing_balanced_both_pay() {
        let e = Env::default();
        jump(&e, 0);
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let mut data = default_market_data();
            data.l_notional = 1000 * SCALAR_7;
            data.s_notional = 1000 * SCALAR_7;
            data.last_update = 0;

            jump(&e, 3600);
            let r_base: i128 = 10_000_000_000_000;
            let vault_balance = 100_000 * SCALAR_7;
            data.accrue(&e, r_base, 0, SCALAR_7, vault_balance);

            assert!(data.l_borr_idx > 0, "balanced — both sides pay borrowing");
            assert!(data.s_borr_idx > 0, "balanced — both sides pay borrowing");
            assert_eq!(data.l_borr_idx, data.s_borr_idx, "balanced — same rate both sides");
        });
    }
}
