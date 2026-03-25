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

/// Full context needed for any market operation.
///
/// Bundles per-market state (config, data, price) with global state (trading config,
/// vault balance, token/vault/treasury addresses). Loaded once at the start of an
/// operation via [`Market::load`], mutated in-place, then persisted via [`Market::store`].
///
/// WHY: auto-accrue on load -- every Market::load call accrues borrowing and funding
/// indices to the current timestamp, so all subsequent operations see up-to-date
/// cumulative rates without the caller needing to remember to accrue first.
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
    /// Load full market context from storage and accrue indices to current timestamp.
    ///
    /// # Parameters
    /// - `price_data` - Verified price data from the oracle (contains feed_id, price, exponent)
    ///
    /// # Side effects
    /// - Calls `MarketData::accrue()` to advance borrowing and funding indices
    /// - Computes `price_scalar = 10^(-exponent)` from Pyth exponent
    ///
    /// WHY: price_scalar is computed per-call rather than stored because the exponent
    /// comes from the oracle and could theoretically change between feed updates.
    /// Computing it fresh avoids stale scalar bugs.
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

    /// Open a position: compute fees, deduct from collateral, fill, and update market stats.
    ///
    /// # Parameters
    /// - `position` - Mutable position to fill (collateral reduced by fees)
    /// - `position_id` - Storage key for the position
    ///
    /// # Returns
    /// `(base_fee, impact_fee)` -- both in token_decimals.
    ///
    /// # Fee logic
    /// - `base_fee`: dominant-side openings pay `fee_dom`, non-dominant pay `fee_non_dom`
    ///   (SCALAR_7 fraction of notional). WHY: opening on the dominant side worsens
    ///   market imbalance, so the higher fee disincentivizes that.
    /// - `impact_fee`: `notional / impact` (SCALAR_7), simulates price impact.
    ///
    /// # Panics
    /// - `TradingError::UtilizationExceeded` (791) if position pushes utilization past caps
    /// - All panics from `Position::validate()`
    pub fn open(&mut self, e: &Env, position: &mut Position, position_id: u32) -> (i128, i128) {
        let base_fee = if self.data.is_dominant(position.long, position.notional) {
            // WHY: ceil rounding on fees -- protocol never under-charges
            position.notional.fixed_mul_ceil(e, &self.trading_config.fee_dom, &SCALAR_7)
        } else {
            position.notional.fixed_mul_ceil(e, &self.trading_config.fee_non_dom, &SCALAR_7)
        };
        let impact_fee = position.notional.fixed_div_ceil(e, &self.config.impact, &SCALAR_7);

        // WHY: fees deducted from collateral before validation -- ensures post-fee
        // collateral still meets margin requirements, preventing under-collateralized positions.
        position.col -= base_fee + impact_fee;
        position.validate(e, self.config.enabled, self.trading_config.min_notional, self.trading_config.max_notional, self.config.margin);
        position.fill(e, &self.data);
        storage::set_position(e, position_id, position);

        // WHY: entry_wt (entry-weighted aggregate) tracks Sigma(notional/entry_price) per side.
        // This enables O(1) PnL calculation for the entire side during ADL checks,
        // without iterating over every position.
        // floor rounding on entry_wt -- conservative (slightly understates aggregate weight).
        let ew_delta = position.notional.fixed_div_floor(e, &position.entry_price, &self.price_scalar);
        self.data.update_stats(position.long, position.notional, ew_delta);
        self.total_notional += position.notional;
        self.require_within_util(e);

        (base_fee, impact_fee)
    }

    /// Close a position: settle PnL and all accrued fees, update market stats, remove from storage.
    ///
    /// # Parameters
    /// - `position` - Mutable position to settle (notional may be reduced by ADL)
    /// - `position_id` - Storage key (position + user tracking removed)
    ///
    /// # Returns
    /// [`Settlement`] with broken-down PnL and fee components.
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

    /// Accrue borrowing then funding indices to the current ledger timestamp.
    ///
    /// # Parameters
    /// - `r_base` - Global base borrowing rate (SCALAR_18, hourly)
    /// - `r_var` - Variable borrowing multiplier (SCALAR_7)
    /// - `r_borrow` - Per-market borrowing weight (SCALAR_7)
    /// - `vault_balance` - Current vault total assets (token_decimals)
    ///
    /// # Accrual order
    /// 1. **Borrowing** (dominant side only): `borr_delta = borr_rate × elapsed / 3600`
    ///    - If longs dominate: only `l_borr_idx` increases
    ///    - If shorts dominate: only `s_borr_idx` increases
    ///    - If balanced: both sides accrue equally
    /// 2. **Funding** (peer-to-peer, skipped if either side is empty):
    ///    - Paying side: `fund_idx += pay_delta`
    ///    - Receiving side: `fund_idx -= recv_delta` (negative = gain)
    ///    - `recv_delta` is scaled by `pay_notional / recv_notional` to conserve the total
    ///
    /// WHY: borrowing accrues before funding so that the borrowing index reflects
    /// the pre-funding state. This prevents circular dependency between the two rates.
    ///
    /// WHY: funding skips if either side is zero -- no counterparty to pay/receive.
    pub fn accrue(&mut self, e: &Env, r_base: i128, r_var: i128, r_borrow: i128, vault_balance: i128) {
        let current_time = e.ledger().timestamp();
        let seconds = current_time.saturating_sub(self.last_update) as i128;
        self.last_update = current_time;

        if seconds == 0 {
            return;
        }

        let hour = ONE_HOUR_SECONDS as i128;

        // WHY: Only the dominant (heavier) side pays borrowing. The non-dominant
        // side does not accrue borrowing because their positions reduce systemic risk.
        // When balanced, both sides pay equally as neither is dominant.
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

        // WHY: Funding is peer-to-peer — skip if either side is empty because
        // there is no counterparty to receive/pay the funding. Accumulating
        // a one-sided index would create an unrecoverable debt.
        if self.fund_rate == 0 || self.l_notional == 0 || self.s_notional == 0 {
            return;
        }

        let pay_delta = self.fund_rate.abs().fixed_mul_ceil(e, &seconds, &hour);

        let (pay_notional, recv_notional) = if self.fund_rate > 0 {
            (self.l_notional, self.s_notional)
        } else {
            (self.s_notional, self.l_notional)
        };

        // WHY: recv_delta is scaled by (pay_notional / recv_notional) so that
        // the total funding paid by the dominant side equals the total received
        // by the non-dominant side. Floor rounding on receive -- paying side never
        // pays less than what receivers get (any rounding residual stays in the system).
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
