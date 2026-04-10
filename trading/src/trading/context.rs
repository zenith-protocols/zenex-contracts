use crate::constants::SCALAR_7;
use crate::dependencies::{VaultClient, TreasuryClient};
use crate::errors::TradingError;
use crate::storage;
use crate::trading::position::{Position, Settlement};
use crate::types::{MarketConfig, MarketData, TradingConfig};
use crate::dependencies::{PriceData, scalar_from_exponent};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{panic_with_error, Address, Env};

/// Full context needed for any market operation.
///
/// Bundles per-market state (config, data, price) with global state (trading config,
/// vault balance, token/vault/treasury addresses). Loaded once at the start of an
/// operation via [`Context::load`], mutated in-place, then persisted via [`Context::store`].
///
/// auto-accrue on load every Context::load call accrues borrowing and funding
/// indices to the current timestamp, so all subsequent operations see up-to-date
/// cumulative rates.
pub struct Context {
    // Per-market
    pub market_id:    u32,
    pub feed_id:      u32,
    pub price:        i128,
    pub price_scalar: i128,
    pub publish_time: u64,
    pub config:       MarketConfig,
    pub data:         MarketData,
    // Global
    pub trading_config: TradingConfig,
    pub vault:          Address,
    pub vault_balance:  i128,
    pub token:          Address,
    pub treasury:       Address,
    pub total_notional: i128,
}

impl Context {
    /// Load full market context from storage and accrue indices to current timestamp.
    ///
    /// # Parameters
    /// - `market_id` - Market identifier (storage key)
    /// - `price_data` - Verified price data from the oracle (contains feed_id, price, exponent)
    ///
    /// # Side effects
    /// - Calls `MarketData::accrue()` to advance borrowing and funding indices
    /// - Computes `price_scalar = 10^(-exponent)` from Pyth exponent
    ///
    /// # Panics
    /// - `TradingError::InvalidPrice` if `price_data.feed_id != config.feed_id`
    pub fn load(e: &Env, market_id: u32, price_data: &PriceData) -> Self {
        let trading_config = storage::get_config(e);
        let vault = storage::get_vault(e);
        let vault_balance = VaultClient::new(e, &vault).total_assets();
        let token = storage::get_token(e);
        let treasury = storage::get_treasury(e);
        let total_notional = storage::get_total_notional(e);
        let config = storage::get_market_config(e, market_id);
        if price_data.feed_id != config.feed_id {
            panic_with_error!(e, TradingError::InvalidPrice);
        }
        let mut data = storage::get_market_data(e, market_id);
        data.accrue(
            e,
            trading_config.r_base,
            trading_config.r_var,
            config.r_var_market,
            vault_balance,
            total_notional,
            trading_config.max_util,
            config.max_util,
        );
        Context {
            market_id,
            feed_id: config.feed_id,
            price: price_data.price,
            price_scalar: scalar_from_exponent(price_data.exponent),
            publish_time: price_data.publish_time,
            config,
            data,
            trading_config,
            vault,
            vault_balance,
            token,
            treasury,
            total_notional,
        }
    }

    /// Panics if per-market or global utilization exceeds caps.
    ///
    /// Computes util = notional / vault_balance directly (not scaled by max_util
    /// like `calc_util` used in rate computation). The bound check against
    /// `config.max_util` is equivalent: notional / vault_balance <= max_util.
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

