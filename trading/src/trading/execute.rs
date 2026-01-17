use crate::constants::{SCALAR_7, STATUS_ON_ICE};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::storage;
use crate::trading::core::Trading;
use crate::trading::position::Position;
use crate::types::{ExecuteRequest, ExecuteRequestType, PositionStatus};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, vec, Address, Env, IntoVal, Map, Symbol, Val, Vec};

/// Internal processing result that tracks transfers for execution
/// The transfers map is used internally but not exposed to callers
pub(crate) struct ProcessingResult {
    pub transfers: Map<Address, i128>,
    pub results: Vec<u32>,
}

impl ProcessingResult {
    /// Create an empty processing result
    pub fn new(e: &Env) -> Self {
        ProcessingResult {
            transfers: Map::new(e),
            results: Vec::new(e),
        }
    }

    pub fn add_transfer(&mut self, address: &Address, amount: i128) {
        self.transfers.set(
            address.clone(),
            amount + self.transfers.get(address.clone()).unwrap_or(0),
        );
    }
}

/// Execute keeper triggers (Fill, StopLoss, TakeProfit, Liquidate)
/// This function is permissionless - anyone can call it to trigger these actions
/// Returns Vec<u32> with result codes (0 = success, error code otherwise)
pub fn execute_trigger(
    e: &Env,
    caller: &Address,
    requests: Vec<ExecuteRequest>,
) -> Vec<u32> {
    // Allow keeper actions in Active (0) and OnIce (1), block in Frozen (2) and Setup (99)
    if storage::get_status(e) > STATUS_ON_ICE {
        panic_with_error!(e, TradingError::ContractPaused);
    }

    let mut trading = Trading::load(e, caller.clone());
    let processing_result = process_execute_requests(e, &mut trading, requests);

    let token_client = TokenClient::new(e, &storage::get_token(e));
    let vault_client = VaultClient::new(e, &storage::get_vault(e));

    // STEP 1: Vault pays to contract (if needed)
    // This is done first to ensure the contract has enough balance to handle transfers
    let vault_transfer = processing_result.transfers.get(trading.vault.clone()).unwrap_or(0);
    if vault_transfer < 0 {
        // Vault pays: withdraw from vault to this contract
        vault_client.strategy_withdraw(&e.current_contract_address(), &vault_transfer.abs());
    }

    // STEP 2: Handle all other transfers (callers receiving fees, users receiving payouts)
    for (address, amount) in processing_result.transfers.iter() {
        if address != trading.vault {
            if amount > 0 {
                // Contract pays to user/caller
                token_client.transfer(&e.current_contract_address(), &address, &amount);
            }
            // Note: For keeper actions, we don't expect negative amounts (users paying)
            // since all user payments are handled by user_actions.rs directly
        }
    }

    // STEP 3: Contract pays to vault if needed
    // This is done last to ensure the contract has enough balance
    if vault_transfer > 0 {
        // Vault receives: deposit from this contract to vault
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

    processing_result.results
}

/// Process batch of keeper requests (Fill, StopLoss, TakeProfit, Liquidate)
/// This function is permissionless - anyone can call it to trigger these actions
/// Returns ProcessingResult which contains transfers (for internal use) and results
fn process_execute_requests(
    e: &Env,
    trading: &mut Trading,
    requests: Vec<ExecuteRequest>,
) -> ProcessingResult {
    let mut result = ProcessingResult::new(e);

    for request in requests.iter() {
        // Try to load position - return error code if not found
        let mut position = match trading.try_load_position(e, request.position_id) {
            Ok(pos) => pos,
            Err(error_code) => {
                result.results.push_back(error_code);
                continue;
            }
        };

        // Validate position status for the requested action
        let (is_valid, specific_error) = match request.request_type {
            ExecuteRequestType::Fill => {
                if position.status != PositionStatus::Pending {
                    (false, TradingError::PositionNotPending as u32)
                } else {
                    (true, 0)
                }
            }
            ExecuteRequestType::StopLoss
            | ExecuteRequestType::TakeProfit
            | ExecuteRequestType::Liquidate => {
                if position.status != PositionStatus::Open {
                    (false, TradingError::ActionNotAllowedForStatus as u32)
                } else {
                    (true, 0)
                }
            }
        };

        if !is_valid {
            result.results.push_back(specific_error);
            continue;
        }

        let action_result = match request.request_type {
            ExecuteRequestType::Fill => apply_fill(e, &mut result, trading, &mut position),
            ExecuteRequestType::StopLoss => apply_stop_loss(e, &mut result, trading, &mut position),
            ExecuteRequestType::TakeProfit => apply_take_profit(e, &mut result, trading, &mut position),
            ExecuteRequestType::Liquidate => apply_liquidation(e, &mut result, trading, &mut position),
        };

        result.results.push_back(action_result);
    }

    trading.store_cached_markets(e);
    trading.store_cached_positions(e);

    result
}

/// Handle position close logic shared by multiple actions
/// Uses shared calculate_close for PnL/fee calculation
fn handle_close(
    e: &Env,
    result: &mut ProcessingResult,
    trading: &mut Trading,
    position: &mut Position,
) -> (i128, i128) {
    let mut market = trading.load_market(e, &position.asset);
    let calc = position.calculate_close(e, trading, &market);

    // User receives their payout (if positive)
    if calc.user_payout > 0 {
        result.add_transfer(&position.user, calc.user_payout);
    }

    // Vault transfer (positive = receives, negative = pays)
    if calc.vault_transfer != 0 {
        result.add_transfer(&trading.vault, calc.vault_transfer);
    }

    // Caller fee
    if calc.caller_fee > 0 {
        result.add_transfer(&trading.caller, calc.caller_fee);
    }

    storage::remove_user_position(e, &position.user, position.id);
    position.status = PositionStatus::Closed;

    market.update_stats(
        -position.collateral,
        -position.notional_size,
        position.is_long,
    );
    trading.cache_market(&market);
    trading.cache_position(position);

    (calc.price, calc.fee)
}

/// Fill a pending limit order
fn apply_fill(
    e: &Env,
    result: &mut ProcessingResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);

    let can_fill = if position.is_long {
        current_price <= position.entry_price
    } else {
        current_price >= position.entry_price
    };

    if !can_fill {
        return TradingError::LimitOrderNotFillable as u32;
    }

    position.status = PositionStatus::Open;
    position.entry_price = current_price;

    let mut market = trading.load_market(e, &position.asset);
    market.update_stats(
        position.collateral,
        position.notional_size,
        position.is_long,
    );

    // The base fee is in the current contract address, so we need to transfer it to the vault
    // and give the caller their fee
    let base_fee = position
        .collateral
        .fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7);
    let caller_fee = trading.calculate_caller_fee(e, base_fee);
    result.add_transfer(&trading.caller, caller_fee);
    result.add_transfer(&trading.vault, base_fee - caller_fee);

    trading.cache_market(&market);
    trading.cache_position(position);

    TradingEvents::fill_position(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
    );

    0
}

