use crate::constants::{MIN_OPEN_TIME, SCALAR_7, SCALAR_18};
use crate::errors::TradingError;
use crate::storage;
use crate::trading::market::Market;
use crate::types::MarketData;
pub(crate) use crate::types::Position;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{panic_with_error, Address, Env};
// ── Result structs ──────────────────────────────────────────────────

/// Breakdown of PnL and all fee components for a closed/settled position.
///
/// All amounts are in token_decimals. PnL and funding can be negative (loss/payment).
/// Fees (base, impact, borrowing) are always >= 0.
pub struct Settlement {
    /// Raw price PnL: (exit_price - entry_price) / entry_price * notional.
    /// Positive = profit, negative = loss. Does not include fees.
    pub pnl: i128,
    /// Base trading fee charged at close (notional * fee_rate, SCALAR_7 fraction).
    pub base_fee: i128,
    /// Price-impact fee (notional / impact_divisor, SCALAR_7 fraction).
    pub impact_fee: i128,
    /// Accrued funding since position open. Positive = owed (dominant side), negative = earned.
    pub funding: i128,
    /// Accrued borrowing fee since position open. Always >= 0.
    pub borrowing_fee: i128,
}

impl Settlement {
    pub fn equity(&self, col: i128) -> i128 {
        col + self.pnl - self.total_fee()
    }

    pub fn total_fee(&self) -> i128 {
        self.base_fee + self.impact_fee + self.funding + self.borrowing_fee
    }

    /// Net PnL: raw PnL minus all fees, clamped to `-col` (user can never lose more than collateral).
    /// WHY: clamped to protect against fee accumulation exceeding collateral, which would
    /// cause an underflow. The vault absorbs the shortfall in that case.
    pub fn net_pnl(&self, col: i128) -> i128 {
        (self.pnl - self.total_fee()).max(-col)
    }

    /// Trading fees only (base + impact).
    pub fn trading_fee(&self) -> i128 {
        self.base_fee + self.impact_fee
    }

    /// Protocol revenue: trading fees + borrowing. Excludes funding (P2P).
    /// Treasury gets a cut of this. Caller only gets a cut of trading_fee.
    pub fn protocol_fee(&self) -> i128 {
        self.base_fee + self.impact_fee + self.borrowing_fee
    }
}


// ── Position methods ────────────────────────────────────────────────

