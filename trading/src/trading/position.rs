use crate::constants::{SCALAR_18, SCALAR_7, STATUS_ACTIVE};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::storage;
use crate::trading::core::Trading;
use crate::trading::market::Market;
use crate::types::ExecuteRequestType;
pub(crate) use crate::types::{Position, PositionStatus};
use sep_40_oracle::Asset;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, vec, Address, Env, IntoVal, Symbol, Val, Vec};

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
    ///
    /// Returns true if the action is allowed, false otherwise
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

        let price_impact_scalar =
            self.notional_size
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

    /// Calculate all values needed to close this position
    /// Returns a CloseCalculation struct with all transfer amounts
    pub fn calculate_close(&self, e: &Env, trading: &mut Trading, market: &Market) -> CloseCalculation {
        let price = trading.load_price(e, &self.asset);
        let pnl = self.calculate_pnl(e, price);
        let fee = self.calculate_fee(e, market);
        let raw_caller_fee = trading.calculate_caller_fee(e, fee).abs();

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
        let caller_fee = if fee >= 0 { raw_caller_fee.min(remaining) } else { 0 };

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

#[allow(clippy::too_many_arguments)]
pub fn execute_create_position(
    e: &Env,
    user: &Address,
    asset: &Asset,
    collateral: i128,
    notional_size: i128,
    is_long: bool,
    entry_price: i128,
    take_profit: i128,
    stop_loss: i128,
) -> (u32, i128) {
    user.require_auth();

    // Only allow opening new positions when contract is Active (0)
    if storage::get_status(e) != STATUS_ACTIVE {
        panic_with_error!(e, TradingError::ContractPaused);
    }

    let mut trading = Trading::load(e, user.clone());
    let mut market = trading.load_market(e, asset);

    if collateral < 0 || notional_size < 0 || entry_price < 0 {
        panic_with_error!(e, TradingError::InvalidCollateral);
    }

    // Check utilization limit: total_notional must not exceed vault_assets * max_utilization
    let config = storage::get_config(e);
    if config.max_utilization > 0 {
        let vault_client = VaultClient::new(e, &storage::get_vault(e));
        let vault_assets = vault_client.total_assets();
        let current_total_notional = market.data.long_notional_size + market.data.short_notional_size;
        let new_total_notional = current_total_notional + notional_size;

        // Check: new_total_notional * SCALAR_7 <= vault_assets * max_utilization
        let max_allowed_notional = vault_assets.fixed_mul_floor(e, &config.max_utilization, &SCALAR_7);
        if new_total_notional > max_allowed_notional {
            panic_with_error!(e, TradingError::UtilizationLimitExceeded);
        }
    }

    // Check collateral bounds
    if collateral < market.config.min_collateral {
        panic_with_error!(e, TradingError::InvalidCollateral);
    }
    if collateral > market.config.max_collateral {
        panic_with_error!(e, TradingError::InvalidCollateral);
    }

    // Check user position count limit
    let positions = storage::get_user_positions(e, user);
    if !trading.check_max_positions(positions) {
        panic_with_error!(e, TradingError::MaxPositionsReached)
    }

    let current_price = trading.load_price(e, asset);
    let market_order = entry_price == 0;
    let status = if market_order {
        PositionStatus::Open
    } else {
        PositionStatus::Pending
    };

    let actual_entry_price = if market_order {
        current_price
    } else {
        // Check if entry price is valid
        if (is_long && entry_price < current_price) || (!is_long && entry_price > current_price) {
            panic_with_error!(e, TradingError::InvalidEntryPrice);
        }
        entry_price
    };

    // Pay base fee only for the dominant side (side that increases market imbalance)
    // If the position balances the market, no base fee is charged
    // Check BEFORE updating market stats to see if this position would balance the market
    let should_pay_base_fee = if is_long {
        // Long position pays fee if it increases long dominance
        // Check if after adding this position, longs would still be >= shorts
        let new_long_notional = market.data.long_notional_size + notional_size;
        new_long_notional > market.data.short_notional_size
    } else {
        // Short position pays fee if it increases short dominance
        // Check if after adding this position, shorts would still be >= longs
        let new_short_notional = market.data.short_notional_size + notional_size;
        new_short_notional > market.data.long_notional_size
    };

    // If market order, update market stats immediately
    if market_order {
        market.update_stats(collateral, notional_size, is_long);
        trading.cache_market(&market);
    }

    let interest_index = if is_long {
        market.data.long_interest_index
    } else {
        market.data.short_interest_index
    };

    let id = storage::bump_position_id(e);
    let position = Position {
        id,
        user: user.clone(),
        status: status.clone(),
        asset: asset.clone(),
        is_long,
        stop_loss,
        take_profit,
        entry_price: actual_entry_price,
        collateral,
        notional_size,
        interest_index,
        created_at: e.ledger().timestamp(),
    };

    let open_fee = if should_pay_base_fee {
        notional_size.fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7)
    } else {
        0 // No base fee for balancing trades
    };

    let price_impact_scalar =
        notional_size.fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

    // Transfer tokens from user to contract
    let token_client = TokenClient::new(e, &trading.token);
    token_client.transfer(
        user,
        &e.current_contract_address(),
        &(collateral + open_fee + price_impact_scalar),
    );

    // Only pay fee to vault when the position fills
    if market_order {
        let vault_client = VaultClient::new(e, &storage::get_vault(e));
        let vault_transfer = open_fee + price_impact_scalar;

        // Authorize the token transfer that happens inside strategy_deposit
        let args: Vec<Val> = vec![
            e,
            e.current_contract_address().into_val(e),
            vault_client.address.into_val(e),
            vault_transfer.into_val(e),
        ];
        e.authorize_as_current_contract(vec![
            e,
            InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: token_client.address.clone(),
                    fn_name: Symbol::new(e, "transfer"),
                    args: args.clone(),
                },
                sub_invocations: vec![e],
            })
        ]);

        vault_client.strategy_deposit(&e.current_contract_address(), &vault_transfer);
    }

    trading.cache_position(&position);
    trading.store_cached_markets(e);
    trading.store_cached_positions(e);

    storage::add_user_position(e, user, id);

    TradingEvents::open_position(e, user.clone(), asset.clone(), id);

    (id, open_fee + price_impact_scalar)
}
