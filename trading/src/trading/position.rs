use crate::constants::{SCALAR_18, SCALAR_7};
use crate::storage;
use crate::trading::actions::ActionContext;
use crate::trading::execute::ExecuteContext;
use crate::trading::market::Market;
use crate::types::ExecuteRequestType;
pub(crate) use crate::types::{Position, PositionStatus};
use sep_40_oracle::Asset;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{Address, Env};

/// Calculated values for closing a position
/// Used by both user-initiated close and keeper actions
pub struct CloseCalculation {
    pub price: i128,
    pub pnl: i128,
    pub fee: i128,
    pub user_payout: i128,    // Amount to pay user (0 if loss > collateral)
    pub caller_fee: i128,     // Amount to pay caller/keeper
    pub vault_transfer: i128, // Positive = vault receives, negative = vault pays
}

/// Implementation of position-related methods
impl Position {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        e: &Env,
        id: u32,
        user: Address,
        status: PositionStatus,
        asset: Asset,
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
            status,
            asset,
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

    /// Check if the requested keeper action is allowed based on this position's status
    pub fn validate_keeper_action(&self, action: &ExecuteRequestType) -> bool {
        match action {
            ExecuteRequestType::Fill => self.status == PositionStatus::Pending,
            ExecuteRequestType::StopLoss => self.status == PositionStatus::Open,
            ExecuteRequestType::TakeProfit => self.status == PositionStatus::Open,
            ExecuteRequestType::Liquidate => self.status == PositionStatus::Open,
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

    pub fn calculate_fee(&self, e: &Env, market: &Market) -> i128 {
        // Pay base fee only for the dominant side (side that increases market imbalance)
        // When closing, we check if closing this position would REDUCE the dominant side's imbalance
        // If closing reduces imbalance (i.e., this position is on the dominant side), charge base fee
        let should_pay_base_fee = if self.is_long {
            // Closing a long position reduces long dominance, so pay fee if long is currently dominant
            market.data.long_notional_size >= market.data.short_notional_size
        } else {
            // Closing a short position reduces short dominance, so pay fee if short is currently dominant
            market.data.short_notional_size >= market.data.long_notional_size
        };

        let base_fee = if should_pay_base_fee {
            self.notional_size
                .fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7)
        } else {
            0 // No base fee when closing a balancing position
        };

        let price_impact_scalar = self
            .notional_size
            .fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

        let interest_fee = self.calculate_accrued_interest(e, market);

        base_fee + price_impact_scalar + interest_fee
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

    /// Calculate all values needed to close this position (for batch keeper execution)
    /// Returns a CloseCalculation struct with all transfer amounts including caller_fee
    pub fn calculate_close(
        &self,
        e: &Env,
        ctx: &mut ExecuteContext,
        market: &Market,
    ) -> CloseCalculation {
        let price = ctx.load_price(e, &self.asset);
        self.calculate_close_internal(e, price, ctx.config.caller_take_rate, market)
    }

    /// Calculate all values needed to close this position (for user-initiated close)
    /// No caller_fee since user is closing their own position
    pub fn calculate_close_for_user(
        &self,
        e: &Env,
        ctx: &ActionContext,
        market: &Market,
    ) -> CloseCalculation {
        let price = ctx.load_price(e, &self.asset);
        self.calculate_close_internal(e, price, 0, market) // No caller fee for user actions
    }

    /// Internal calculation logic shared by both contexts
    fn calculate_close_internal(
        &self,
        e: &Env,
        price: i128,
        caller_take_rate: i128,
        market: &Market,
    ) -> CloseCalculation {
        let pnl = self.calculate_pnl(e, price);
        let fee = self.calculate_fee(e, market);
        let raw_caller_fee = fee
            .fixed_mul_floor(e, &caller_take_rate, &SCALAR_7)
            .abs();

        let net_pnl = if fee >= 0 {
            pnl - fee
        } else {
            pnl + fee.abs() - raw_caller_fee
        };

        let payout = self.collateral + net_pnl;
        let user_payout = payout.max(0);

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
            fee,
            user_payout,
            caller_fee,
            vault_transfer,
        }
    }
}
