use crate::constants::{SCALAR_18, SCALAR_7};
use crate::storage;
use crate::trading::market::Market;
use crate::types::ExecuteRequestType;
pub(crate) use crate::types::Position;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{Address, Env};

/// Calculated values for closing a position
/// Used by both user-initiated close and keeper actions
pub struct CloseCalculation {
    pub price: i128,
    pub pnl: i128,
    pub base_fee: i128,       // Fee based on notional size (may be 0 if balancing)
    pub impact_fee: i128,     // Price impact fee
    pub interest: i128,       // Accrued interest (can be negative for rebates)
    pub user_payout: i128,    // Amount to pay user (0 if loss > collateral)
    pub caller_fee: i128,     // Amount to pay caller/keeper
    pub vault_transfer: i128, // Positive = vault receives, negative = vault pays
}

impl CloseCalculation {
    /// Total fee (for internal calculations)
    pub fn total_fee(&self) -> i128 {
        self.base_fee + self.impact_fee + self.interest
    }
}

/// Fee breakdown for closing a position
pub struct FeeBreakdown {
    pub base_fee: i128,
    pub impact_fee: i128,
    pub interest: i128,
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

    pub fn require_auth(&self) {
        self.user.require_auth();
    }

    /// Check if the requested keeper action is allowed based on this position's filled status
    pub fn validate_keeper_action(&self, action: &ExecuteRequestType) -> bool {
        match action {
            ExecuteRequestType::Fill => !self.filled,
            ExecuteRequestType::StopLoss => self.filled,
            ExecuteRequestType::TakeProfit => self.filled,
            ExecuteRequestType::Liquidate => self.filled,
        }
    }

    /// Calculate accrued interest since the position was opened or last settled
    /// Returns positive value if interest is owed, negative if rebate is due
    pub fn calculate_accrued_interest(&self, e: &Env, market: &Market) -> i128 {
        let index_difference = if self.is_long {
            market.data.long_interest_index - self.interest_index
        } else {
            market.data.short_interest_index - self.interest_index
        };

        self.notional_size
            .fixed_mul_floor(e, &index_difference, &SCALAR_18)
    }

    pub fn calculate_fee_breakdown(&self, e: &Env, market: &Market) -> FeeBreakdown {
        // Pay base fee when closing a position on the dominant side
        // If balanced (both sides equal), both sides pay the base fee
        let is_long_dominant = market.data.long_notional_size > market.data.short_notional_size;
        let is_short_dominant = market.data.short_notional_size > market.data.long_notional_size;
        let is_balanced = market.data.long_notional_size == market.data.short_notional_size;

        let should_pay_base_fee = is_balanced
            || (is_long_dominant && self.is_long)
            || (is_short_dominant && !self.is_long);

        let base_fee = if should_pay_base_fee {
            self.notional_size
                .fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7)
        } else {
            0 // No base fee when closing on the non-dominant side
        };

        let impact_fee = self
            .notional_size
            .fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

        let interest = self.calculate_accrued_interest(e, market);

        FeeBreakdown {
            base_fee,
            impact_fee,
            interest,
        }
    }

    pub fn calculate_fee(&self, e: &Env, market: &Market) -> i128 {
        let breakdown = self.calculate_fee_breakdown(e, market);
        breakdown.base_fee + breakdown.impact_fee + breakdown.interest
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

    /// Calculate all values needed to close this position
    /// caller_take_rate: 0 for user-initiated close, non-zero for keeper actions
    pub fn calculate_close(
        &self,
        e: &Env,
        price: i128,
        caller_take_rate: i128,
        market: &Market,
    ) -> CloseCalculation {
        let pnl = self.calculate_pnl(e, price);
        let fee_breakdown = self.calculate_fee_breakdown(e, market);
        let fee = fee_breakdown.base_fee + fee_breakdown.impact_fee + fee_breakdown.interest;
        let raw_caller_fee = fee
            .fixed_mul_floor(e, &caller_take_rate, &SCALAR_7)
            .abs();

        let net_pnl = if fee >= 0 {
            pnl - fee
        } else {
            pnl + fee.abs() - raw_caller_fee
        };

        let payout = self.collateral + net_pnl;

        // Cap payout to max_payout (percentage of notional size)
        let max_payout_amount = self
            .notional_size
            .fixed_mul_floor(e, &market.config.max_payout, &SCALAR_7);
        let user_payout = payout.max(0).min(max_payout_amount);

        // Calculate remaining funds after paying user
        let remaining = (self.collateral - user_payout).max(0);

        // Caller fee capped to remaining funds (only when fee >= 0)
        let caller_fee = if fee >= 0 {
            raw_caller_fee.min(remaining)
        } else {
            0
        };

        // Vault transfer: positive = receives, negative = pays
        // Vault receives remaining collateral, but pays if user profit exceeds collateral
        let vault_transfer = if user_payout > self.collateral {
            -(user_payout - self.collateral) // Vault pays the excess
        } else {
            remaining - caller_fee // Vault receives the remainder
        };

        CloseCalculation {
            price,
            pnl,
            base_fee: fee_breakdown.base_fee,
            impact_fee: fee_breakdown.impact_fee,
            interest: fee_breakdown.interest,
            user_payout,
            caller_fee,
            vault_transfer,
        }
    }
}
