use crate::constants::MAINTENANCE_MARGIN_DIVISOR;
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{FillLimit, Liquidation, StopLoss, TakeProfit};
use crate::storage;
use crate::trading::market::Market;
use crate::trading::oracle::{get_price_scalar, load_price};
use crate::trading::position::{FeeBreakdown, Position};
use crate::types::{ExecuteRequest, ExecuteRequestType, TradingConfig};
use crate::validation::{check_min_open_time, require_not_frozen, require_market_enabled};
use sep_40_oracle::Asset;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{map, vec, Address, Env, Map, Vec};

/// Context for batch execution operations (keeper triggers)
/// Caches markets and prices to minimize storage reads
pub struct ExecuteContext {
    pub config: TradingConfig,
    pub oracle: Address,
    pub vault: Address,
    pub token: Address,
    pub caller: Address,
    pub price_scalar: i128,
    pub token_scalar: i128,
    pub markets: Map<u32, Market>,
    pub markets_to_update: Vec<u32>,
    prices: Map<u32, i128>,
}

impl ExecuteContext {
    pub fn load(e: &Env, caller: Address) -> Self {
        let config = storage::get_config(e);
        let oracle = storage::get_oracle(e);
        let vault = storage::get_vault(e);
        let token = storage::get_token(e);
        let price_scalar = get_price_scalar(e, &oracle);
        let token_scalar = storage::get_token_scalar(e, &token);
        ExecuteContext {
            config,
            oracle,
            vault,
            token,
            caller,
            price_scalar,
            token_scalar,
            markets: map![e],
            markets_to_update: vec![e],
            prices: map![e],
        }
    }

    pub fn load_market(&mut self, e: &Env, asset_index: u32) -> Market {
        let mut market = if let Some(market) = self.markets.get(asset_index) {
            market
        } else {
            Market::load(e, asset_index)
        };
        market.accrue(e, self.config.vault_skim, self.token_scalar);
        market
    }

    pub fn cache_market(&mut self, market: &Market) {
        self.markets.set(market.asset_index, market.clone());
        if !self.markets_to_update.contains(&market.asset_index) {
            self.markets_to_update.push_back(market.asset_index);
        }
    }

    pub fn store_cached_markets(&mut self, e: &Env) {
        for asset_index in self.markets_to_update.iter() {
            let reserve = self.markets.get(asset_index).unwrap();
            reserve.store(e);
        }
    }

    pub fn get_price(&mut self, e: &Env, asset_index: u32, asset: &Asset) -> i128 {
        if let Some(price) = self.prices.get(asset_index) {
            return price;
        }
        let price = load_price(e, &self.oracle, asset);
        self.prices.set(asset_index, price);
        price
    }

