use crate::constants::{MAINTENANCE_MARGIN_DIVISOR, MAX_POSITIONS, SCALAR_7, SCALAR_18};
use crate::errors::TradingError;
use crate::storage;
use crate::types::{MarketConfig, MarketData, TradingConfig};
pub(crate) use crate::types::Position;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{panic_with_error, Address, Env};

// ── Result structs ──────────────────────────────────────────────────

pub struct FeeBreakdown {
    pub base_fee: i128,
    pub impact_fee: i128,
    pub funding: i128,
    pub vault_skim: i128,
}

impl FeeBreakdown {
    pub fn total_fee(&self) -> i128 {
        self.base_fee + self.impact_fee + self.funding
    }
}

pub struct FillResult {
    pub is_dominant: bool,
    pub fee_dominant: i128,
    pub fee_non_dominant: i128,
    pub price_impact_fee: i128,
}

pub struct CloseResult {
    pub pnl: i128,
    pub fees: FeeBreakdown,
    pub user_payout: i128,
    pub vault_transfer: i128,
}

pub struct LiquidationCheck {
    pub is_liquidatable: bool,
    pub pnl: i128,
    pub fees: FeeBreakdown,
}

// ── Position methods ────────────────────────────────────────────────

impl Position {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        e: &Env,
        user: Address,
        feed_id: u32,
        is_long: bool,
        entry_price: i128,
        collateral: i128,
        notional_size: i128,
        stop_loss: i128,
        take_profit: i128,
    ) -> (u32, Self) {
        if notional_size <= 0 || entry_price <= 0 || take_profit < 0 || stop_loss < 0 {
            panic_with_error!(e, TradingError::NegativeValueNotAllowed);
        }
        let positions = storage::get_user_positions(e, &user);
        if positions.len() >= MAX_POSITIONS {
            panic_with_error!(e, TradingError::MaxPositionsReached);
        }

        let id = storage::next_position_id(e);
        let position = Position {
            user,
            filled: false,
            feed_id,
            is_long,
            stop_loss,
            take_profit,
            entry_price,
            collateral,
            notional_size,
            entry_funding_index: 0,
            created_at: e.ledger().timestamp(),
            entry_adl_index: 0,
        };
        storage::add_user_position(e, &position.user, id);
        (id, position)
    }

    /// Returns the ADL-adjusted notional size.
    pub fn effective_notional(&self, e: &Env, data: &MarketData) -> i128 {
        let current_index = if self.is_long {
            data.long_adl_index
        } else {
            data.short_adl_index
        };

        if self.entry_adl_index != current_index && self.entry_adl_index != 0 {
            self.notional_size
                .fixed_mul_floor(e, &current_index, &self.entry_adl_index)
        } else {
            self.notional_size
        }
    }

    /// Transition pending → filled. Snapshots funding/ADL indices, computes fill fees.
    /// Caller must set `self.entry_price` beforehand if the fill price differs from creation.
    /// Caller is responsible for `data.update_stats()` afterward.
    pub fn fill(
        &mut self,
        e: &Env,
        data: &MarketData,
        market_config: &MarketConfig,
        config: &TradingConfig,
    ) -> FillResult {
        self.filled = true;
        self.created_at = e.ledger().timestamp();
        self.entry_funding_index = if self.is_long {
            data.long_funding_index
        } else {
            data.short_funding_index
        };
        self.entry_adl_index = if self.is_long {
            data.long_adl_index
        } else {
            data.short_adl_index
        };

        let is_dominant = if self.is_long {
            data.long_notional_size + self.notional_size > data.short_notional_size
        } else {
            data.short_notional_size + self.notional_size > data.long_notional_size
        };

        let fee_dominant = self
            .notional_size
            .fixed_mul_ceil(e, &config.base_fee_dominant, &SCALAR_7);
        let fee_non_dominant = self
            .notional_size
            .fixed_mul_ceil(e, &config.base_fee_non_dominant, &SCALAR_7);
        let price_impact_fee = self
            .notional_size
            .fixed_div_ceil(e, &market_config.price_impact_scalar, &SCALAR_7);

        FillResult {
            is_dominant,
            fee_dominant,
            fee_non_dominant,
            price_impact_fee,
        }
    }

    /// Compute settlement for closing this position. Pure calculation — no side effects.
    /// Caller must ADL-adjust `self.notional_size` via `effective_notional()` beforehand.
    pub fn close(
        &self,
        e: &Env,
        data: &MarketData,
        market_config: &MarketConfig,
        config: &TradingConfig,
        current_price: i128,
        price_scalar: i128,
    ) -> CloseResult {
        let pnl = self.calculate_pnl(e, current_price, price_scalar);
        let fees = self.calculate_fee_breakdown(e, data, market_config, config);

        let equity = self.collateral + pnl - fees.total_fee();
        let max_payout = self
            .collateral
            .fixed_mul_floor(e, &config.max_payout, &SCALAR_7);
        let user_payout = equity.max(0).min(max_payout);
        let vault_transfer = self.collateral - user_payout;

        CloseResult {
            pnl,
            fees,
            user_payout,
            vault_transfer,
        }
    }

    /// Check whether this position is liquidatable at the given price.
    /// Returns pre-computed fees so the caller can use them for caller_fee without recomputing.
    pub fn check_liquidation(
        &self,
        e: &Env,
        data: &MarketData,
        market_config: &MarketConfig,
        config: &TradingConfig,
        current_price: i128,
        price_scalar: i128,
    ) -> LiquidationCheck {
        let pnl = self.calculate_pnl(e, current_price, price_scalar);
        let fees = self.calculate_fee_breakdown(e, data, market_config, config);
        let equity = self.collateral + pnl - fees.total_fee();
        let maintenance_margin = SCALAR_7 / MAINTENANCE_MARGIN_DIVISOR;
        let required_margin = self
            .notional_size
            .fixed_mul_floor(e, &maintenance_margin, &SCALAR_7);

        LiquidationCheck {
            is_liquidatable: equity < required_margin,
            pnl,
            fees,
        }
    }

    pub fn calculate_fee_breakdown(&self, e: &Env, data: &MarketData, market_config: &MarketConfig, config: &TradingConfig) -> FeeBreakdown {
        let same_side_notional = if self.is_long {
            data.long_notional_size
        } else {
            data.short_notional_size
        };
        let other_side_notional = if self.is_long {
            data.short_notional_size
        } else {
            data.long_notional_size
        };

        let base_fee = if same_side_notional >= other_side_notional {
            self.notional_size
                .fixed_mul_ceil(e, &config.base_fee_dominant, &SCALAR_7)
        } else {
            self.notional_size
                .fixed_mul_ceil(e, &config.base_fee_non_dominant, &SCALAR_7)
        };

        let impact_fee = self
            .notional_size
            .fixed_div_ceil(e, &market_config.price_impact_scalar, &SCALAR_7);

        let funding_index = if self.is_long {
            data.long_funding_index
        } else {
            data.short_funding_index
        };
        let funding = self
            .notional_size
            .fixed_mul_floor(e, &(funding_index - self.entry_funding_index), &SCALAR_18);

        // Vault skim: when position received funding (negative), vault takes a cut
        let vault_skim = if funding < 0 {
            (-funding).fixed_mul_ceil(e, &config.vault_skim, &SCALAR_7)
        } else {
            0
        };

        FeeBreakdown {
            base_fee,
            impact_fee,
            funding,
            vault_skim,
        }
    }

    pub fn calculate_pnl(&self, e: &Env, current_price: i128, price_scalar: i128) -> i128 {
        let price_diff = if self.is_long {
            current_price - self.entry_price
        } else {
            self.entry_price - current_price
        };

        if price_diff == 0 {
            0
        } else {
            let price_change_ratio =
                price_diff.fixed_div_floor(e, &self.entry_price, &price_scalar);
            self.notional_size
                .fixed_mul_floor(e, &price_change_ratio, &price_scalar)
        }
    }

    pub fn check_take_profit(&self, current_price: i128) -> bool {
        if self.take_profit == 0 {
            return false;
        }

        if self.is_long {
            current_price >= self.take_profit
        } else {
            current_price <= self.take_profit
        }
    }

    pub fn check_stop_loss(&self, current_price: i128) -> bool {
        if self.stop_loss == 0 {
            return false;
        }

        if self.is_long {
            current_price <= self.stop_loss
        } else {
            current_price >= self.stop_loss
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{SCALAR_7, SCALAR_18};
    use crate::testutils::{create_trading, default_config, default_market, default_market_data, BTC_FEED_ID};
    use soroban_sdk::{testutils::Address as _, Address, Env};

    fn create_test_position(e: &Env) -> Position {
        Position {
            user: Address::generate(e),
            filled: true,
            feed_id: 1,
            is_long: true,
            stop_loss: 0,
            take_profit: 0,
            entry_price: 100_000 * SCALAR_7, // $100,000
            collateral: 1_000 * SCALAR_7,    // $1,000
            notional_size: 10_000 * SCALAR_7, // $10,000 (10x leverage)
            entry_funding_index: 0,
            created_at: 0,
            entry_adl_index: SCALAR_18,
        }
    }

    // ==========================================
    // PnL Calculation Tests
    // ==========================================

    #[test]
    fn test_calculate_pnl_long_profit() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let position = create_test_position(&e);

        e.as_contract(&address, || {

            // Entry: $100,000, Current: $110,000 (+10%)
            let current_price = 110_000 * SCALAR_7;
            let pnl = position.calculate_pnl(&e, current_price, SCALAR_7);

            // 10% gain on $10,000 notional = $1,000 profit
            let expected_pnl = 1_000 * SCALAR_7;
            assert_eq!(pnl, expected_pnl);
        });
    }

    #[test]
    fn test_calculate_pnl_long_loss() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let position = create_test_position(&e);

        e.as_contract(&address, || {

            // Entry: $100,000, Current: $90,000 (-10%)
            let current_price = 90_000 * SCALAR_7;
            let pnl = position.calculate_pnl(&e, current_price, SCALAR_7);

            // 10% loss on $10,000 notional = -$1,000
            let expected_pnl = -1_000 * SCALAR_7;
            assert_eq!(pnl, expected_pnl);
        });
    }

    #[test]
    fn test_calculate_pnl_short_profit() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut position = create_test_position(&e);
        position.is_long = false;

        e.as_contract(&address, || {

            // Entry: $100,000, Current: $90,000 (-10%, profit for short)
            let current_price = 90_000 * SCALAR_7;
            let pnl = position.calculate_pnl(&e, current_price, SCALAR_7);

            // 10% drop = profit for short
            let expected_pnl = 1_000 * SCALAR_7;
            assert_eq!(pnl, expected_pnl);
        });
    }

    #[test]
    fn test_calculate_pnl_short_loss() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let mut position = create_test_position(&e);
        position.is_long = false;

        e.as_contract(&address, || {

            // Entry: $100,000, Current: $110,000 (+10%, loss for short)
            let current_price = 110_000 * SCALAR_7;
            let pnl = position.calculate_pnl(&e, current_price, SCALAR_7);

            // 10% rise = loss for short
            let expected_pnl = -1_000 * SCALAR_7;
            assert_eq!(pnl, expected_pnl);
        });
    }

    #[test]
    fn test_calculate_pnl_no_change() {
        let e = Env::default();
        let position = create_test_position(&e);

        // Price unchanged - pnl is 0, no storage access needed
        let current_price = 100_000 * SCALAR_7;
        let pnl = position.calculate_pnl(&e, current_price, SCALAR_7);

        assert_eq!(pnl, 0);
    }

    // ==========================================
    // Fee Breakdown Tests
    // ==========================================

    #[test]
    fn test_fee_balanced() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let position = create_test_position(&e);
        let market_config = default_market(&e);
        let mut data = default_market_data();
        data.long_notional_size = 100_000 * SCALAR_7;
        data.short_notional_size = 100_000 * SCALAR_7;

        e.as_contract(&address, || {
            let fees = position.calculate_fee_breakdown(&e, &data, &market_config, &default_config());

            // Both sides pay base fee when balanced
            assert_eq!(fees.base_fee, 5 * SCALAR_7);
            assert!(fees.impact_fee > 0);
            assert_eq!(fees.funding, 0);
        });
    }

    #[test]
    fn test_fee_long_dominant() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let market_config = default_market(&e);
        let mut data = default_market_data();
        data.long_notional_size = 200_000 * SCALAR_7;
        data.short_notional_size = 100_000 * SCALAR_7;

        e.as_contract(&address, || {
            // Long pays dominant base fee (0.05% of 10000 = 5)
            let long_pos = create_test_position(&e);
            let long_fees = long_pos.calculate_fee_breakdown(&e, &data, &market_config, &default_config());
            assert_eq!(long_fees.base_fee, 5 * SCALAR_7);

            // Short pays non-dominant base fee (0.01% of 10000 = 1)
            let mut short_pos = create_test_position(&e);
            short_pos.is_long = false;
            let short_fees = short_pos.calculate_fee_breakdown(&e, &data, &market_config, &default_config());
            assert_eq!(short_fees.base_fee, 1 * SCALAR_7);
        });
    }

    #[test]
    fn test_fee_short_dominant() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let market_config = default_market(&e);
        let mut data = default_market_data();
        data.long_notional_size = 100_000 * SCALAR_7;
        data.short_notional_size = 200_000 * SCALAR_7;

        e.as_contract(&address, || {
            // Long pays non-dominant base fee (0.01% of 10000 = 1)
            let long_pos = create_test_position(&e);
            let long_fees = long_pos.calculate_fee_breakdown(&e, &data, &market_config, &default_config());
            assert_eq!(long_fees.base_fee, 1 * SCALAR_7);

            // Short pays dominant base fee (0.05% of 10000 = 5)
            let mut short_pos = create_test_position(&e);
            short_pos.is_long = false;
            let short_fees = short_pos.calculate_fee_breakdown(&e, &data, &market_config, &default_config());
            assert_eq!(short_fees.base_fee, 5 * SCALAR_7);
        });
    }

    #[test]
    fn test_fee_with_funding() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let market_config = default_market(&e);
        let mut position = create_test_position(&e);
        position.entry_funding_index = 0;

        let mut data = default_market_data();
        data.long_funding_index = SCALAR_18 / 100; // 1% funding

        e.as_contract(&address, || {
            let fees = position.calculate_fee_breakdown(&e, &data, &market_config, &default_config());

            // Funding = notional * (current_index - entry_index)
            // = 10000 * 0.01 = 100 tokens
            let expected_funding = 100 * SCALAR_7;
            assert_eq!(fees.funding, expected_funding);
        });
    }

    #[test]
    fn test_fee_total() {
        let e = Env::default();
        let (address, _) = create_trading(&e);
        let market_config = default_market(&e);
        let position = create_test_position(&e);
        let mut data = default_market_data();
        data.long_notional_size = 100_000 * SCALAR_7;
        data.short_notional_size = 100_000 * SCALAR_7;
        data.long_funding_index = SCALAR_18 / 100;

        e.as_contract(&address, || {
            let fees = position.calculate_fee_breakdown(&e, &data, &market_config, &default_config());
            let total = fees.total_fee();

            assert_eq!(total, fees.base_fee + fees.impact_fee + fees.funding);
        });
    }

    // ==========================================
    // Take Profit Tests
    // ==========================================

    #[test]
    fn test_take_profit_long_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.take_profit = 110_000 * SCALAR_7;

        // Price at or above TP
        assert!(position.check_take_profit(110_000 * SCALAR_7));
        assert!(position.check_take_profit(115_000 * SCALAR_7));
    }

    #[test]
    fn test_take_profit_long_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.take_profit = 110_000 * SCALAR_7;

        // Price below TP
        assert!(!position.check_take_profit(109_999 * SCALAR_7));
    }

    #[test]
    fn test_take_profit_short_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;
        position.take_profit = 90_000 * SCALAR_7;

        // For short, TP is below entry
        assert!(position.check_take_profit(90_000 * SCALAR_7));
        assert!(position.check_take_profit(85_000 * SCALAR_7));
    }

    #[test]
    fn test_take_profit_short_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;
        position.take_profit = 90_000 * SCALAR_7;

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
        position.stop_loss = 95_000 * SCALAR_7;

        // Price at or below SL
        assert!(position.check_stop_loss(95_000 * SCALAR_7));
        assert!(position.check_stop_loss(90_000 * SCALAR_7));
    }

    #[test]
    fn test_stop_loss_long_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.stop_loss = 95_000 * SCALAR_7;

        // Price above SL
        assert!(!position.check_stop_loss(95_001 * SCALAR_7));
    }

    #[test]
    fn test_stop_loss_short_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;
        position.stop_loss = 105_000 * SCALAR_7;

        // For short, SL is above entry
        assert!(position.check_stop_loss(105_000 * SCALAR_7));
        assert!(position.check_stop_loss(110_000 * SCALAR_7));
    }

    #[test]
    fn test_stop_loss_short_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;
        position.stop_loss = 105_000 * SCALAR_7;

        // Price below SL (not reached for short)
        assert!(!position.check_stop_loss(104_999 * SCALAR_7));
    }

    #[test]
    fn test_stop_loss_not_set() {
        let e = Env::default();
        let position = create_test_position(&e);

        // SL = 0 means not set
        assert!(!position.check_stop_loss(1 * SCALAR_7));
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
                user.clone(),
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
            assert_eq!(position.feed_id, BTC_FEED_ID);
            assert!(position.is_long);
            assert_eq!(position.stop_loss, 90_000 * SCALAR_7);
            assert_eq!(position.take_profit, 110_000 * SCALAR_7);
            assert_eq!(position.entry_price, 100_000 * SCALAR_7);
            assert_eq!(position.collateral, 1_000 * SCALAR_7);
            assert_eq!(position.notional_size, 10_000 * SCALAR_7);
            assert_eq!(position.entry_funding_index, 0);
            assert_eq!(position.entry_adl_index, 0);
            assert_eq!(position.created_at, 1000);
        });
    }

    #[test]
    fn test_position_load_and_store() {
        use crate::testutils::{create_trading, jump};

        let e = Env::default();
        jump(&e, 1000);

        let (address, _) = create_trading(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            let (id, position) = Position::create(
                &e,
                user.clone(),
                BTC_FEED_ID,
                true,
                100_000 * SCALAR_7,
                1_000 * SCALAR_7,
                10_000 * SCALAR_7,
                90_000 * SCALAR_7,
                110_000 * SCALAR_7,
            );
            storage::set_position(&e, id, &position);

            let loaded = storage::get_position(&e, id);
            assert_eq!(loaded.user, user);
            assert_eq!(loaded.entry_price, 100_000 * SCALAR_7);
        });
    }
}
