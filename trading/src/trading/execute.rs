use crate::constants::{MAX_PRICE_AGE, SCALAR_7, STATUS_ACTIVE, STATUS_ON_ICE};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{emit_fill_position, emit_liquidation, emit_stop_loss, emit_take_profit};
use crate::storage;
use crate::trading::market::Market;
use crate::trading::position::Position;
use crate::types::{ExecuteRequest, ExecuteRequestType, PositionStatus, TradingConfig};
use sep_40_oracle::{Asset, PriceFeedClient};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::token::TokenClient;
use soroban_sdk::{map, panic_with_error, vec, Address, Env, IntoVal, Map, Symbol, Val, Vec};

/// Context for batch execution operations (keeper triggers)
/// Caches markets, positions, and prices to minimize storage reads
pub struct ExecuteContext {
    pub config: TradingConfig,
    pub vault: Address,
    pub token: Address,
    pub caller: Address,
    pub markets: Map<Asset, Market>,
    pub positions: Map<u32, Position>,
    pub positions_to_update: Vec<u32>,
    pub markets_to_update: Vec<Asset>,
    prices: Map<Asset, i128>,
}

impl ExecuteContext {
    pub fn load(e: &Env, caller: Address) -> Self {
        if !storage::has_name(e) {
            panic_with_error!(e, TradingError::NotInitialized);
        }
        let config = storage::get_config(e);
        let vault = storage::get_vault(e);
        let token = storage::get_token(e);
        ExecuteContext {
            config,
            vault,
            token,
            caller,
            markets: map![e],
            positions: map![e],
            positions_to_update: vec![e],
            markets_to_update: vec![e],
            prices: map![e],
        }
    }

    pub fn load_market(&mut self, e: &Env, asset: &Asset) -> Market {
        let mut market = if let Some(market) = self.markets.get(asset.clone()) {
            market
        } else {
            Market::load_checked(e, asset) // panics if not found or disabled
        };
        market.update_borrowing_index(e);
        market
    }

    pub fn load_position(&mut self, e: &Env, position_id: u32) -> Position {
        if let Some(position) = self.positions.get(position_id) {
            position
        } else {
            Position::load(e, position_id) // panics if not found
        }
    }

    pub fn cache_market(&mut self, market: &Market) {
        self.markets.set(market.asset.clone(), market.clone());
        if !self.markets_to_update.contains(&market.asset) {
            self.markets_to_update.push_back(market.asset.clone());
        }
    }

    pub fn cache_position(&mut self, position: &Position) {
        self.positions.set(position.id, position.clone());
        if !self.positions_to_update.contains(position.id) {
            self.positions_to_update.push_back(position.id);
        }
    }

    pub fn store_cached_markets(&mut self, e: &Env) {
        for asset in self.markets_to_update.iter() {
            let reserve = self.markets.get(asset).unwrap();
            reserve.store(e);
        }
    }

    pub fn store_cached_positions(&mut self, e: &Env) {
        for position_id in self.positions_to_update.iter() {
            let position = self.positions.get(position_id).unwrap();
            position.store(e);
        }
    }

    pub fn load_price(&mut self, e: &Env, asset: &Asset) -> i128 {
        if let Some(price) = self.prices.get(asset.clone()) {
            return price;
        }
        let price_data = match PriceFeedClient::new(e, &self.config.oracle).lastprice(asset) {
            Some(price) => price,
            None => panic_with_error!(e, TradingError::PriceNotFound),
        };
        if price_data.timestamp + MAX_PRICE_AGE < e.ledger().timestamp() {
            panic_with_error!(e, TradingError::PriceStale);
        }

        self.prices.set(asset.clone(), price_data.price);
        price_data.price
    }

    pub fn calculate_caller_fee(&self, e: &Env, fee: i128) -> i128 {
        let caller_fee = fee.fixed_mul_floor(e, &self.config.caller_take_rate, &SCALAR_7);
        if caller_fee > 0 { caller_fee } else { 0 }
    }