    pub fn calculate_caller_fee(&self, e: &Env, fee: i128) -> i128 {
        let caller_fee = fee.fixed_mul_floor(e, &self.config.caller_take_rate, &self.token_scalar);
        if caller_fee > 0 { caller_fee } else { 0 }
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
    require_not_frozen(e);

    let mut ctx = ExecuteContext::load(e, caller.clone());
    let processing_result = process_execute_requests(e, &mut ctx, requests);

    let token_client = TokenClient::new(e, &ctx.token);
    let vault_client = VaultClient::new(e, &ctx.vault);

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
        // Vault receives: direct transfer from this contract to vault
        token_client.transfer(&e.current_contract_address(), &ctx.vault, &vault_transfer);
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
        let request_type = ExecuteRequestType::from_u32(e, request.request_type);
        let mut position = Position::load(e, request.position_id);

        // Validate position filled status for the requested action
        let (is_valid, specific_error) = match request_type {
            ExecuteRequestType::Fill => {
                if position.filled {
                    (false, TradingError::PositionNotPending as u32)
                } else {
                    (true, 0)
                }
            }
            ExecuteRequestType::StopLoss
            | ExecuteRequestType::TakeProfit
            | ExecuteRequestType::Liquidate => {
                if !position.filled {
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

        // Apply ADL reduction for filled positions before any PnL/margin calculations
        if position.filled {
            let market = ctx.load_market(e, position.asset_index);
            position.notional_size = position.effective_notional(e, &market);
        }

        let position_id = request.position_id;
        let action_result = match request_type {
            ExecuteRequestType::Fill => apply_fill(e, &mut result, ctx, &mut position, position_id),
            ExecuteRequestType::StopLoss => apply_stop_loss(e, &mut result, ctx, &mut position, position_id),
            ExecuteRequestType::TakeProfit => apply_take_profit(e, &mut result, ctx, &mut position, position_id),
            ExecuteRequestType::Liquidate => apply_liquidation(e, &mut result, ctx, &mut position, position_id),
        };

        result.results.push_back(action_result);
    }

    ctx.store_cached_markets(e);

    result
}

/// Handle position close logic shared by multiple actions
/// Returns (price, pnl, FeeBreakdown) for event emission
fn handle_close(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
    position_id: u32,
) -> (i128, i128, FeeBreakdown) {
    let mut market = ctx.load_market(e, position.asset_index);
    let price = ctx.get_price(e, position.asset_index, &market.config.asset);
    let pnl = position.calculate_pnl(e, price, ctx.price_scalar);
    let fees = position.calculate_fee_breakdown(e, &market, &ctx.config, ctx.token_scalar);

    // Calculate payouts
    let equity = position.collateral + pnl - fees.total_fee();
    let max_payout = position
        .collateral
        .fixed_mul_floor(e, &ctx.config.max_payout, &ctx.token_scalar);
    let user_payout = equity.max(0).min(max_payout);

    // Vault transfer (positive = receives, negative = pays)
    let mut vault_transfer = position.collateral - user_payout;

    // Caller fee from vault's portion (only when vault receives funds)
    let caller_fee = if vault_transfer > 0 {
        ctx.calculate_caller_fee(e, fees.total_fee()).min(vault_transfer)
    } else {
        0
    };
    vault_transfer -= caller_fee;

    // User receives their payout
    if user_payout > 0 {
        result.add_transfer(&position.user, user_payout);
    }

    // Vault transfer
    if vault_transfer != 0 {
        result.add_transfer(&ctx.vault, vault_transfer);
    }

    // Caller fee
    if caller_fee > 0 {
        result.add_transfer(&ctx.caller, caller_fee);
    }

    storage::remove_user_position(e, &position.user, position_id);
    storage::remove_position(e, position_id);

    market.update_stats(e, -position.notional_size, position.is_long, position.entry_price, ctx.price_scalar);
    market.update_funding_rate(e);
    ctx.cache_market(&market);

    (price, pnl, fees)
}

/// Fill a pending limit order
fn apply_fill(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
    position_id: u32,
) -> u32 {
    let mut market = ctx.load_market(e, position.asset_index);
    require_market_enabled(e, &market.config);
    let current_price = ctx.get_price(e, position.asset_index, &market.config.asset);

    let can_fill = if position.is_long {
        current_price <= position.entry_price
    } else {
        current_price >= position.entry_price
    };

    if !can_fill {
        return TradingError::LimitOrderNotFillable as u32;
    }

    position.filled = true;
    position.created_at = e.ledger().timestamp();
    position.entry_price = current_price;
    position.entry_funding_index = if position.is_long {
        market.data.long_funding_index
    } else {
        market.data.short_funding_index
    };
    position.entry_adl_index = if position.is_long {
        market.data.long_adl_index
    } else {
        market.data.short_adl_index
    };

    // Check if position would be dominant AFTER updating market stats
    let is_dominant = if position.is_long {
        let new_long = market.data.long_notional_size + position.notional_size;
        new_long > market.data.short_notional_size
    } else {
        let new_short = market.data.short_notional_size + position.notional_size;
        new_short > market.data.long_notional_size
    };

    market.update_stats(e, position.notional_size, position.is_long, position.entry_price, ctx.price_scalar);
    market.update_funding_rate(e);

    // Fee charged upfront on create was base_fee_dominant (worst case for limit orders)
    let fee_dominant = position
        .notional_size
        .fixed_mul_ceil(e, &ctx.config.base_fee_dominant, &ctx.token_scalar);
    let fee_non_dominant = position
        .notional_size
        .fixed_mul_ceil(e, &ctx.config.base_fee_non_dominant, &ctx.token_scalar);
    let price_impact = position
        .notional_size
        .fixed_div_ceil(e, &market.config.price_impact_scalar, &ctx.token_scalar);

    let actual_base_fee = if is_dominant {
        // Imbalancing: vault gets fee_dominant + impact - caller_fee
        let total_fee = fee_dominant + price_impact;
        let caller_fee = ctx.calculate_caller_fee(e, total_fee);
        result.add_transfer(&ctx.caller, caller_fee);
        result.add_transfer(&ctx.vault, total_fee - caller_fee);
        fee_dominant
    } else {
        // Balancing: refund difference (fee_dominant - fee_non_dominant) to user
        let refund = fee_dominant - fee_non_dominant;
        if refund > 0 {
            result.add_transfer(&position.user, refund);
        }
        let total_fee = fee_non_dominant + price_impact;
        let caller_fee = ctx.calculate_caller_fee(e, total_fee);
        result.add_transfer(&ctx.caller, caller_fee);
        result.add_transfer(&ctx.vault, total_fee - caller_fee);
        fee_non_dominant
    };

    ctx.cache_market(&market);
    position.store(e, position_id);

    FillLimit {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id,
        base_fee: actual_base_fee,
        impact_fee: price_impact,
    }
    .publish(e);

    0
}

/// Trigger stop loss on a position
fn apply_stop_loss(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
    position_id: u32,
) -> u32 {
    if !check_min_open_time(e, position, ctx.config.min_open_time) {
        return TradingError::PositionTooNew as u32;
    }
    let market = ctx.load_market(e, position.asset_index);
    let current_price = ctx.get_price(e, position.asset_index, &market.config.asset);
    if !position.check_stop_loss(current_price) {
        return TradingError::StopLossNotTriggered as u32;
    }

    let (price, pnl, fees) = handle_close(e, result, ctx, position, position_id);

    StopLoss {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id,
        price,
        pnl,
        base_fee: fees.base_fee,
        impact_fee: fees.impact_fee,
        funding: fees.funding,
    }
    .publish(e);

    0
}

/// Trigger take profit on a position
fn apply_take_profit(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
    position_id: u32,
) -> u32 {
    if !check_min_open_time(e, position, ctx.config.min_open_time) {
        return TradingError::PositionTooNew as u32;
    }
    let market = ctx.load_market(e, position.asset_index);
    let current_price = ctx.get_price(e, position.asset_index, &market.config.asset);
    if !position.check_take_profit(current_price) {
        return TradingError::TakeProfitNotTriggered as u32;
    }

    let (price, pnl, fees) = handle_close(e, result, ctx, position, position_id);

    TakeProfit {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id,
        price,
        pnl,
        base_fee: fees.base_fee,
        impact_fee: fees.impact_fee,
        funding: fees.funding,
    }
    .publish(e);

    0
}

/// Liquidate an underwater position
fn apply_liquidation(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
    position_id: u32,
) -> u32 {
    let mut market = ctx.load_market(e, position.asset_index);
    let current_price = ctx.get_price(e, position.asset_index, &market.config.asset);

    let pnl = position.calculate_pnl(e, current_price, ctx.price_scalar);
    let fees = position.calculate_fee_breakdown(e, &market, &ctx.config, ctx.token_scalar);
    let equity = position.collateral + pnl - fees.total_fee();
    let maintenance_margin = ctx.token_scalar / MAINTENANCE_MARGIN_DIVISOR;
    let required_margin =
        position
            .notional_size
            .fixed_mul_floor(e, &maintenance_margin, &ctx.token_scalar);

    if equity >= required_margin {
        return TradingError::PositionNotLiquidatable as u32;
    }

    let caller_fee = ctx.calculate_caller_fee(e, fees.total_fee());
    result.add_transfer(&ctx.caller, caller_fee);

    let vault_amount = position.collateral - caller_fee;
    result.add_transfer(&ctx.vault, vault_amount);

    Liquidation {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id,
        price: current_price,
        pnl,
        base_fee: fees.base_fee,
        impact_fee: fees.impact_fee,
        funding: fees.funding,
    }
    .publish(e);

    market.update_stats(e, -position.notional_size, position.is_long, position.entry_price, ctx.price_scalar);
    market.update_funding_rate(e);

    storage::remove_user_position(e, &position.user, position_id);
    storage::remove_position(e, position_id);
    ctx.cache_market(&market);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{SCALAR_18, SCALAR_7};
    use crate::testutils::{default_market_data, setup_contract, setup_env, BTC_PRICE};
    use crate::types::ExecuteRequest;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{vec};

    fn create_test_position(
        e: &Env,
        user: &Address,
        filled: bool,
        is_long: bool,
        entry_price: i128,
    ) -> crate::types::Position {
        crate::types::Position {
            user: user.clone(),
            filled,
            asset_index: 0,
            is_long,
            stop_loss: 0,
            take_profit: 0,
            entry_price,
            collateral: 1_000 * SCALAR_7,
            notional_size: 10_000 * SCALAR_7,
            entry_funding_index: SCALAR_18,
            created_at: e.ledger().timestamp(),
            entry_adl_index: SCALAR_18,
        }
    }

    // ==========================================
    // apply_fill Tests
    // ==========================================

    #[test]
    fn test_apply_fill_long_success() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Create pending long position with entry above current price
            let mut position =
                create_test_position(&e, &user, false, true, BTC_PRICE + 1000 * SCALAR_7);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_fill(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, 0);
            assert!(position.filled);
            assert_eq!(position.entry_price, BTC_PRICE);
        });
    }

