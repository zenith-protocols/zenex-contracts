use crate::constants::{MAX_PRICE_AGE, SCALAR_7, STATUS_ACTIVE, STATUS_ON_ICE, STATUS_SETUP};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{
    emit_cancel_position, emit_close_position, emit_deposit_collateral, emit_open_position,
    emit_set_stop_loss, emit_set_take_profit, emit_withdraw_collateral,
};
use crate::storage;
use crate::trading::market::Market;
use crate::trading::position::Position;
use crate::types::{PositionStatus, TradingConfig};
use sep_40_oracle::{Asset, PriceFeedClient};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, vec, Address, Env, IntoVal, Symbol, Val, Vec};

/// Lightweight context for single-operation user actions
/// Unlike ExecuteContext, does not cache markets/positions (not needed for single ops)
pub struct ActionContext {
    pub config: TradingConfig,
    pub vault: Address,
    pub token: Address,
    pub status: u32,
}

impl ActionContext {
    pub fn load(e: &Env) -> Self {
        let status = storage::get_status(e);
        if status == STATUS_SETUP {
            panic_with_error!(e, TradingError::ContractPaused);
        }
        ActionContext {
            config: storage::get_config(e),
            vault: storage::get_vault(e),
            token: storage::get_token(e),
            status,
        }
    }

    pub fn load_price(&self, e: &Env, asset: &Asset) -> i128 {
        let price_data = match PriceFeedClient::new(e, &self.config.oracle).lastprice(asset) {
            Some(price) => price,
            None => panic_with_error!(e, TradingError::PriceNotFound),
        };
        if price_data.timestamp + MAX_PRICE_AGE < e.ledger().timestamp() {
            panic_with_error!(e, TradingError::PriceStale);
        }
        price_data.price
    }
}

/// Close a position (handles both Open and Pending positions)
/// Pending positions are cancelled, Open positions are closed with PnL settlement
/// Returns (pnl, fee) tuple
pub fn execute_close_position(e: &Env, position_id: u32) -> (i128, i128) {
    let mut position = Position::load(e, position_id);
    position.user.require_auth();

    let ctx = ActionContext::load(e);

    // Allow closing in Active (0) and OnIce (1), block in Frozen (2)
    if ctx.status != STATUS_ACTIVE && ctx.status != STATUS_ON_ICE {
        panic_with_error!(e, TradingError::ContractPaused);
    }

    match position.status {
        PositionStatus::Pending => execute_cancel(e, &ctx, &mut position),
        PositionStatus::Open => execute_close(e, &ctx, &mut position),
        PositionStatus::Closed => {
            panic_with_error!(e, TradingError::PositionAlreadyClosed);
        }
    }
}

/// Internal cancel logic for pending positions
/// Refunds collateral + base_fee + price_impact that were charged at open
fn execute_cancel(e: &Env, ctx: &ActionContext, position: &mut Position) -> (i128, i128) {
    let token_client = TokenClient::new(e, &ctx.token);
    let mut market = Market::load_checked(e, &position.asset);
    market.update_borrowing_index(e);

    // Calculate fees using the same formula as open (notional_size based)
    let base_fee = position
        .notional_size
        .fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7);
    let price_impact = position
        .notional_size
        .fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

    // Refund collateral + fees to user
    let total_refund = position.collateral + base_fee + price_impact;
    token_client.transfer(&e.current_contract_address(), &position.user, &total_refund);

    position.status = PositionStatus::Closed;
    storage::remove_user_position(e, &position.user, position.id);
    position.store(e);

    emit_cancel_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
    );

    // For cancelled pending orders: no PnL, no fee
    (0, 0)
}