impl Position {
    /// Create a new position, persist it, and register it under the user.
    /// Returns (position_id, position).
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        e: &Env,
        user: &Address,
        feed: u32,
        long: bool,
        entry_price: i128,
        col: i128,
        notional: i128,
        sl: i128,
        tp: i128,
    ) -> (u32, Self) {
        let position = Position {
            user: user.clone(),
            filled: false,
            feed,
            long,
            sl,
            tp,
            entry_price,
            col,
            notional,
            fund_idx: 0,
            borr_idx: 0,
            created_at: e.ledger().timestamp(),
            adl_idx: SCALAR_18,
        };
        let id = storage::next_position_id(e);
        storage::add_user_position(e, user, id);
        (id, position)
    }

    /// Validate position parameters against trading and market constraints.
    ///
    /// # Parameters
    /// - `enabled` - Whether the market is accepting new positions
    /// - `min_notional` / `max_notional` - Notional bounds (token_decimals)
    /// - `margin` - Initial margin requirement (SCALAR_7, e.g. 1e6 = 10% = 10x max leverage)
    ///
    /// # Panics
    /// - `TradingError::NegativeValueNotAllowed` (735) if notional, price, or col <= 0
    /// - `TradingError::MarketDisabled` (712) if market is not enabled
    /// - `TradingError::NotionalBelowMinimum` (736) / `NotionalAboveMaximum` (737)
    /// - `TradingError::LeverageAboveMaximum` (739) if `notional * margin > col`
    ///
    /// WHY: `margin` constraint ensures `leverage = notional / col <= 1 / margin`.
    /// Margin check uses ceil rounding to prevent users from sneaking above max leverage
    /// via rounding.
    pub fn validate(&self, e: &Env, enabled: bool, min_notional: i128, max_notional: i128, margin: i128) {
        if self.notional <= 0 || self.entry_price <= 0 || self.col <= 0 || self.tp < 0 || self.sl < 0 {
            panic_with_error!(e, TradingError::NegativeValueNotAllowed);
        }
        if !enabled {
            panic_with_error!(e, TradingError::MarketDisabled);
        }
        if self.notional < min_notional {
            panic_with_error!(e, TradingError::NotionalBelowMinimum);
        }
        if self.notional > max_notional {
            panic_with_error!(e, TradingError::NotionalAboveMaximum);
        }
        if self.notional.fixed_mul_ceil(e, &margin, &SCALAR_7) > self.col {
            panic_with_error!(e, TradingError::LeverageAboveMaximum);
        }
    }

    /// Guard for user-initiated close: position must be filled and at least MIN_OPEN_TIME old.
    ///
    /// # Panics
    /// - `TradingError::ActionNotAllowedForStatus` (750) if position is not filled
    /// - `TradingError::PositionTooNew` (748) if < 30s since fill
    ///
    /// WHY: MIN_OPEN_TIME (30 seconds) prevents same-block open+close price arbitrage.
    /// Without it, a user could open and close in the same ledger using the same oracle
    /// price, extracting risk-free profit from fee asymmetry.
    pub fn require_closable(&self, e: &Env) {
        if !self.filled {
            panic_with_error!(e, TradingError::ActionNotAllowedForStatus);
        }
        let earliest_close = self.created_at.saturating_add(MIN_OPEN_TIME);
        if e.ledger().timestamp() < earliest_close {
            panic_with_error!(e, TradingError::PositionTooNew);
        }
    }

    /// Guard for liquidation path: position must be filled, and price must be
    /// at least as recent as the position open time. This prevents stale-price
    /// liquidation attacks without blocking timely liquidations with MIN_OPEN_TIME.
    pub fn require_liquidatable(&self, e: &Env, price_publish_time: u64) {
        if !self.filled {
            panic_with_error!(e, TradingError::ActionNotAllowedForStatus);
        }
        // Guard: price must be at least as recent as position open.
        // Prevents manipulation with stale prices predating the position.
        // Uses StalePrice (749) not PositionTooNew (748) -- semantically distinct:
        // PositionTooNew = "opened too recently to close", StalePrice = "price data is too old"
        if price_publish_time < self.created_at {
            panic_with_error!(e, TradingError::StalePrice);
        }
    }

    pub fn validate_triggers(&self, e: &Env) {
        if self.tp < 0 || self.sl < 0 {
            panic_with_error!(e, TradingError::NegativeValueNotAllowed);
        }
        if self.tp > 0 {
            if self.long && self.tp <= self.entry_price {
                panic_with_error!(e, TradingError::InvalidTakeProfitPrice);
            }
            if !self.long && self.tp >= self.entry_price {
                panic_with_error!(e, TradingError::InvalidTakeProfitPrice);
            }
        }
        if self.sl > 0 {
            if self.long && self.sl >= self.entry_price {
                panic_with_error!(e, TradingError::InvalidStopLossPrice);
            }
            if !self.long && self.sl <= self.entry_price {
                panic_with_error!(e, TradingError::InvalidStopLossPrice);
            }
        }
    }

    /// Transition pending → filled. Snapshots funding/borrowing/ADL indices.
    pub fn fill(&mut self, e: &Env, data: &MarketData) {
        self.filled = true;
        self.created_at = e.ledger().timestamp();
        let (fi, bi, ai) = data.indices(self.long);
        self.fund_idx = fi;
        self.borr_idx = bi;
        self.adl_idx = ai;
    }

    /// Settle a position: compute PnL and all accrued fees using index-based accounting.
    ///
    /// # Index-based settlement formula
    /// ```text
    /// funding  = notional × (current_fund_idx - position.fund_idx) / SCALAR_18
    /// borrowing = notional × (current_borr_idx - position.borr_idx) / SCALAR_18
    /// pnl      = notional × (exit_price - entry_price) / entry_price
    /// ```
    ///
    /// The position snapshots funding/borrowing/ADL indices at fill time. At settlement,
    /// the difference between current and snapshotted index represents the per-unit
    /// accrued rate, multiplied by notional to get the total amount.
    ///
    /// # ADL adjustment
    /// If the ADL index has changed since fill, the position's notional is reduced
    /// proportionally before any other calculation. This ensures the position's
    /// exposure reflects the deleveraging that occurred.
    ///
    /// # Returns
    /// [`Settlement`] with all components broken out.
    pub fn settle(&mut self, e: &Env, market: &Market) -> Settlement {
        let (funding_index, borrowing_index, adl_index) = market.data.indices(self.long);

        // Apply ADL: scale down notional by the ratio of current/original ADL index.
        // WHY: floor rounding on ADL reduction -- conservative for the position holder
        // (slightly less notional = slightly less exposure = safer for the vault).
        if self.adl_idx != adl_index {
            self.notional = self.notional.fixed_mul_floor(e, &adl_index, &self.adl_idx);
            self.adl_idx = adl_index;
        }

        // PnL: floor rounding -- conservative for the trader (vault keeps rounding dust).
        let price_diff = if self.long {
            market.price - self.entry_price
        } else {
            self.entry_price - market.price
        };
        let pnl = if price_diff == 0 {
            0
        } else {
            let ratio = price_diff.fixed_div_floor(e, &self.entry_price, &market.price_scalar);
            self.notional.fixed_mul_floor(e, &ratio, &market.price_scalar)
        };

        // WHY: Closing from the dominant side rebalances the market (reduces imbalance),
        // so it gets the lower non-dom fee. Closing from non-dominant side worsens
        // imbalance, so it pays the higher dom fee. This mirrors open-side fee logic.
        let base_fee = if market.data.is_dominant(self.long, -self.notional) {
            self.notional.fixed_mul_ceil(e, &market.trading_config.fee_non_dom, &SCALAR_7)
        } else {
            self.notional.fixed_mul_ceil(e, &market.trading_config.fee_dom, &SCALAR_7)
        };
        let impact_fee = self.notional.fixed_div_ceil(e, &market.config.impact, &SCALAR_7);

        // WHY: funding uses floor (conservative for receiver), borrowing uses ceil
        // (protocol never under-collects).
        let funding = self.notional.fixed_mul_floor(e, &(funding_index - self.fund_idx), &SCALAR_18);
        let borrowing_fee = self.notional.fixed_mul_ceil(e, &(borrowing_index - self.borr_idx), &SCALAR_18);

        Settlement {
            pnl,
            base_fee,
            impact_fee,
            funding,
            borrowing_fee,
        }
    }

    pub fn check_take_profit(&self, current_price: i128) -> bool {
        if self.tp == 0 {
            return false;
        }

        if self.long {
            current_price >= self.tp
        } else {
            current_price <= self.tp
        }
    }

    pub fn check_stop_loss(&self, current_price: i128) -> bool {
        if self.sl == 0 {
            return false;
        }

        if self.long {
            current_price <= self.sl
        } else {
            current_price >= self.sl
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{SCALAR_7, SCALAR_18};
    use crate::trading::market::Market;
    use crate::testutils::{create_trading, default_config, default_market, default_market_data, BTC_FEED_ID};
    use soroban_sdk::{testutils::Address as _, Address, Env};

    fn create_test_position(e: &Env) -> Position {
        Position {
            user: Address::generate(e),
            filled: true,
            feed: 1,
            long: true,
            sl: 0,
            tp: 0,
            entry_price: 100_000 * SCALAR_7, // $100,000
            col: 1_000 * SCALAR_7,    // $1,000
            notional: 10_000 * SCALAR_7, // $10,000 (10x leverage)
            fund_idx: 0,
            borr_idx: 0,
            created_at: 0,
            adl_idx: SCALAR_18,
        }
    }

    fn test_market(data: MarketData) -> Market {
        let e = Env::default();
        Market {
            feed_id: BTC_FEED_ID,
            price: 100_000 * SCALAR_7,
            price_scalar: SCALAR_7,
            config: default_market(&e),
            data,
            trading_config: default_config(),
            vault: Address::generate(&e),
            vault_balance: 1_000_000 * SCALAR_7,
            token: Address::generate(&e),
            treasury: Address::generate(&e),
            total_notional: 0,
            publish_time: 0,
        }
    }

    fn test_market_at(price: i128, data: MarketData) -> Market {
        let e = Env::default();
        Market {
            feed_id: BTC_FEED_ID,
            price,
            price_scalar: SCALAR_7,
            config: default_market(&e),
            data,
            trading_config: default_config(),
            vault: Address::generate(&e),
            vault_balance: 1_000_000 * SCALAR_7,
            token: Address::generate(&e),
            treasury: Address::generate(&e),
            total_notional: 0,
            publish_time: 0,
        }
    }

    // ==========================================
    // Settlement Tests (PnL + Fees)
    // ==========================================

    #[test]
    fn test_settle_long_profit() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut position = create_test_position(&e);
        let m = test_market_at(110_000 * SCALAR_7, default_market_data());

        e.as_contract(&address, || {
            let s = position.settle(&e, &m);
            // 10% gain on $10,000 notional = $1,000 profit
            assert_eq!(s.pnl, 1_000 * SCALAR_7);
        });
    }

    #[test]
    fn test_settle_long_loss() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut position = create_test_position(&e);
        let m = test_market_at(90_000 * SCALAR_7, default_market_data());

        e.as_contract(&address, || {
            let s = position.settle(&e, &m);
            assert_eq!(s.pnl, -1_000 * SCALAR_7);
        });
    }

    #[test]
    fn test_settle_short_profit() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut position = create_test_position(&e);
        position.long = false;
        let m = test_market_at(90_000 * SCALAR_7, default_market_data());

        e.as_contract(&address, || {
            let s = position.settle(&e, &m);
            assert_eq!(s.pnl, 1_000 * SCALAR_7);
        });
    }

    #[test]
    fn test_settle_short_loss() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut position = create_test_position(&e);
        position.long = false;
        let m = test_market_at(110_000 * SCALAR_7, default_market_data());

        e.as_contract(&address, || {
            let s = position.settle(&e, &m);
            assert_eq!(s.pnl, -1_000 * SCALAR_7);
        });
    }

    #[test]
    fn test_settle_no_pnl() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut position = create_test_position(&e);
        let m = test_market(default_market_data());

        e.as_contract(&address, || {
            let s = position.settle(&e, &m);
            assert_eq!(s.pnl, 0);
        });
    }

    #[test]
    fn test_settle_fee_balanced() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut position = create_test_position(&e);
        let mut data = default_market_data();
        data.l_notional = 100_000 * SCALAR_7;
        data.s_notional = 100_000 * SCALAR_7;
        let m = test_market(data);

        e.as_contract(&address, || {
            let s = position.settle(&e, &m);
            // Balanced: closing either side makes the other dominant → dom fee
            assert_eq!(s.base_fee, 5 * SCALAR_7);
            assert!(s.impact_fee > 0);
            assert_eq!(s.funding, 0);
        });
    }

    #[test]
    fn test_settle_fee_long_dominant() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut data = default_market_data();
        data.l_notional = 200_000 * SCALAR_7;
        data.s_notional = 100_000 * SCALAR_7;
        let m = test_market(data);

        e.as_contract(&address, || {
            // Long closing from dominant side → rebalances → non-dom fee
            let mut long_pos = create_test_position(&e);
            let s = long_pos.settle(&e, &m);
            assert_eq!(s.base_fee, SCALAR_7);

            // Short closing from non-dominant side → worsens imbalance → dom fee
            let mut short_pos = create_test_position(&e);
            short_pos.long = false;
            let s = short_pos.settle(&e, &m);
            assert_eq!(s.base_fee, 5 * SCALAR_7);
        });
    }

    #[test]
    fn test_settle_fee_short_dominant() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut data = default_market_data();
        data.l_notional = 100_000 * SCALAR_7;
        data.s_notional = 200_000 * SCALAR_7;
        let m = test_market(data);

        e.as_contract(&address, || {
            // Long closing from non-dominant side → worsens imbalance → dom fee
            let mut long_pos = create_test_position(&e);
            let s = long_pos.settle(&e, &m);
            assert_eq!(s.base_fee, 5 * SCALAR_7);

            // Short closing from dominant side → rebalances → non-dom fee
            let mut short_pos = create_test_position(&e);
            short_pos.long = false;
            let s = short_pos.settle(&e, &m);
            assert_eq!(s.base_fee, SCALAR_7);
        });
    }

    #[test]
    fn test_settle_fee_with_funding() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut position = create_test_position(&e);
        position.fund_idx = 0;
        let mut data = default_market_data();
        data.l_fund_idx = SCALAR_18 / 100; // 1% funding
        let m = test_market(data);

        e.as_contract(&address, || {
            let s = position.settle(&e, &m);
            // Funding = notional * (current_index - entry_index) = 10000 * 0.01 = 100
            assert_eq!(s.funding, 100 * SCALAR_7);
        });
    }

    #[test]
    fn test_settle_fee_total() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut position = create_test_position(&e);
        let mut data = default_market_data();
        data.l_notional = 100_000 * SCALAR_7;
        data.s_notional = 100_000 * SCALAR_7;
        data.l_fund_idx = SCALAR_18 / 100;
        let m = test_market(data);

        e.as_contract(&address, || {
            let s = position.settle(&e, &m);
            assert_eq!(s.total_fee(), s.base_fee + s.impact_fee + s.funding);
        });
    }

    // ==========================================
    // Take Profit Tests
    // ==========================================

    #[test]
    fn test_take_profit_long_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.tp = 110_000 * SCALAR_7;

        // Price at or above TP
        assert!(position.check_take_profit(110_000 * SCALAR_7));
        assert!(position.check_take_profit(115_000 * SCALAR_7));
    }

    #[test]
    fn test_take_profit_long_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.tp = 110_000 * SCALAR_7;

        // Price below TP
        assert!(!position.check_take_profit(109_999 * SCALAR_7));
    }

    #[test]
    fn test_take_profit_short_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.long = false;
        position.tp = 90_000 * SCALAR_7;

        // For short, TP is below entry
        assert!(position.check_take_profit(90_000 * SCALAR_7));
        assert!(position.check_take_profit(85_000 * SCALAR_7));
    }

    #[test]
    fn test_take_profit_short_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.long = false;
        position.tp = 90_000 * SCALAR_7;

        // Price above TP (not reached for short)
        assert!(!position.check_take_profit(90_001 * SCALAR_7));
    }

    #[test]
    fn test_take_profit_not_set() {
        let e = Env::default();
        let position = create_test_position(&e);

        // TP = 0 means not set
        assert!(!position.check_take_profit(200_000 * SCALAR_7));
    }

    // ==========================================
    // Stop Loss Tests
    // ==========================================

    #[test]
    fn test_stop_loss_long_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.sl = 95_000 * SCALAR_7;

        // Price at or below SL
        assert!(position.check_stop_loss(95_000 * SCALAR_7));
        assert!(position.check_stop_loss(90_000 * SCALAR_7));
    }

    #[test]
    fn test_stop_loss_long_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.sl = 95_000 * SCALAR_7;

        // Price above SL
        assert!(!position.check_stop_loss(95_001 * SCALAR_7));
    }

    #[test]
    fn test_stop_loss_short_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.long = false;
        position.sl = 105_000 * SCALAR_7;

        // For short, SL is above entry
        assert!(position.check_stop_loss(105_000 * SCALAR_7));
        assert!(position.check_stop_loss(110_000 * SCALAR_7));
    }

    #[test]
    fn test_stop_loss_short_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.long = false;
        position.sl = 105_000 * SCALAR_7;

        // Price below SL (not reached for short)
        assert!(!position.check_stop_loss(104_999 * SCALAR_7));
    }

    #[test]
    fn test_stop_loss_not_set() {
        let e = Env::default();
        let position = create_test_position(&e);

        // SL = 0 means not set
        assert!(!position.check_stop_loss(SCALAR_7));
    }

    #[test]
    fn test_position_create() {
        use crate::testutils::{create_trading, jump};

        let e = Env::default();
        jump(&e, 1000);

        let (address, _) = create_trading(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            let (id, position) = Position::create(
                &e,
                &user,
                BTC_FEED_ID,
                true,
                100_000 * SCALAR_7,
                1_000 * SCALAR_7,
                10_000 * SCALAR_7,
                90_000 * SCALAR_7,
                110_000 * SCALAR_7,
            );

            assert_eq!(id, 0);
            assert_eq!(position.user, user);
            assert!(!position.filled);
            assert_eq!(position.feed, BTC_FEED_ID);
            assert!(position.long);
            assert_eq!(position.sl, 90_000 * SCALAR_7);
            assert_eq!(position.tp, 110_000 * SCALAR_7);
            assert_eq!(position.entry_price, 100_000 * SCALAR_7);
            assert_eq!(position.col, 1_000 * SCALAR_7);
            assert_eq!(position.notional, 10_000 * SCALAR_7);
            assert_eq!(position.fund_idx, 0);
            assert_eq!(position.adl_idx, SCALAR_18);
            assert_eq!(position.created_at, 1000);

            // Verify user position tracking (create registers but does not persist position)
            let user_positions = storage::get_user_positions(&e, &user);
            assert_eq!(user_positions.len(), 1);
            assert_eq!(user_positions.get(0), Some(id));
        });
    }

    // ==========================================
    // require_liquidatable Tests
    // ==========================================

    #[test]
    #[should_panic(expected = "Error(Contract, #749)")]
    fn test_require_liquidatable_stale_price_fails() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.created_at = 1000;
        position.filled = true;
        // price publish_time before position open -> StalePrice
        position.require_liquidatable(&e, 999);
    }

    #[test]
    fn test_require_liquidatable_valid_price_succeeds() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.created_at = 1000;
        position.filled = true;
        // Exact match: price at same time as position open
        position.require_liquidatable(&e, 1000);
        // Newer price: should also succeed
        position.require_liquidatable(&e, 1001);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #750)")]
    fn test_require_liquidatable_unfilled_fails() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.filled = false;
        // Even with a valid publish_time, unfilled position should fail
        position.require_liquidatable(&e, 2000);
    }

    #[test]
    fn test_require_liquidatable_immediate_ok() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.created_at = 500;
        position.filled = true;
        // Same timestamp as position: immediately liquidatable
        position.require_liquidatable(&e, 500);
    }
}