    #[test]
    fn test_apply_fill_short_success() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Create pending short position with entry below current price
            let mut position =
                create_test_position(&e, &user, false, false, BTC_PRICE - 1000 * SCALAR_7);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_fill(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, 0);
            assert!(position.filled);
        });
    }

    #[test]
    fn test_apply_fill_long_not_fillable() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Entry price below current - not fillable for long
            let mut position =
                create_test_position(&e, &user, false, true, BTC_PRICE - 1000 * SCALAR_7);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_fill(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, TradingError::LimitOrderNotFillable as u32);
            assert!(!position.filled);
        });
    }

    #[test]
    fn test_apply_fill_short_not_fillable() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Entry price above current - not fillable for short
            let mut position =
                create_test_position(&e, &user, false, false, BTC_PRICE + 1000 * SCALAR_7);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_fill(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, TradingError::LimitOrderNotFillable as u32);
        });
    }

    #[test]
    fn test_apply_fill_balancing_trade() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Setup market with short dominant
            let mut market_data = default_market_data();
            market_data.long_notional_size = 100_000 * SCALAR_7;
            market_data.short_notional_size = 200_000 * SCALAR_7;
            market_data.last_update = e.ledger().timestamp();
            market_data.long_funding_index = SCALAR_18;
            market_data.short_funding_index = SCALAR_18;
            storage::set_market_data(&e, 0, &market_data);

            // Long order will be balancing
            let mut position =
                create_test_position(&e, &user, false, true, BTC_PRICE + 1000 * SCALAR_7);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller.clone());
            let mut result = ProcessingResult::new(&e);

            let code = apply_fill(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, 0);

            // User should receive base_fee refund for balancing
            assert!(result.transfers.get(user).unwrap_or(0) > 0);
        });
    }

    // ==========================================
    // apply_stop_loss Tests
    // ==========================================

    #[test]
    fn test_apply_stop_loss_long_triggered() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Create filled long with SL above current price
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.stop_loss = BTC_PRICE + 1000 * SCALAR_7; // SL triggers when price <= this
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_stop_loss(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, 0);
        });
    }

    #[test]
    fn test_apply_stop_loss_not_triggered() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Long with SL way below current price
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.stop_loss = BTC_PRICE - 50000 * SCALAR_7;
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_stop_loss(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, TradingError::StopLossNotTriggered as u32);
        });
    }

    #[test]
    fn test_apply_stop_loss_short_triggered() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Short with SL below current price (triggers when price >= SL)
            let mut position = create_test_position(&e, &user, true, false, BTC_PRICE);
            position.stop_loss = BTC_PRICE - 1000 * SCALAR_7;
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_stop_loss(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, 0);
        });
    }

    // ==========================================
    // apply_take_profit Tests
    // ==========================================

    #[test]
    fn test_apply_take_profit_long_triggered() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Long with TP at current price (triggers when price >= TP)
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.take_profit = BTC_PRICE;
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_take_profit(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, 0);
        });
    }

    #[test]
    fn test_apply_take_profit_not_triggered() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Long with TP way above current price
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.take_profit = BTC_PRICE + 50000 * SCALAR_7;
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_take_profit(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, TradingError::TakeProfitNotTriggered as u32);
        });
    }

    #[test]
    fn test_apply_take_profit_short_triggered() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Short with TP above current price (triggers when price <= TP)
            let mut position = create_test_position(&e, &user, true, false, BTC_PRICE);
            position.take_profit = BTC_PRICE + 1000 * SCALAR_7;
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_take_profit(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, 0);
        });
    }

    // ==========================================
    // apply_liquidation Tests
    // ==========================================

    #[test]
    fn test_apply_liquidation_underwater() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Create position that's underwater
            // Entry at 100k, current at 100k, but with high interest
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.collateral = 100 * SCALAR_7; // Very small collateral
            position.notional_size = 10_000 * SCALAR_7; // 100x leverage
            position.entry_funding_index = SCALAR_18;

            // Set high interest index so position is underwater
            let mut market_data = default_market_data();
            market_data.long_funding_index = SCALAR_18 + SCALAR_18 / 10; // 10% interest accrued
            market_data.short_funding_index = SCALAR_18;
            market_data.last_update = e.ledger().timestamp();
            storage::set_market_data(&e, 0, &market_data);

            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut ctx = ExecuteContext::load(&e, caller.clone());
            let mut result = ProcessingResult::new(&e);

            let code = apply_liquidation(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, 0);

            // Caller should receive fee
            assert!(result.transfers.get(caller).unwrap_or(0) > 0);
        });
    }

    #[test]
    fn test_apply_liquidation_not_liquidatable() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Healthy position with plenty of margin
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.collateral = 5_000 * SCALAR_7;
            position.notional_size = 10_000 * SCALAR_7; // 2x leverage
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_liquidation(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, TradingError::PositionNotLiquidatable as u32);
        });
    }

    // ==========================================
    // process_execute_requests Tests
    // ==========================================

    #[test]
    fn test_process_filled_position_error() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Create already filled position
            let position = create_test_position(&e, &user, true, true, BTC_PRICE);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: 1,
                }
            ];

            let result = process_execute_requests(&e, &mut ctx, requests);
            assert_eq!(result.results.len(), 1);
            assert_eq!(
                result.results.get(0),
                Some(TradingError::PositionNotPending as u32)
            );
        });
    }

    #[test]
    fn test_process_pending_position_error() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Create pending (not filled) position
            let position = create_test_position(&e, &user, false, true, BTC_PRICE);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);

            // Try to trigger SL on unfilled position
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::StopLoss as u32,
                    position_id: 1,
                }
            ];

            let result = process_execute_requests(&e, &mut ctx, requests);
            assert_eq!(result.results.len(), 1);
            assert_eq!(
                result.results.get(0),
                Some(TradingError::ActionNotAllowedForStatus as u32)
            );
        });
    }

    #[test]
    fn test_process_multiple_requests() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Create positions
            let pos1 = create_test_position(&e, &user, false, true, BTC_PRICE + 1000 * SCALAR_7);
            storage::set_position(&e, 1, &pos1);
            storage::add_user_position(&e, &user, 1);

            let mut pos2 = create_test_position(&e, &user, true, true, BTC_PRICE);
            pos2.take_profit = BTC_PRICE;
            storage::set_position(&e, 2, &pos2);
            storage::add_user_position(&e, &user, 2);

            let mut ctx = ExecuteContext::load(&e, caller);
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: 1,
                },
                ExecuteRequest {
                    request_type: ExecuteRequestType::TakeProfit as u32,
                    position_id: 2,
                }
            ];

            let result = process_execute_requests(&e, &mut ctx, requests);
            assert_eq!(result.results.len(), 2);
            assert_eq!(result.results.get(0), Some(0)); // Fill success
            assert_eq!(result.results.get(1), Some(0)); // TP success
        });
    }

    // ==========================================
    // handle_close Tests
    // ==========================================

    #[test]
    fn test_handle_close_profitable() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Entry at price below current (profitable long)
            let mut position =
                create_test_position(&e, &user, true, true, BTC_PRICE - 10000 * SCALAR_7);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            // Update market data timestamp
            let mut market_data = default_market_data();
            market_data.last_update = e.ledger().timestamp();
            market_data.long_funding_index = SCALAR_18;
            market_data.short_funding_index = SCALAR_18;
            storage::set_market_data(&e, 0, &market_data);

            let mut ctx = ExecuteContext::load(&e, caller.clone());
            let mut result = ProcessingResult::new(&e);

            let (price, pnl, _fees) = handle_close(&e, &mut result, &mut ctx, &mut position, 1);

            assert_eq!(price, BTC_PRICE);
            assert!(pnl > 0);
            // User should receive payout
            assert!(result.transfers.get(user).unwrap_or(0) > 0);
        });
    }

    #[test]
    fn test_handle_close_loss() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Entry above current (losing long)
            let mut position =
                create_test_position(&e, &user, true, true, BTC_PRICE + 10000 * SCALAR_7);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut ctx = ExecuteContext::load(&e, caller.clone());
            let mut result = ProcessingResult::new(&e);

            let (price, pnl, _fees) = handle_close(&e, &mut result, &mut ctx, &mut position, 1);

            assert_eq!(price, BTC_PRICE);
            assert!(pnl < 0);
        });
    }

    // ==========================================
    // execute_trigger Tests (full flow with token transfers)
    // ==========================================

    /// Helper: set up market data with proper interest indices and timestamp
    fn setup_market_data(e: &Env) {
        let mut data = default_market_data();
        data.long_funding_index = SCALAR_18;
        data.short_funding_index = SCALAR_18;
        data.last_update = e.ledger().timestamp();
        storage::set_market_data(e, 0, &data);
    }

    #[test]
    fn test_execute_trigger_fill_limit() {
        let e = setup_env();
        let (address, token_client) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            setup_market_data(&e);

            // Pending long with entry above current → fillable
            let position =
                create_test_position(&e, &user, false, true, BTC_PRICE + 1000 * SCALAR_7);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let vault = storage::get_vault(&e);
            let vault_bal_before = token_client.balance(&vault);
            let caller_bal_before = token_client.balance(&caller);

            let results = execute_trigger(
                &e,
                &caller,
                vec![
                    &e,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::Fill as u32,
                        position_id: 1,
                    },
                ],
            );

            assert_eq!(results.get(0), Some(0)); // success

            // Vault should receive fees
            let vault_bal_after = token_client.balance(&vault);
            assert!(vault_bal_after > vault_bal_before);

            // Caller should receive caller_take_rate portion
            let caller_bal_after = token_client.balance(&caller);
            assert!(caller_bal_after > caller_bal_before);

            // Position should now be filled
            let pos = storage::get_position(&e, 1);
            assert!(pos.filled);
            assert_eq!(pos.entry_price, BTC_PRICE);
        });
    }

    #[test]
    fn test_execute_trigger_take_profit_profitable() {
        let e = setup_env();
        let (address, token_client) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            setup_market_data(&e);

            // Profitable long: entry at 90k, current at 100k
            // collateral=1000, notional=10,000, PnL ≈ +1,111
            // user_payout > collateral → vault must pay out via strategy_withdraw
            let mut position =
                create_test_position(&e, &user, true, true, BTC_PRICE - 10_000 * SCALAR_7);
            position.take_profit = BTC_PRICE; // TP at current price
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let vault = storage::get_vault(&e);
            let vault_bal_before = token_client.balance(&vault);
            let user_bal_before = token_client.balance(&user);

            let results = execute_trigger(
                &e,
                &caller,
                vec![
                    &e,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::TakeProfit as u32,
                        position_id: 1,
                    },
                ],
            );

            assert_eq!(results.get(0), Some(0));

            // Vault balance should decrease (strategy_withdraw was called)
            let vault_bal_after = token_client.balance(&vault);
            assert!(vault_bal_after < vault_bal_before);

            // User should receive payout > collateral (profitable)
            let user_bal_after = token_client.balance(&user);
            assert!(user_bal_after > user_bal_before);
            assert!(user_bal_after - user_bal_before > position.collateral);
        });
    }

    #[test]
    fn test_execute_trigger_stop_loss_loss() {
        let e = setup_env();
        let (address, token_client) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            setup_market_data(&e);

            // Losing long: entry at 100k, SL triggers, price still at 100k but fees eat into collateral
            // SL above current price triggers for long
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.stop_loss = BTC_PRICE + 1000 * SCALAR_7;
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let vault = storage::get_vault(&e);
            let vault_bal_before = token_client.balance(&vault);

            let results = execute_trigger(
                &e,
                &caller,
                vec![
                    &e,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::StopLoss as u32,
                        position_id: 1,
                    },
                ],
            );

            assert_eq!(results.get(0), Some(0));

            // Vault should receive funds (fees from the position)
            let vault_bal_after = token_client.balance(&vault);
            assert!(vault_bal_after > vault_bal_before);
        });
    }

    #[test]
    fn test_execute_trigger_liquidation() {
        let e = setup_env();
        let (address, token_client) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Underwater position: small collateral, moderate interest
            // collateral=100, notional=10,000 (100x)
            // maintenance_margin=0.5% → required_margin=50
            // 1% interest on 10,000 = 100 in fees → equity = 100 - ~105 = -5 < 50
            // caller_fee = 10% of ~105 ≈ 10.5, vault_amount = 100 - 10.5 ≈ 89.5
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.collateral = 100 * SCALAR_7;
            position.notional_size = 10_000 * SCALAR_7;
            position.entry_funding_index = SCALAR_18;

            let mut data = default_market_data();
            data.long_funding_index = SCALAR_18 + SCALAR_18 / 100; // 1% interest
            data.short_funding_index = SCALAR_18;
            data.last_update = e.ledger().timestamp();
            storage::set_market_data(&e, 0, &data);

            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let vault = storage::get_vault(&e);
            let vault_bal_before = token_client.balance(&vault);
            let caller_bal_before = token_client.balance(&caller);

            let results = execute_trigger(
                &e,
                &caller,
                vec![
                    &e,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::Liquidate as u32,
                        position_id: 1,
                    },
                ],
            );

            assert_eq!(results.get(0), Some(0));

            // Vault receives remaining collateral (minus caller fee)
            let vault_bal_after = token_client.balance(&vault);
            assert!(vault_bal_after > vault_bal_before);

            // Caller receives liquidation fee
            let caller_bal_after = token_client.balance(&caller);
            assert!(caller_bal_after > caller_bal_before);
        });
    }

    #[test]
    fn test_execute_trigger_batch_mixed() {
        let e = setup_env();
        let (address, token_client) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            setup_market_data(&e);

            // Position 1: fillable limit (pending long, entry above current)
            let pos1 =
                create_test_position(&e, &user, false, true, BTC_PRICE + 1000 * SCALAR_7);
            storage::set_position(&e, 1, &pos1);
            storage::add_user_position(&e, &user, 1);

            // Position 2: profitable TP (filled long, entry below current)
            let mut pos2 =
                create_test_position(&e, &user, true, true, BTC_PRICE - 10_000 * SCALAR_7);
            pos2.take_profit = BTC_PRICE;
            storage::set_position(&e, 2, &pos2);
            storage::add_user_position(&e, &user, 2);

            // Position 3: not liquidatable (healthy) → should return error
            let mut pos3 = create_test_position(&e, &user, true, true, BTC_PRICE);
            pos3.collateral = 5_000 * SCALAR_7;
            pos3.notional_size = 10_000 * SCALAR_7;
            storage::set_position(&e, 3, &pos3);
            storage::add_user_position(&e, &user, 3);

            let user_bal_before = token_client.balance(&user);

            let results = execute_trigger(
                &e,
                &caller,
                vec![
                    &e,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::Fill as u32,
                        position_id: 1,
                    },
                    ExecuteRequest {
                        request_type: ExecuteRequestType::TakeProfit as u32,
                        position_id: 2,
                    },
                    ExecuteRequest {
                        request_type: ExecuteRequestType::Liquidate as u32,
                        position_id: 3,
                    },
                ],
            );

            assert_eq!(results.len(), 3);
            assert_eq!(results.get(0), Some(0)); // Fill success
            assert_eq!(results.get(1), Some(0)); // TP success
            assert_eq!(
                results.get(2),
                Some(TradingError::PositionNotLiquidatable as u32)
            ); // Liquidation fails

            // User should receive payout from the profitable TP
            let user_bal_after = token_client.balance(&user);
            assert!(user_bal_after > user_bal_before);
        });
    }

    #[test]
    fn test_execute_trigger_error_fill_already_filled() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            setup_market_data(&e);

            let position = create_test_position(&e, &user, true, true, BTC_PRICE);
            storage::set_position(&e, 1, &position);

            let results = execute_trigger(
                &e,
                &caller,
                vec![
                    &e,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::Fill as u32,
                        position_id: 1,
                    },
                ],
            );

            assert_eq!(
                results.get(0),
                Some(TradingError::PositionNotPending as u32)
            );
        });
    }

    #[test]
    fn test_execute_trigger_error_sl_on_pending() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            setup_market_data(&e);

            let position = create_test_position(&e, &user, false, true, BTC_PRICE);
            storage::set_position(&e, 1, &position);

            let results = execute_trigger(
                &e,
                &caller,
                vec![
                    &e,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::StopLoss as u32,
                        position_id: 1,
                    },
                ],
            );

            assert_eq!(
                results.get(0),
                Some(TradingError::ActionNotAllowedForStatus as u32)
            );
        });
    }

    #[test]
    fn test_execute_trigger_error_tp_not_triggered() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            setup_market_data(&e);

            // Long with TP way above current price → not triggered
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.take_profit = BTC_PRICE + 50_000 * SCALAR_7;
            storage::set_position(&e, 1, &position);

            let results = execute_trigger(
                &e,
                &caller,
                vec![
                    &e,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::TakeProfit as u32,
                        position_id: 1,
                    },
                ],
            );

            assert_eq!(
                results.get(0),
                Some(TradingError::TakeProfitNotTriggered as u32)
            );
        });
    }

    #[test]
    fn test_execute_trigger_error_sl_not_triggered() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            setup_market_data(&e);

            // Long with SL way below current price → not triggered
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.stop_loss = BTC_PRICE - 50_000 * SCALAR_7;
            storage::set_position(&e, 1, &position);

            let results = execute_trigger(
                &e,
                &caller,
                vec![
                    &e,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::StopLoss as u32,
                        position_id: 1,
                    },
                ],
            );

            assert_eq!(
                results.get(0),
                Some(TradingError::StopLossNotTriggered as u32)
            );
        });
    }

    #[test]
    fn test_execute_trigger_error_not_liquidatable() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            setup_market_data(&e);

            // Healthy position
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.collateral = 5_000 * SCALAR_7;
            position.notional_size = 10_000 * SCALAR_7;
            storage::set_position(&e, 1, &position);

            let results = execute_trigger(
                &e,
                &caller,
                vec![
                    &e,
                    ExecuteRequest {
                        request_type: ExecuteRequestType::Liquidate as u32,
                        position_id: 1,
                    },
                ],
            );

            assert_eq!(
                results.get(0),
                Some(TradingError::PositionNotLiquidatable as u32)
            );
        });
    }

    // ==========================================
    // min_open_time Tests
    // ==========================================

    #[test]
    fn test_stop_loss_blocked_by_min_open_time() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            let mut config = storage::get_config(&e);
            config.min_open_time = 60;
            storage::set_config(&e, &config);

            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.stop_loss = BTC_PRICE + 1000 * SCALAR_7;
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_stop_loss(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, TradingError::PositionTooNew as u32);
        });
    }

    #[test]
    fn test_take_profit_blocked_by_min_open_time() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            let mut config = storage::get_config(&e);
            config.min_open_time = 60;
            storage::set_config(&e, &config);

            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.take_profit = BTC_PRICE;
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_take_profit(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, TradingError::PositionTooNew as u32);
        });
    }

    #[test]
    fn test_liquidation_ignores_min_open_time() {
        let e = setup_env();
        let (address, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            let mut config = storage::get_config(&e);
            config.min_open_time = 60;
            storage::set_config(&e, &config);

            // Create underwater position
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.collateral = 100 * SCALAR_7;
            position.notional_size = 10_000 * SCALAR_7;
            position.entry_funding_index = SCALAR_18;

            let mut market_data = default_market_data();
            market_data.long_funding_index = SCALAR_18 + SCALAR_18 / 10;
            market_data.short_funding_index = SCALAR_18;
            market_data.last_update = e.ledger().timestamp();
            storage::set_market_data(&e, 0, &market_data);

            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            // Liquidation should succeed immediately despite min_open_time
            let code = apply_liquidation(&e, &mut result, &mut ctx, &mut position, 1);
            assert_eq!(code, 0);
        });
    }
}