/// Internal close logic for open positions
fn execute_close(e: &Env, ctx: &ActionContext, position: &mut Position) -> (i128, i128) {
    let mut market = Market::load_checked(e, &position.asset);
    market.update_borrowing_index(e);
    let calc = position.calculate_close_for_user(e, ctx, &market);

    let token_client = TokenClient::new(e, &ctx.token);
    let vault_client = VaultClient::new(e, &ctx.vault);

    // Handle vault transfer (negative = vault pays, positive = vault receives)
    if calc.vault_transfer < 0 {
        // Vault pays: withdraw from vault to this contract
        vault_client.strategy_withdraw(&e.current_contract_address(), &(-calc.vault_transfer));
    } else if calc.vault_transfer > 0 {
        // Vault receives: deposit from this contract to vault
        let args: Vec<Val> = vec![
            e,
            e.current_contract_address().into_val(e),
            vault_client.address.into_val(e),
            calc.vault_transfer.into_val(e),
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
            }),
        ]);
        vault_client.strategy_deposit(&e.current_contract_address(), &calc.vault_transfer);
    }

    // Pay user their payout
    if calc.user_payout > 0 {
        token_client.transfer(&e.current_contract_address(), &position.user, &calc.user_payout);
    }

    // Update market stats
    market.update_stats(
        -position.collateral,
        -position.notional_size,
        position.is_long,
    );

    position.status = PositionStatus::Closed;
    storage::remove_user_position(e, &position.user, position.id);

    emit_close_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        calc.price,
        calc.fee,
    );

    market.store(e);
    position.store(e);

    (calc.pnl, calc.fee)
}

/// Modify collateral on a position to a new absolute value
pub fn execute_modify_collateral(e: &Env, position_id: u32, new_collateral: i128) {
    let mut position = Position::load(e, position_id);
    position.user.require_auth();

    let ctx = ActionContext::load(e);

    // Allow modifying in Active (0) and OnIce (1), block in Frozen (2)
    if ctx.status != STATUS_ACTIVE && ctx.status != STATUS_ON_ICE {
        panic_with_error!(e, TradingError::ContractPaused);
    }

    // Must be open position
    if position.status != PositionStatus::Open {
        panic_with_error!(e, TradingError::PositionNotOpen);
    }

    if new_collateral <= 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    let mut market = Market::load_checked(e, &position.asset);
    market.update_borrowing_index(e);
    let current_price = ctx.load_price(e, &position.asset);

    let token_client = TokenClient::new(e, &ctx.token);

    // Calculate the difference between new and current collateral
    let collateral_diff = new_collateral - position.collateral;

    if collateral_diff > 0 {
        // Deposit collateral
        token_client.transfer(&position.user, &e.current_contract_address(), &collateral_diff);
        position.collateral = new_collateral;
        market.update_stats(collateral_diff, 0, position.is_long);

        emit_deposit_collateral(
            e,
            position.user.clone(),
            position.asset.clone(),
            position.id,
            collateral_diff,
        );
    } else if collateral_diff < 0 {
        // Withdraw collateral
        let withdraw_amount = -collateral_diff;

        let pnl = position.calculate_pnl(e, current_price);
        let equity = new_collateral + pnl;
        let required_margin = position
            .notional_size
            .fixed_mul_floor(e, &market.config.init_margin, &SCALAR_7);

        if equity < required_margin {
            panic_with_error!(e, TradingError::WithdrawalBreaksMargin);
        }

        position.collateral = new_collateral;
        token_client.transfer(&e.current_contract_address(), &position.user, &withdraw_amount);
        market.update_stats(-withdraw_amount, 0, position.is_long);

        emit_withdraw_collateral(
            e,
            position.user.clone(),
            position.asset.clone(),
            position.id,
            withdraw_amount,
        );
    }
    // If collateral_diff == 0, no transfer needed

    market.store(e);
    position.store(e);
}