/// Trigger stop loss on a position
fn apply_stop_loss(
    e: &Env,
    result: &mut ProcessingResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);
    if !position.check_stop_loss(current_price) {
        return TradingError::StopLossNotTriggered as u32;
    }

    let (price, fee) = handle_close(e, result, trading, position);

    TradingEvents::stop_loss(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        price,
        fee,
    );

    0
}

/// Trigger take profit on a position
fn apply_take_profit(
    e: &Env,
    result: &mut ProcessingResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);
    if !position.check_take_profit(current_price) {
        return TradingError::TakeProfitNotTriggered as u32;
    }

    let (price, fee) = handle_close(e, result, trading, position);

    TradingEvents::take_profit(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        price,
        fee,
    );

    0
}

/// Liquidate an underwater position
fn apply_liquidation(
    e: &Env,
    result: &mut ProcessingResult,
    trading: &mut Trading,
    position: &mut Position,
) -> u32 {
    let current_price = trading.load_price(e, &position.asset);
    let mut market = trading.load_market(e, &position.asset);

    let pnl = position.calculate_pnl(e, current_price);
    let fee = position.calculate_fee(e, &market);

    let equity = position.collateral + pnl - fee;
    let required_margin =
        position
            .notional_size
            .fixed_mul_floor(e, &market.config.maintenance_margin, &SCALAR_7);

    if equity >= required_margin {
        return TradingError::PositionNotLiquidatable as u32;
    }

    let caller_fee = trading.calculate_caller_fee(e, fee);
    result.add_transfer(&trading.caller, caller_fee);

    let vault_amount = position.collateral - caller_fee;
    result.add_transfer(&trading.vault, vault_amount);

    TradingEvents::liquidation(
        e,
        position.user.clone(),
        position.asset.clone(),
        position.id,
        current_price,
        fee,
    );

    market.update_stats(
        -position.collateral,
        -position.notional_size,
        position.is_long,
    );

    position.status = PositionStatus::Closed;

    storage::remove_user_position(e, &position.user, position.id);
    trading.cache_market(&market);
    trading.cache_position(position);
    0
}