    /// Compute the treasury's cut from a revenue amount.
    ///
    /// Returns `floor(revenue × rate / SCALAR_7)` where rate is queried from
    /// the treasury contract (SCALAR_7 fraction, e.g. 500_000 = 5%).
    /// Returns 0 when revenue <= 0 or rate is 0.
    pub(crate) fn treasury_fee(&self, e: &Env, revenue: i128) -> i128 {
        if revenue > 0 {
            let rate = TreasuryClient::new(e, &self.treasury).get_rate();
            if rate > 0 {
                revenue.fixed_mul_floor(e, &rate, &SCALAR_7)
            } else {
                0
            }
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
    /// `(base_fee, impact_fee)` both in token_decimals.
    ///
    /// # Fee logic
    /// - `base_fee`: dominant-side openings pay `fee_dom`, non-dominant pay `fee_non_dom`
    ///   (SCALAR_7 fraction of notional). Opening on the dominant side worsens
    ///   market imbalance, so the higher fee disincentivizes that.
    /// - `impact_fee`: `notional / impact` (SCALAR_7), simulates price impact.
    ///
    /// # Panics
    /// - `TradingError::UtilizationExceeded` (751) if position pushes utilization past caps
    /// - All panics from `Position::validate()`
    pub fn open(&mut self, e: &Env, position: &mut Position, position_id: u32) -> (i128, i128) {
        let base_fee = if self.data.is_dominant(position.long, position.notional) {
            position.notional.fixed_mul_ceil(e, &self.trading_config.fee_dom, &SCALAR_7)
        } else {
            position.notional.fixed_mul_ceil(e, &self.trading_config.fee_non_dom, &SCALAR_7)
        };
        let impact_fee = position.notional.fixed_div_floor(e, &self.config.impact, &SCALAR_7);

        // fees deducted from collateral before validation, ensures post-fee
        // collateral still meets margin requirements, preventing under-collateralized positions.
        position.col -= base_fee + impact_fee;
        position.validate(e, self.config.enabled, self.trading_config.min_notional, self.trading_config.max_notional, self.config.margin);
        position.fill(e, &self.data);
        storage::set_position(e, position_id, position);

        // entry_wt (entry-weighted aggregate) tracks Sigma(notional/entry_price) per side.
        // This enables O(1) estimate PnL calculation for the entire side during ADL checks,
        // without iterating over every position.
        // floor rounding on entry_wt, conservative (slightly understates aggregate weight).
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
        storage::set_market_data(e, self.market_id, &self.data);
        storage::set_total_notional(e, self.total_notional);
    }
}

#[cfg(test)]
mod tests {
    use crate::constants::SCALAR_7;
    use crate::testutils::{default_config, default_market, default_market_data, FEED_BTC};
    use crate::types::MarketData;
    use super::Context;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    fn test_ctx(e: &Env, vault_balance: i128, market_data: MarketData, total_notional: i128) -> Context {
        Context {
            market_id: FEED_BTC,
            feed_id: FEED_BTC,
            price: 0,
            price_scalar: SCALAR_7,
            publish_time: 0,
            config: default_market(e),
            data: market_data,
            trading_config: default_config(),
            vault: Address::generate(e),
            vault_balance,
            token: Address::generate(e),
            treasury: Address::generate(e),
            total_notional,
        }
    }

    #[test]
    fn test_util_within_caps() {
        let e = Env::default();
        // vault=100k, market notional=100k (1x), global=100k (1x)
        // max_util_market=5x, max_util_global=10x → both within caps
        let mut data = default_market_data();
        data.l_notional = 50_000 * SCALAR_7;
        data.s_notional = 50_000 * SCALAR_7;
        let ctx = test_ctx(&e, 100_000 * SCALAR_7, data, 100_000 * SCALAR_7);
        ctx.require_within_util(&e);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #751)")]
    fn test_util_market_exceeds_cap() {
        let e = Env::default();
        // vault=100k, market notional=600k (6x) > max_util_market(5x)
        let mut data = default_market_data();
        data.l_notional = 300_000 * SCALAR_7;
        data.s_notional = 300_000 * SCALAR_7;
        let ctx = test_ctx(&e, 100_000 * SCALAR_7, data, 600_000 * SCALAR_7);
        ctx.require_within_util(&e);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #751)")]
    fn test_util_global_exceeds_cap() {
        let e = Env::default();
        // vault=100k, market notional=100k (1x, within 5x market cap)
        // but global notional=1_100k (11x) > max_util_global(10x)
        let mut data = default_market_data();
        data.l_notional = 50_000 * SCALAR_7;
        data.s_notional = 50_000 * SCALAR_7;
        let ctx = test_ctx(&e, 100_000 * SCALAR_7, data, 1_100_000 * SCALAR_7);
        ctx.require_within_util(&e);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #751)")]
    fn test_util_zero_vault_balance() {
        let e = Env::default();
        let ctx = test_ctx(&e, 0, default_market_data(), 0);
        ctx.require_within_util(&e);
    }
}

