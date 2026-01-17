use crate::constants::{SCALAR_7, STATUS_ON_ICE};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::storage;
use crate::trading::core::Trading;
use crate::types::PositionStatus;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, vec, Env, IntoVal, Symbol, Val, Vec};

/// Close a position (handles both Open and Pending positions)
/// Pending positions are cancelled, Open positions are closed with PnL settlement
/// Returns (pnl, fee) tuple
pub fn execute_close_position(e: &Env, position_id: u32) -> (i128, i128) {
    let position = storage::get_position(e, position_id);
    position.user.require_auth();

    // Allow closing in Active (0) and OnIce (1), block in Frozen (2) and Setup (99)
    if storage::get_status(e) > STATUS_ON_ICE {
        panic_with_error!(e, TradingError::ContractPaused);
    }

    let mut trading = Trading::load(e, position.user.clone());
    let mut position = trading.load_position(e, position_id);

    let result = match position.status {
        PositionStatus::Pending => {
            // Cancel: refund collateral
            execute_cancel(e, &trading, &mut position)
        }
        PositionStatus::Open => {
            // Close: calculate PnL, fees, settle
            execute_close(e, &mut trading, &mut position)
        }
        PositionStatus::Closed => {
            panic_with_error!(e, TradingError::PositionAlreadyClosed);
        }
    };

    trading.store_cached_markets(e);
    trading.store_cached_positions(e);

    result
}

/// Internal cancel logic for pending positions
fn execute_cancel(
    e: &Env,
    trading: &Trading,
    position: &mut crate::types::Position,
) -> (i128, i128) {
    let token_client = TokenClient::new(e, &trading.token);

    // Refund collateral to user
    token_client.transfer(
        &e.current_contract_address(),
        &position.user,
        &position.collateral,
    );

    position.status = PositionStatus::Closed;
    storage::remove_user_position(e, &position.user, position.id);

    TradingEvents::cancel_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
    );

    // For cancelled pending orders: no PnL, no fee
    (0, 0)
}

/// Internal close logic for open positions
/// Uses shared calculate_close for PnL/fee calculation
fn execute_close(
    e: &Env,
    trading: &mut Trading,
    position: &mut crate::types::Position,
) -> (i128, i128) {
    let mut market = trading.load_market(e, &position.asset);
    let calc = position.calculate_close(e, trading, &market);

    let token_client = TokenClient::new(e, &trading.token);
    let vault_client = VaultClient::new(e, &storage::get_vault(e));

    // Handle vault transfer (negative = vault pays, positive = vault receives)
    if calc.vault_transfer < 0 {
        // Vault pays: withdraw from vault to this contract
        vault_client.strategy_withdraw(&e.current_contract_address(), &(-calc.vault_transfer));
    } else if calc.vault_transfer > 0 {
        // Vault receives: deposit from this contract to vault
        // Authorize the token transfer that happens inside strategy_deposit
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
            })
        ]);
        vault_client.strategy_deposit(&e.current_contract_address(), &calc.vault_transfer);
    }

    // Pay user
    if calc.user_payout > 0 {
        token_client.transfer(&e.current_contract_address(), &position.user, &calc.user_payout);
    }

    // Pay caller fee
    if calc.caller_fee > 0 {
        token_client.transfer(&e.current_contract_address(), &trading.caller, &calc.caller_fee);
    }

    // Update market stats
    market.update_stats(
        -position.collateral,
        -position.notional_size,
        position.is_long,
    );

    position.status = PositionStatus::Closed;
    storage::remove_user_position(e, &position.user, position.id);

    TradingEvents::close_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        calc.price,
        calc.fee,
    );

    trading.cache_market(&market);
    trading.cache_position(position);

    (calc.pnl, calc.fee)
}

/// Modify collateral on a position to a new absolute value
/// Returns the interest fee settled (positive = paid, negative = received)
pub fn execute_modify_collateral(e: &Env, position_id: u32, new_collateral: i128) -> i128 {
    let position = storage::get_position(e, position_id);
    position.user.require_auth();

    // Allow modifying in Active (0) and OnIce (1), block in Frozen (2) and Setup (99)
    if storage::get_status(e) > STATUS_ON_ICE {
        panic_with_error!(e, TradingError::ContractPaused);
    }

    let mut trading = Trading::load(e, position.user.clone());
    let mut position = trading.load_position(e, position_id);

    // Must be open position
    if position.status != PositionStatus::Open {
        panic_with_error!(e, TradingError::PositionNotOpen);
    }

    if new_collateral <= 0 {
        panic_with_error!(e, TradingError::InvalidCollateral);
    }

    let mut market = trading.load_market(e, &position.asset);
    let current_price = trading.load_price(e, &position.asset);

    // Handle accrued interest first
    let interest_fee = position.calculate_accrued_interest(e, &market);

    let token_client = TokenClient::new(e, &trading.token);
    let vault_client = VaultClient::new(e, &storage::get_vault(e));

    if interest_fee > 0 {
        position.collateral -= interest_fee;
        market.update_stats(-interest_fee, 0, position.is_long);

        // Transfer interest to vault via strategy_deposit
        // Authorize the token transfer that happens inside strategy_deposit
        let args: Vec<Val> = vec![
            e,
            e.current_contract_address().into_val(e),
            vault_client.address.into_val(e),
            interest_fee.into_val(e),
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
        vault_client.strategy_deposit(&e.current_contract_address(), &interest_fee);
    } else if interest_fee < 0 {
        position.collateral += interest_fee.abs();
        market.update_stats(interest_fee.abs(), 0, position.is_long);
        // Receive interest rebate from vault via strategy_withdraw
        vault_client.strategy_withdraw(&e.current_contract_address(), &interest_fee.abs());
    }

    // Update interest index
    position.interest_index = if position.is_long {
        market.data.long_interest_index
    } else {
        market.data.short_interest_index
    };

    // Calculate the difference between new and current collateral
    let collateral_diff = new_collateral - position.collateral;

    if collateral_diff > 0 {
        // Deposit collateral
        token_client.transfer(&position.user, &e.current_contract_address(), &collateral_diff);
        position.collateral = new_collateral;
        market.update_stats(collateral_diff, 0, position.is_long);

        TradingEvents::deposit_collateral(
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

        TradingEvents::withdraw_collateral(
            e,
            position.user.clone(),
            position.asset.clone(),
            position.id,
            withdraw_amount,
        );
    }
    // If collateral_diff == 0, no transfer needed (just settled interest)

    trading.cache_market(&market);
    trading.cache_position(&position);
    trading.store_cached_markets(e);
    trading.store_cached_positions(e);

    interest_fee
}

/// Set take profit and stop loss triggers
/// Use 0 to clear/disable a trigger
pub fn execute_set_triggers(
    e: &Env,
    position_id: u32,
    take_profit: i128,
    stop_loss: i128,
) {
    let position = storage::get_position(e, position_id);
    position.user.require_auth();

    // Allow setting triggers in Active (0) and OnIce (1), block in Frozen (2) and Setup (99)
    if storage::get_status(e) > STATUS_ON_ICE {
        panic_with_error!(e, TradingError::ContractPaused);
    }

    let mut trading = Trading::load(e, position.user.clone());
    let mut position = trading.load_position(e, position_id);

    // Must be open position
    if position.status != PositionStatus::Open {
        panic_with_error!(e, TradingError::PositionNotOpen);
    }

    let current_price = trading.load_price(e, &position.asset);

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

        TradingEvents::set_take_profit(
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

        TradingEvents::set_stop_loss(
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

    trading.cache_position(&position);
    trading.store_cached_positions(e);
}