/// Set take profit and stop loss triggers
/// Use 0 to clear/disable a trigger
pub fn execute_set_triggers(e: &Env, position_id: u32, take_profit: i128, stop_loss: i128) {
    let mut position = Position::load(e, position_id);
    position.user.require_auth();

    let ctx = ActionContext::load(e);

    // Allow setting triggers in Active (0) and OnIce (1), block in Frozen (2)
    if ctx.status != STATUS_ACTIVE && ctx.status != STATUS_ON_ICE {
        panic_with_error!(e, TradingError::ContractPaused);
    }

    // Must be open position
    if position.status != PositionStatus::Open {
        panic_with_error!(e, TradingError::PositionNotOpen);
    }

    let current_price = ctx.load_price(e, &position.asset);

    // Validate and set take profit
    if take_profit > 0 {
        if position.is_long {
            // For long positions, take profit must be above current price
            if take_profit <= current_price {
                panic_with_error!(e, TradingError::InvalidTakeProfitPrice);
            }
        } else {
            // For short positions, take profit must be below current price
            if take_profit >= current_price {
                panic_with_error!(e, TradingError::InvalidTakeProfitPrice);
            }
        }
        position.take_profit = take_profit;

        emit_set_take_profit(
            e,
            position.user.clone(),
            position.asset.clone(),
            position.id,
            take_profit,
        );
    } else if take_profit == 0 {
        // Clear take profit
        position.take_profit = 0;
    }

    // Validate and set stop loss
    if stop_loss > 0 {
        if position.is_long {
            // For long positions, stop loss must be below current price
            if stop_loss >= current_price {
                panic_with_error!(e, TradingError::InvalidStopLossPrice);
            }
        } else {
            // For short positions, stop loss must be above current price
            if stop_loss <= current_price {
                panic_with_error!(e, TradingError::InvalidStopLossPrice);
            }
        }
        position.stop_loss = stop_loss;

        emit_set_stop_loss(
            e,
            position.user.clone(),
            position.asset.clone(),
            position.id,
            stop_loss,
        );
    } else if stop_loss == 0 {
        // Clear stop loss
        position.stop_loss = 0;
    }

    position.store(e);
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

    let ctx = ActionContext::load(e);

    // Only allow opening new positions when contract is Active
    if ctx.status != STATUS_ACTIVE {
        panic_with_error!(e, TradingError::ContractPaused);
    }

    let mut market = Market::load_checked(e, asset);
    market.update_borrowing_index(e);

    if collateral < 0 || notional_size < 0 || entry_price < 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    // Check utilization limit: total_notional must not exceed vault_assets * max_utilization
    if ctx.config.max_utilization > 0 {
        let vault_client = VaultClient::new(e, &ctx.vault);
        let vault_assets = vault_client.total_assets();
        let current_total_notional =
            market.data.long_notional_size + market.data.short_notional_size;
        let new_total_notional = current_total_notional + notional_size;

        let max_allowed_notional =
            vault_assets.fixed_mul_floor(e, &ctx.config.max_utilization, &SCALAR_7);
        if new_total_notional > max_allowed_notional {
            panic_with_error!(e, TradingError::UtilizationLimitExceeded);
        }
    }

    // Check collateral bounds
    if collateral < market.config.min_collateral {
        panic_with_error!(e, TradingError::CollateralBelowMinimum);
    }
    if collateral > market.config.max_collateral {
        panic_with_error!(e, TradingError::CollateralAboveMaximum);
    }

    // Check user position count limit
    let positions = storage::get_user_positions(e, user);
    if positions.len() >= ctx.config.max_positions {
        panic_with_error!(e, TradingError::MaxPositionsReached)
    }

    let current_price = ctx.load_price(e, asset);
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

    // For limit orders: always charge base fee (will be refunded on fill if balancing)
    // For market orders: only charge base fee if position increases market imbalance
    let should_pay_base_fee = if market_order {
        // Check BEFORE updating market stats to see if this position would balance the market
        if is_long {
            let new_long_notional = market.data.long_notional_size + notional_size;
            new_long_notional > market.data.short_notional_size
        } else {
            let new_short_notional = market.data.short_notional_size + notional_size;
            new_short_notional > market.data.long_notional_size
        }
    } else {
        // Limit orders always pay base fee upfront (refunded on fill if balancing)
        true
    };

    // If market order, update market stats immediately
    if market_order {
        market.update_stats(collateral, notional_size, is_long);
    }

    let interest_index = if is_long {
        market.data.long_interest_index
    } else {
        market.data.short_interest_index
    };

    let id = storage::bump_position_id(e);
    let position = Position::new(
        e,
        id,
        user.clone(),
        status,
        asset.clone(),
        is_long,
        stop_loss,
        take_profit,
        actual_entry_price,
        collateral,
        notional_size,
        interest_index,
    );

    let open_fee = if should_pay_base_fee {
        notional_size.fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7)
    } else {
        0 // No base fee for balancing trades
    };

    let price_impact_scalar =
        notional_size.fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

    // Transfer tokens from user to contract
    let token_client = TokenClient::new(e, &ctx.token);
    token_client.transfer(
        user,
        &e.current_contract_address(),
        &(collateral + open_fee + price_impact_scalar),
    );

    // Only pay fee to vault when the position fills
    if market_order {
        let vault_client = VaultClient::new(e, &ctx.vault);
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
            }),
        ]);

        vault_client.strategy_deposit(&e.current_contract_address(), &vault_transfer);
    }

    market.store(e);
    position.store(e);

    storage::add_user_position(e, user, id);

    emit_open_position(e, user.clone(), asset.clone(), id);

    (id, open_fee + price_impact_scalar)
}
