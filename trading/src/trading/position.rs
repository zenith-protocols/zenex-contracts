use crate::constants::{SCALAR_18, SCALAR_7};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::storage;
use crate::trading::actions::RequestType;
use crate::trading::core::Trading;
use crate::trading::market::Market;
pub(crate) use crate::types::{Position, PositionStatus};
use sep_40_oracle::Asset;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{log, panic_with_error, vec, Address, Env, IntoVal, Symbol, Val, Vec};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};

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

    /// Check if the requested action is allowed based on this position's status
    ///
    /// Returns true if the action is allowed, false otherwise
    pub fn validate_action(&self, action: &RequestType) -> bool {
        match action {
            RequestType::Close => self.status == PositionStatus::Open,
            RequestType::Fill => self.status == PositionStatus::Pending,
            RequestType::StopLoss => self.status == PositionStatus::Open,
            RequestType::TakeProfit => self.status == PositionStatus::Open,
            RequestType::Liquidation => self.status == PositionStatus::Open,
            RequestType::Cancel => self.status == PositionStatus::Pending,
            RequestType::WithdrawCollateral => self.status == PositionStatus::Open,
            RequestType::DepositCollateral => self.status == PositionStatus::Open,
            RequestType::SetTakeProfit => self.status == PositionStatus::Open,
            RequestType::SetStopLoss => self.status == PositionStatus::Open,
        }
    }

    pub fn calculate_fee(&self, e: &Env, market: &Market) -> i128 {
        // Pay base fee when closing a position on the dominant side.
        // If there's no dominant side (both sides equal), both sides pay the base fee.
        // Dominant side is determined before closing this position.
        let is_long_dominant = market.data.long_notional_size > market.data.short_notional_size;
        let is_short_dominant = market.data.short_notional_size > market.data.long_notional_size;
        let is_balanced = market.data.long_notional_size == market.data.short_notional_size;
        
        // Charge base fee if closing on the dominant side, or if market is balanced (both sides pay)
        let should_pay_base_fee = is_balanced || (is_long_dominant && self.is_long) || (is_short_dominant && !self.is_long);

        let base_fee = if should_pay_base_fee {
            self.notional_size
                .fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7)
        } else {
            0 // No base fee when closing on the non-dominant side
        };

        let price_impact_scalar =
            self.notional_size
                .fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

        let index_difference = if self.is_long {
            market.data.long_interest_index - self.interest_index
        } else {
            market.data.short_interest_index - self.interest_index
        };

        let interest_fee = self
            .notional_size
            .fixed_mul_floor(e, &index_difference, &SCALAR_18);

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
) -> u32 {
    user.require_auth();
    let mut trading = Trading::load(e, user.clone());
    let mut market = trading.load_market(e, asset);

    if collateral < 0 || notional_size < 0 || entry_price < 0 {
        panic_with_error!(e, TradingError::BadRequest);
    }

    // Check user position count limit
    let positions = storage::get_user_positions(e, user);
    if !trading.check_max_positions(positions) {
        panic_with_error!(e, TradingError::MaxPositions)
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
            panic_with_error!(e, TradingError::BadRequest);
        }
        entry_price
    };

    // Pay base fee when opening a position on the dominant side.
    // If there's no dominant side (both sides equal, including when both are 0), both sides pay the base fee.
    // Dominant side is determined before adding this new position.
    let is_long_dominant = market.data.long_notional_size > market.data.short_notional_size;
    let is_short_dominant = market.data.short_notional_size > market.data.long_notional_size;
    let is_balanced = market.data.long_notional_size == market.data.short_notional_size;
    
    // Charge base fee if opening on the dominant side, or if market is balanced (both sides pay)
    let should_pay_base_fee = is_balanced || (is_long_dominant && is_long) || (is_short_dominant && !is_long);

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
        0 // No base fee when opening on the non-dominant side
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

        vault_client.transfer_from(
            &e.current_contract_address(),
            &vault_transfer,
        );
    }

    trading.cache_position(&position);
    trading.store_cached_markets(e);
    trading.store_cached_positions(e);

    storage::add_user_position(e, user, id);

    TradingEvents::open_position(e, user.clone(), asset.clone(), id);

    id
}