    pub fn check_max_positions(&self, positions: Vec<u32>) -> bool {
        positions.len() < self.config.max_positions
    }
}

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
    // Allow keeper actions in Active and OnIce
    let status = storage::get_status(e);
    if status != STATUS_ACTIVE && status != STATUS_ON_ICE {
        panic_with_error!(e, TradingError::ContractPaused);
    }

    let mut ctx = ExecuteContext::load(e, caller.clone());
    let processing_result = process_execute_requests(e, &mut ctx, requests);

    let token_client = TokenClient::new(e, &storage::get_token(e));
    let vault_client = VaultClient::new(e, &storage::get_vault(e));

    // STEP 1: Vault pays to contract (if needed)
    // This is done first to ensure the contract has enough balance to handle transfers
    let vault_transfer = processing_result.transfers.get(ctx.vault.clone()).unwrap_or(0);
    if vault_transfer < 0 {
        // Vault pays: withdraw from vault to this contract
        vault_client.strategy_withdraw(&e.current_contract_address(), &vault_transfer.abs());
    }

    // STEP 2: Handle all other transfers (callers receiving fees, users receiving payouts)
    for (address, amount) in processing_result.transfers.iter() {
        if address != ctx.vault {
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
    ctx: &mut ExecuteContext,
    requests: Vec<ExecuteRequest>,
) -> ProcessingResult {
    let mut result = ProcessingResult::new(e);

    for request in requests.iter() {
        let mut position = ctx.load_position(e, request.position_id);

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
            ExecuteRequestType::Fill => apply_fill(e, &mut result, ctx, &mut position),
            ExecuteRequestType::StopLoss => apply_stop_loss(e, &mut result, ctx, &mut position),
            ExecuteRequestType::TakeProfit => apply_take_profit(e, &mut result, ctx, &mut position),
            ExecuteRequestType::Liquidate => apply_liquidation(e, &mut result, ctx, &mut position),
        };

        result.results.push_back(action_result);
    }

    ctx.store_cached_markets(e);
    ctx.store_cached_positions(e);

    result
}

/// Handle position close logic shared by multiple actions
/// Uses shared calculate_close for PnL/fee calculation
fn handle_close(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
) -> (i128, i128) {
    let mut market = ctx.load_market(e, &position.asset);
    let calc = position.calculate_close(e, ctx, &market);

    // User receives their payout (if positive)
    if calc.user_payout > 0 {
        result.add_transfer(&position.user, calc.user_payout);
    }

    // Vault transfer (positive = receives, negative = pays)
    if calc.vault_transfer != 0 {
        result.add_transfer(&ctx.vault, calc.vault_transfer);
    }

    // Caller fee
    if calc.caller_fee > 0 {
        result.add_transfer(&ctx.caller, calc.caller_fee);
    }

    storage::remove_user_position(e, &position.user, position.id);
    position.status = PositionStatus::Closed;

    market.update_stats(
        -position.collateral,
        -position.notional_size,
        position.is_long,
    );
    ctx.cache_market(&market);
    ctx.cache_position(position);

    (calc.price, calc.fee)
}

/// Fill a pending limit order
fn apply_fill(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
) -> u32 {
    let current_price = ctx.load_price(e, &position.asset);

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

    let mut market = ctx.load_market(e, &position.asset);

    // Check if position is balancing BEFORE updating market stats
    let should_pay_base_fee = if position.is_long {
        let new_long = market.data.long_notional_size + position.notional_size;
        new_long > market.data.short_notional_size
    } else {
        let new_short = market.data.short_notional_size + position.notional_size;
        new_short > market.data.long_notional_size
    };

    market.update_stats(
        position.collateral,
        position.notional_size,
        position.is_long,
    );

    // Calculate fees from notional_size (same formula as open)
    let base_fee = position
        .notional_size
        .fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7);
    let price_impact = position
        .notional_size
        .fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

    if should_pay_base_fee {
        // Position increases imbalance: send fees to vault (minus caller fee)
        let total_fee = base_fee + price_impact;
        let caller_fee = ctx.calculate_caller_fee(e, total_fee);
        result.add_transfer(&ctx.caller, caller_fee);
        result.add_transfer(&ctx.vault, total_fee - caller_fee);
    } else {
        // Position is balancing: refund base_fee to user, only price_impact goes to vault
        result.add_transfer(&position.user, base_fee);
        let caller_fee = ctx.calculate_caller_fee(e, price_impact);
        result.add_transfer(&ctx.caller, caller_fee);
        result.add_transfer(&ctx.vault, price_impact - caller_fee);
    }

    ctx.cache_market(&market);
    ctx.cache_position(position);

    emit_fill_position(
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
    ctx: &mut ExecuteContext,
    position: &mut Position,
) -> u32 {
    let current_price = ctx.load_price(e, &position.asset);
    if !position.check_stop_loss(current_price) {
        return TradingError::StopLossNotTriggered as u32;
    }

    let (price, fee) = handle_close(e, result, ctx, position);

    emit_stop_loss(
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
    ctx: &mut ExecuteContext,
    position: &mut Position,
) -> u32 {
    let current_price = ctx.load_price(e, &position.asset);
    if !position.check_take_profit(current_price) {
        return TradingError::TakeProfitNotTriggered as u32;
    }

    let (price, fee) = handle_close(e, result, ctx, position);

    emit_take_profit(
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
    ctx: &mut ExecuteContext,
    position: &mut Position,
) -> u32 {
    let current_price = ctx.load_price(e, &position.asset);
    let mut market = ctx.load_market(e, &position.asset);

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

    let caller_fee = ctx.calculate_caller_fee(e, fee);
    result.add_transfer(&ctx.caller, caller_fee);

    let vault_amount = position.collateral - caller_fee;
    result.add_transfer(&ctx.vault, vault_amount);

    emit_liquidation(
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
    ctx.cache_market(&market);
    ctx.cache_position(position);
    0
}
