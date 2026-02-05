use crate::constants::{SCALAR_18, SCALAR_7};
use crate::storage;
use crate::trading::market::Market;
pub(crate) use crate::types::Position;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{Address, Env};

/// Breakdown of position fees
/// Used for close calculations and event emission
pub struct FeeBreakdown {
    pub base_fee: i128,
    pub impact_fee: i128,
    pub interest: i128,
}

impl FeeBreakdown {
    pub fn total_fee(&self) -> i128 {
        self.base_fee + self.impact_fee + self.interest
    }
}

/// Implementation of position-related methods
impl Position {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        e: &Env,
        id: u32,
        user: Address,
        filled: bool,
        asset_index: u32,
        is_long: bool,
        stop_loss: i128,
        take_profit: i128,
        entry_price: i128,
        collateral: i128,
        notional_size: i128,
        interest_index: i128,
    ) -> Self {
        Position {
            id,
            user,
            filled,
            asset_index,
            is_long,
            stop_loss,
            take_profit,
            entry_price,
            collateral,
            notional_size,
            interest_index,
            created_at: e.ledger().timestamp(),
        }
    }

    pub fn load(e: &Env, position_id: u32) -> Self {
        storage::get_position(e, position_id)
    }

    pub fn store(&self, e: &Env) {
        storage::set_position(e, self.id, self);
    }

    pub fn calculate_fee_breakdown(&self, e: &Env, market: &Market) -> FeeBreakdown {
        // Pay base fee when closing a position on the dominant side
        // If balanced (both sides equal), both sides pay the base fee
        let same_side_notional = if self.is_long {
            market.data.long_notional_size
        } else {
            market.data.short_notional_size
        };
        let other_side_notional = if self.is_long {
            market.data.short_notional_size
        } else {
            market.data.long_notional_size
        };

        let base_fee = if same_side_notional >= other_side_notional {
            self.notional_size
                .fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7)
        } else {
            0
        };

        let impact_fee = self
            .notional_size
            .fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

        let interest_index = if self.is_long {
            market.data.long_interest_index
        } else {
            market.data.short_interest_index
        };
        let interest = self
            .notional_size
            .fixed_mul_floor(e, &(interest_index - self.interest_index), &SCALAR_18);

        FeeBreakdown {
            base_fee,
            impact_fee,
            interest,
        }
    }

    pub fn equity(&self, e: &Env, price: i128, market: &Market) -> i128 {
        let pnl = self.calculate_pnl(e, price);
        let fees = self.calculate_fee_breakdown(e, market);
        self.collateral + pnl - fees.total_fee()
    }

    pub fn calculate_pnl(&self, e: &Env, current_price: i128) -> i128 {
        let price_diff = if self.is_long {
            current_price - self.entry_price
        } else {
            self.entry_price - current_price
        };

        if price_diff == 0 {
            0
        } else {
            // PnL = notional_size * (price_change / entry_price)
            let price_change_ratio = price_diff.fixed_div_floor(e, &self.entry_price, &SCALAR_7);
            self.notional_size
                .fixed_mul_floor(e, &price_change_ratio, &SCALAR_7)
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
    use crate::types::{MarketConfig, MarketData};
    use sep_40_oracle::Asset;
    use soroban_sdk::{testutils::Address as _, Address, Env, Symbol};

    fn create_test_market(e: &Env) -> Market {
        Market {
            asset_index: 0,
            config: MarketConfig {
                asset: Asset::Other(Symbol::new(e, "BTC")),
                enabled: true,
                max_payout: 10 * SCALAR_7,
                min_collateral: SCALAR_7,
                max_collateral: 1_000_000 * SCALAR_7,
                init_margin: 0_0100000,        // 1%
                maintenance_margin: 0_0050000, // 0.5%
                base_fee: 0_0005000,            // 0.05%
                price_impact_scalar: 8_000_000_000 * SCALAR_7,
                base_hourly_rate: 10_000_000_000_000,
                ratio_cap: 5 * SCALAR_18,
            },
            data: MarketData {
                long_notional_size: 0,
                short_notional_size: 0,
                long_interest_index: 0,
                short_interest_index: 0,
                last_update: 0,
            },
        }
    }

    fn create_test_position(e: &Env) -> Position {
        Position {
            id: 1,
            user: Address::generate(e),
            filled: true,
            asset_index: 0,
            is_long: true,
            stop_loss: 0,
            take_profit: 0,
            entry_price: 100_000 * SCALAR_7, // $100,000
            collateral: 1_000 * SCALAR_7,    // $1,000
            notional_size: 10_000 * SCALAR_7, // $10,000 (10x leverage)
            interest_index: 0,
            created_at: 0,
        }
    }

    // ==========================================
    // PnL Calculation Tests
    // ==========================================

    #[test]
    fn test_calculate_pnl_long_profit() {
        let e = Env::default();
        let position = create_test_position(&e);

        // Entry: $100,000, Current: $110,000 (+10%)
        let current_price = 110_000 * SCALAR_7;
        let pnl = position.calculate_pnl(&e, current_price);

        // 10% gain on $10,000 notional = $1,000 profit
        let expected_pnl = 1_000 * SCALAR_7;
        assert_eq!(pnl, expected_pnl);
    }

    #[test]
    fn test_calculate_pnl_long_loss() {
        let e = Env::default();
        let position = create_test_position(&e);

        // Entry: $100,000, Current: $90,000 (-10%)
        let current_price = 90_000 * SCALAR_7;
        let pnl = position.calculate_pnl(&e, current_price);

        // 10% loss on $10,000 notional = -$1,000
        let expected_pnl = -1_000 * SCALAR_7;
        assert_eq!(pnl, expected_pnl);
    }

    #[test]
    fn test_calculate_pnl_short_profit() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;

        // Entry: $100,000, Current: $90,000 (-10%, profit for short)
        let current_price = 90_000 * SCALAR_7;
        let pnl = position.calculate_pnl(&e, current_price);

        // 10% drop = profit for short
        let expected_pnl = 1_000 * SCALAR_7;
        assert_eq!(pnl, expected_pnl);
    }

    #[test]
    fn test_calculate_pnl_short_loss() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;

        // Entry: $100,000, Current: $110,000 (+10%, loss for short)
        let current_price = 110_000 * SCALAR_7;
        let pnl = position.calculate_pnl(&e, current_price);

        // 10% rise = loss for short
        let expected_pnl = -1_000 * SCALAR_7;
        assert_eq!(pnl, expected_pnl);
    }

    #[test]
    fn test_calculate_pnl_no_change() {
        let e = Env::default();
        let position = create_test_position(&e);

        // Price unchanged
        let current_price = 100_000 * SCALAR_7;
        let pnl = position.calculate_pnl(&e, current_price);

        assert_eq!(pnl, 0);
    }

    #[test]
    fn test_calculate_pnl_small_movement() {
        let e = Env::default();
        let position = create_test_position(&e);

        // 0.1% price increase
        let current_price = 100_100 * SCALAR_7;
        let pnl = position.calculate_pnl(&e, current_price);

        // 0.1% gain on $10,000 = $10
        let expected_pnl = 10 * SCALAR_7;
        assert_eq!(pnl, expected_pnl);
    }

    // ==========================================
    // Fee Breakdown Tests
    // ==========================================

    #[test]
    fn test_fee_breakdown_balanced_market() {
        let e = Env::default();
        let position = create_test_position(&e);
        let mut market = create_test_market(&e);

        // Balanced market: long = short
        market.data.long_notional_size = 100_000 * SCALAR_7;
        market.data.short_notional_size = 100_000 * SCALAR_7;

        let fees = position.calculate_fee_breakdown(&e, &market);

        // Both sides pay base fee when balanced
        // base_fee = notional * 0.0005 = 10000 * 0.0005 = 5 tokens
        let expected_base_fee = 5 * SCALAR_7;
        assert_eq!(fees.base_fee, expected_base_fee);

        // Price impact = notional / price_impact_scalar
        // = 10000 / 8_000_000_000 ≈ 0.00000125 tokens
        assert!(fees.impact_fee > 0);

        // No interest accrued (index = 0)
        assert_eq!(fees.interest, 0);
    }

    #[test]
    fn test_fee_breakdown_long_dominant_long_pays() {
        let e = Env::default();
        let position = create_test_position(&e);
        let mut market = create_test_market(&e);

        // Longs dominant
        market.data.long_notional_size = 200_000 * SCALAR_7;
        market.data.short_notional_size = 100_000 * SCALAR_7;

        let fees = position.calculate_fee_breakdown(&e, &market);

        // Long pays base fee when longs are dominant
        let expected_base_fee = 5 * SCALAR_7;
        assert_eq!(fees.base_fee, expected_base_fee);
    }

    #[test]
    fn test_fee_breakdown_long_dominant_short_doesnt_pay() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;
        let mut market = create_test_market(&e);

        // Longs dominant
        market.data.long_notional_size = 200_000 * SCALAR_7;
        market.data.short_notional_size = 100_000 * SCALAR_7;

        let fees = position.calculate_fee_breakdown(&e, &market);

        // Short doesn't pay base fee when longs are dominant
        assert_eq!(fees.base_fee, 0);
    }

    #[test]
    fn test_fee_breakdown_short_dominant() {
        let e = Env::default();
        let position = create_test_position(&e);
        let mut market = create_test_market(&e);

        // Shorts dominant
        market.data.long_notional_size = 100_000 * SCALAR_7;
        market.data.short_notional_size = 200_000 * SCALAR_7;

        let fees = position.calculate_fee_breakdown(&e, &market);

        // Long doesn't pay base fee when shorts are dominant
        assert_eq!(fees.base_fee, 0);
    }

    #[test]
    fn test_fee_breakdown_with_interest() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.interest_index = 0;

        let mut market = create_test_market(&e);
        // Simulate interest accrual
        market.data.long_interest_index = SCALAR_18 / 100; // 1% interest

        let fees = position.calculate_fee_breakdown(&e, &market);

        // Interest = notional * (current_index - position_index)
        // = 10000 * 0.01 = 100 tokens
        let expected_interest = 100 * SCALAR_7;
        assert_eq!(fees.interest, expected_interest);
    }

    #[test]
    fn test_fee_total() {
        let e = Env::default();
        let position = create_test_position(&e);
        let mut market = create_test_market(&e);
        market.data.long_notional_size = 100_000 * SCALAR_7;
        market.data.short_notional_size = 100_000 * SCALAR_7;
        market.data.long_interest_index = SCALAR_18 / 100;

        let fees = position.calculate_fee_breakdown(&e, &market);
        let total = fees.total_fee();

        assert_eq!(total, fees.base_fee + fees.impact_fee + fees.interest);
    }

    // ==========================================
    // Equity Tests
    // ==========================================

    #[test]
    fn test_equity_profitable_position() {
        let e = Env::default();
        let position = create_test_position(&e);
        let market = create_test_market(&e);

        // 10% profit
        let current_price = 110_000 * SCALAR_7;
        let equity = position.equity(&e, current_price, &market);

        // equity = collateral + pnl - fees
        // = 1000 + 1000 - fees ≈ 2000 - small_fees
        assert!(equity > 1_900 * SCALAR_7);
        assert!(equity < 2_000 * SCALAR_7);
    }

    #[test]
    fn test_equity_losing_position() {
        let e = Env::default();
        let position = create_test_position(&e);
        let market = create_test_market(&e);

        // 10% loss
        let current_price = 90_000 * SCALAR_7;
        let equity = position.equity(&e, current_price, &market);

        // equity = 1000 - 1000 - fees ≈ -fees (negative)
        assert!(equity < 0);
    }

    // ==========================================
    // Take Profit Tests
    // ==========================================

    #[test]
    fn test_check_take_profit_long_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.take_profit = 110_000 * SCALAR_7;

        // Price at or above TP
        assert!(position.check_take_profit(110_000 * SCALAR_7));
        assert!(position.check_take_profit(115_000 * SCALAR_7));
    }

    #[test]
    fn test_check_take_profit_long_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.take_profit = 110_000 * SCALAR_7;

        // Price below TP
        assert!(!position.check_take_profit(109_999 * SCALAR_7));
    }

    #[test]
    fn test_check_take_profit_short_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;
        position.take_profit = 90_000 * SCALAR_7;

        // For short, TP is below entry
        assert!(position.check_take_profit(90_000 * SCALAR_7));
        assert!(position.check_take_profit(85_000 * SCALAR_7));
    }

    #[test]
    fn test_check_take_profit_short_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;
        position.take_profit = 90_000 * SCALAR_7;

        // Price above TP (not reached for short)
        assert!(!position.check_take_profit(90_001 * SCALAR_7));
    }

    #[test]
    fn test_check_take_profit_not_set() {
        let e = Env::default();
        let position = create_test_position(&e);

        // TP = 0 means not set
        assert!(!position.check_take_profit(200_000 * SCALAR_7));
    }

    // ==========================================
    // Stop Loss Tests
    // ==========================================

    #[test]
    fn test_check_stop_loss_long_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.stop_loss = 95_000 * SCALAR_7;

        // Price at or below SL
        assert!(position.check_stop_loss(95_000 * SCALAR_7));
        assert!(position.check_stop_loss(90_000 * SCALAR_7));
    }

    #[test]
    fn test_check_stop_loss_long_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.stop_loss = 95_000 * SCALAR_7;

        // Price above SL
        assert!(!position.check_stop_loss(95_001 * SCALAR_7));
    }

    #[test]
    fn test_check_stop_loss_short_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;
        position.stop_loss = 105_000 * SCALAR_7;

        // For short, SL is above entry
        assert!(position.check_stop_loss(105_000 * SCALAR_7));
        assert!(position.check_stop_loss(110_000 * SCALAR_7));
    }

    #[test]
    fn test_check_stop_loss_short_not_triggered() {
        let e = Env::default();
        let mut position = create_test_position(&e);
        position.is_long = false;
        position.stop_loss = 105_000 * SCALAR_7;

        // Price below SL (not reached for short)
        assert!(!position.check_stop_loss(104_999 * SCALAR_7));
    }

    #[test]
    fn test_check_stop_loss_not_set() {
        let e = Env::default();
        let position = create_test_position(&e);

        // SL = 0 means not set
        assert!(!position.check_stop_loss(1 * SCALAR_7));
    }

    #[test]
    fn test_position_new() {
        use soroban_sdk::testutils::{Ledger, LedgerInfo};

        let e = Env::default();
        e.ledger().set(LedgerInfo {
            timestamp: 1000,
            protocol_version: 25,
            sequence_number: 100,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3110400,
        });

        let user = Address::generate(&e);

        let position = Position::new(
            &e,
            1,                      // id
            user.clone(),           // user
            true,                   // filled
            0,                      // asset_index
            true,                   // is_long
            90_000 * SCALAR_7,      // stop_loss
            110_000 * SCALAR_7,     // take_profit
            100_000 * SCALAR_7,     // entry_price
            1_000 * SCALAR_7,       // collateral
            10_000 * SCALAR_7,      // notional_size
            0,                      // interest_index
        );

        assert_eq!(position.id, 1);
        assert_eq!(position.user, user);
        assert_eq!(position.filled, true);
        assert_eq!(position.asset_index, 0);
        assert_eq!(position.is_long, true);
        assert_eq!(position.stop_loss, 90_000 * SCALAR_7);
        assert_eq!(position.take_profit, 110_000 * SCALAR_7);
        assert_eq!(position.entry_price, 100_000 * SCALAR_7);
        assert_eq!(position.collateral, 1_000 * SCALAR_7);
        assert_eq!(position.notional_size, 10_000 * SCALAR_7);
        assert_eq!(position.interest_index, 0);
        assert_eq!(position.created_at, 1000);
    }

    #[test]
    fn test_position_load_and_store() {
        use crate::testutils::create_trading;
        use soroban_sdk::testutils::{Ledger, LedgerInfo};

        let e = Env::default();
        e.ledger().set(LedgerInfo {
            timestamp: 1000,
            protocol_version: 25,
            sequence_number: 100,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3110400,
        });

        let (address, _) = create_trading(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Create and store a position
            let position = Position::new(
                &e,
                1,
                user.clone(),
                true,
                0,
                true,
                90_000 * SCALAR_7,
                110_000 * SCALAR_7,
                100_000 * SCALAR_7,
                1_000 * SCALAR_7,
                10_000 * SCALAR_7,
                0,
            );
            position.store(&e);

            // Load it back
            let loaded = Position::load(&e, 1);
            assert_eq!(loaded.id, 1);
            assert_eq!(loaded.user, user);
            assert_eq!(loaded.entry_price, 100_000 * SCALAR_7);
        });
    }
}
