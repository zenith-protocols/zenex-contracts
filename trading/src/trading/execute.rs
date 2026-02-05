use crate::constants::SCALAR_7;
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{FillLimit, Liquidation, StopLoss, TakeProfit};
use crate::storage;
use crate::trading::market::{load_price, Market};
use crate::trading::position::{FeeBreakdown, Position};
use crate::types::{ExecuteRequest, ExecuteRequestType, TradingConfig};
use crate::validation::{require_not_frozen, require_market_enabled};
use sep_40_oracle::Asset;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{map, panic_with_error, vec, Address, Env, Map, Vec};

/// Context for batch execution operations (keeper triggers)
/// Caches markets, positions, and prices to minimize storage reads
pub struct ExecuteContext {
    pub config: TradingConfig,
    pub vault: Address,
    pub caller: Address,
    pub markets: Map<u32, Market>,
    pub positions: Map<u32, Position>,
    pub positions_to_update: Vec<u32>,
    pub markets_to_update: Vec<u32>,
    prices: Map<u32, i128>,
}

impl ExecuteContext {
    pub fn load(e: &Env, caller: Address) -> Self {
        if !storage::has_name(e) {
            panic_with_error!(e, TradingError::NotInitialized);
        }
        let config = storage::get_config(e);
        let vault = storage::get_vault(e);
        ExecuteContext {
            config,
            vault,
            caller,
            markets: map![e],
            positions: map![e],
            positions_to_update: vec![e],
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
        market.accrue_interest(e);
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
        self.markets.set(market.asset_index, market.clone());
        if !self.markets_to_update.contains(&market.asset_index) {
            self.markets_to_update.push_back(market.asset_index);
        }
    }

    pub fn cache_position(&mut self, position: &Position) {
        self.positions.set(position.id, position.clone());
        if !self.positions_to_update.contains(position.id) {
            self.positions_to_update.push_back(position.id);
        }
    }

    pub fn store_cached_markets(&mut self, e: &Env) {
        for asset_index in self.markets_to_update.iter() {
            let reserve = self.markets.get(asset_index).unwrap();
            reserve.store(e);
        }
    }

    pub fn store_cached_positions(&mut self, e: &Env) {
        for position_id in self.positions_to_update.iter() {
            let position = self.positions.get(position_id).unwrap();
            position.store(e);
        }
    }

    pub fn get_price(&mut self, e: &Env, asset_index: u32, asset: &Asset) -> i128 {
        if let Some(price) = self.prices.get(asset_index) {
            return price;
        }
        let price = load_price(e, &self.config.oracle, asset, self.config.max_price_age);
        self.prices.set(asset_index, price);
        price
    }

    pub fn calculate_caller_fee(&self, e: &Env, fee: i128) -> i128 {
        let caller_fee = fee.fixed_mul_floor(e, &self.config.caller_take_rate, &SCALAR_7);
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
        let mut position = ctx.load_position(e, request.position_id);

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

        let action_result = match request_type {
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
/// Returns (price, pnl, FeeBreakdown) for event emission
fn handle_close(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
) -> (i128, i128, FeeBreakdown) {
    let mut market = ctx.load_market(e, position.asset_index);
    let price = ctx.get_price(e, position.asset_index, &market.config.asset);
    let pnl = position.calculate_pnl(e, price);
    let fees = position.calculate_fee_breakdown(e, &market);

    // Calculate payouts
    let equity = position.collateral + pnl - fees.total_fee();
    let max_payout = position
        .collateral
        .fixed_mul_floor(e, &market.config.max_payout, &SCALAR_7);
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

    storage::remove_user_position(e, &position.user, position.id);
    storage::remove_position(e, position.id);

    market.update_stats(-position.notional_size, position.is_long);
    ctx.cache_market(&market);

    (price, pnl, fees)
}

/// Fill a pending limit order
fn apply_fill(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
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
    position.entry_price = current_price;

    // Check if position is balancing BEFORE updating market stats
    let should_pay_base_fee = if position.is_long {
        let new_long = market.data.long_notional_size + position.notional_size;
        new_long > market.data.short_notional_size
    } else {
        let new_short = market.data.short_notional_size + position.notional_size;
        new_short > market.data.long_notional_size
    };

    market.update_stats(position.notional_size, position.is_long);

    // Calculate fees from notional_size (same formula as open)
    let base_fee = position
        .notional_size
        .fixed_mul_ceil(e, &market.config.base_fee, &SCALAR_7);
    let price_impact = position
        .notional_size
        .fixed_div_ceil(e, &market.config.price_impact_scalar, &SCALAR_7);

    // Actual base_fee charged (0 if refunded for balancing trades)
    let actual_base_fee = if should_pay_base_fee {
        // Position increases imbalance: send fees to vault (minus caller fee)
        let total_fee = base_fee + price_impact;
        let caller_fee = ctx.calculate_caller_fee(e, total_fee);
        result.add_transfer(&ctx.caller, caller_fee);
        result.add_transfer(&ctx.vault, total_fee - caller_fee);
        base_fee
    } else {
        // Position is balancing: refund base_fee to user, only price_impact goes to vault
        result.add_transfer(&position.user, base_fee);
        let caller_fee = ctx.calculate_caller_fee(e, price_impact);
        result.add_transfer(&ctx.caller, caller_fee);
        result.add_transfer(&ctx.vault, price_impact - caller_fee);
        0
    };

    ctx.cache_market(&market);
    ctx.cache_position(position);

    FillLimit {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id: position.id,
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
) -> u32 {
    let market = ctx.load_market(e, position.asset_index);
    let current_price = ctx.get_price(e, position.asset_index, &market.config.asset);
    if !position.check_stop_loss(current_price) {
        return TradingError::StopLossNotTriggered as u32;
    }

    let (price, pnl, fees) = handle_close(e, result, ctx, position);

    StopLoss {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id: position.id,
        price,
        pnl,
        base_fee: fees.base_fee,
        impact_fee: fees.impact_fee,
        interest: fees.interest,
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
) -> u32 {
    let market = ctx.load_market(e, position.asset_index);
    let current_price = ctx.get_price(e, position.asset_index, &market.config.asset);
    if !position.check_take_profit(current_price) {
        return TradingError::TakeProfitNotTriggered as u32;
    }

    let (price, pnl, fees) = handle_close(e, result, ctx, position);

    TakeProfit {
        asset_index: position.asset_index,
        user: position.user.clone(),
        position_id: position.id,
        price,
        pnl,
        base_fee: fees.base_fee,
        impact_fee: fees.impact_fee,
        interest: fees.interest,
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
) -> u32 {
    let mut market = ctx.load_market(e, position.asset_index);
    let current_price = ctx.get_price(e, position.asset_index, &market.config.asset);

    let pnl = position.calculate_pnl(e, current_price);
    let fees = position.calculate_fee_breakdown(e, &market);
    let equity = position.collateral + pnl - fees.total_fee();
    let required_margin =
        position
            .notional_size
            .fixed_mul_floor(e, &market.config.maintenance_margin, &SCALAR_7);

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
        position_id: position.id,
        price: current_price,
        pnl,
        base_fee: fees.base_fee,
        impact_fee: fees.impact_fee,
        interest: fees.interest,
    }
    .publish(e);

    market.update_stats(-position.notional_size, position.is_long);

    storage::remove_user_position(e, &position.user, position.id);
    storage::remove_position(e, position.id);
    ctx.cache_market(&market);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{SCALAR_18, SCALAR_7};
    use crate::testutils::{
        create_oracle, create_token, create_trading, create_vault, default_config, default_market,
        default_market_data, BTC_PRICE,
    };
    use crate::trading::config::execute_initialize;
    use crate::types::{ContractStatus, ExecuteRequest};
    use sep_40_oracle::Asset;
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{vec, String, Symbol};

    fn setup_env() -> Env {
        let e = Env::default();
        e.mock_all_auths();
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
        e
    }

    fn setup_contract(e: &Env) -> (Address, Address, Address) {
        let (address, owner) = create_trading(e);
        let (oracle, _) = create_oracle(e);
        let (token, _) = create_token(e, &owner);
        let (vault, _) = create_vault(e, &token, 1_000_000 * SCALAR_7);

        e.as_contract(&address, || {
            execute_initialize(
                e,
                &String::from_str(e, "Test"),
                &vault,
                &default_config(&oracle),
            );
            storage::set_status(e, ContractStatus::Active as u32);

            // Set up market
            let market_config = default_market(e);
            storage::set_market_config(e, 0, &market_config);
            let mut market_data = default_market_data();
            market_data.last_update = e.ledger().timestamp();
            market_data.long_interest_index = SCALAR_18;
            market_data.short_interest_index = SCALAR_18;
            storage::set_market_data(e, 0, &market_data);
            storage::next_market_index(e); // Advance counter to 1
        });

        (address, vault, oracle)
    }

    fn create_test_position(
        e: &Env,
        user: &Address,
        filled: bool,
        is_long: bool,
        entry_price: i128,
    ) -> crate::types::Position {
        crate::types::Position {
            id: 1,
            user: user.clone(),
            filled,
            asset_index: 0,
            is_long,
            stop_loss: 0,
            take_profit: 0,
            entry_price,
            collateral: 1_000 * SCALAR_7,
            notional_size: 10_000 * SCALAR_7,
            interest_index: SCALAR_18,
            created_at: e.ledger().timestamp(),
        }
    }

    // ==========================================
    // ExecuteContext Tests
    // ==========================================

    #[test]
    fn test_execute_context_load() {
        let e = setup_env();
        let (address, vault, _) = setup_contract(&e);
        let caller = Address::generate(&e);

        e.as_contract(&address, || {
            let ctx = ExecuteContext::load(&e, caller.clone());
            assert_eq!(ctx.vault, vault);
            assert_eq!(ctx.caller, caller);
            assert_eq!(ctx.markets.len(), 0);
            assert_eq!(ctx.positions.len(), 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #301)")]
    fn test_execute_context_load_not_initialized() {
        let e = setup_env();
        let (address, _) = create_trading(&e);
        let caller = Address::generate(&e);

        e.as_contract(&address, || {
            ExecuteContext::load(&e, caller);
        });
    }

    #[test]
    fn test_execute_context_load_market() {
        let e = setup_env();
        let (address, vault, _) = setup_contract(&e);
        let caller = Address::generate(&e);

        e.as_contract(&address, || {
            let mut ctx = ExecuteContext::load(&e, caller);
            let market = ctx.load_market(&e, 0);
            assert_eq!(market.asset_index, 0);
            assert!(market.config.enabled);

            // Load again - should use cache after caching
            ctx.cache_market(&market);
            let market2 = ctx.load_market(&e, 0);
            assert_eq!(market2.asset_index, 0);
        });
    }

    #[test]
    fn test_execute_context_load_position() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            let position = create_test_position(&e, &user, true, true, BTC_PRICE);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let loaded = ctx.load_position(&e, 1);
            assert_eq!(loaded.id, 1);
            assert_eq!(loaded.user, user);

            // Cache and load again
            ctx.cache_position(&loaded);
            let loaded2 = ctx.load_position(&e, 1);
            assert_eq!(loaded2.id, 1);
        });
    }

    #[test]
    fn test_execute_context_cache_market() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);

        e.as_contract(&address, || {
            let mut ctx = ExecuteContext::load(&e, caller);
            let market = ctx.load_market(&e, 0);

            ctx.cache_market(&market);
            assert!(ctx.markets.contains_key(0));
            assert!(ctx.markets_to_update.contains(&0));

            // Caching same market again shouldn't duplicate in update list
            ctx.cache_market(&market);
            assert_eq!(ctx.markets_to_update.len(), 1);
        });
    }

    #[test]
    fn test_execute_context_cache_position() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            let position = create_test_position(&e, &user, true, true, BTC_PRICE);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let loaded = ctx.load_position(&e, 1);
            ctx.cache_position(&loaded);

            assert!(ctx.positions.contains_key(1));
            assert!(ctx.positions_to_update.contains(&1));

            // Caching again shouldn't duplicate
            ctx.cache_position(&loaded);
            assert_eq!(ctx.positions_to_update.len(), 1);
        });
    }

    #[test]
    fn test_execute_context_store_cached() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            let position = create_test_position(&e, &user, true, true, BTC_PRICE);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut market = ctx.load_market(&e, 0);
            market.data.long_notional_size = 5000 * SCALAR_7;
            ctx.cache_market(&market);

            let mut pos = ctx.load_position(&e, 1);
            pos.collateral = 2000 * SCALAR_7;
            ctx.cache_position(&pos);

            ctx.store_cached_markets(&e);
            ctx.store_cached_positions(&e);

            // Verify stored
            let stored_market = storage::get_market_data(&e, 0);
            assert_eq!(stored_market.long_notional_size, 5000 * SCALAR_7);

            let stored_pos = storage::get_position(&e, 1);
            assert_eq!(stored_pos.collateral, 2000 * SCALAR_7);
        });
    }

    #[test]
    fn test_execute_context_get_price() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);

        e.as_contract(&address, || {
            let mut ctx = ExecuteContext::load(&e, caller);
            let asset = Asset::Other(Symbol::new(&e, "BTC"));
            let price = ctx.get_price(&e, 0, &asset);

            assert_eq!(price, BTC_PRICE);

            // Second call should use cache
            let price2 = ctx.get_price(&e, 0, &asset);
            assert_eq!(price2, BTC_PRICE);
        });
    }

    #[test]
    fn test_execute_context_calculate_caller_fee() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);

        e.as_contract(&address, || {
            let ctx = ExecuteContext::load(&e, caller);
            // caller_take_rate = 10% (0_1000000)
            let fee = ctx.calculate_caller_fee(&e, 100 * SCALAR_7);
            assert_eq!(fee, 10 * SCALAR_7);

            // Zero fee
            let fee_zero = ctx.calculate_caller_fee(&e, 0);
            assert_eq!(fee_zero, 0);
        });
    }

    // ==========================================
    // ProcessingResult Tests
    // ==========================================

    #[test]
    fn test_processing_result_new() {
        let e = setup_env();
        let result = ProcessingResult::new(&e);
        assert_eq!(result.transfers.len(), 0);
        assert_eq!(result.results.len(), 0);
    }

    #[test]
    fn test_processing_result_add_transfer() {
        let e = setup_env();
        let mut result = ProcessingResult::new(&e);
        let user = Address::generate(&e);

        result.add_transfer(&user, 100);
        assert_eq!(result.transfers.get(user.clone()), Some(100));

        // Add more to same address
        result.add_transfer(&user, 50);
        assert_eq!(result.transfers.get(user), Some(150));
    }

    #[test]
    fn test_processing_result_multiple_addresses() {
        let e = setup_env();
        let mut result = ProcessingResult::new(&e);
        let user1 = Address::generate(&e);
        let user2 = Address::generate(&e);

        result.add_transfer(&user1, 100);
        result.add_transfer(&user2, 200);
        result.add_transfer(&user1, -30);

        assert_eq!(result.transfers.get(user1), Some(70));
        assert_eq!(result.transfers.get(user2), Some(200));
    }

    // ==========================================
    // apply_fill Tests
    // ==========================================

    #[test]
    fn test_apply_fill_long_success() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
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

            let code = apply_fill(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, 0);
            assert!(position.filled);
            assert_eq!(position.entry_price, BTC_PRICE);
        });
    }

    #[test]
    fn test_apply_fill_short_success() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
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

            let code = apply_fill(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, 0);
            assert!(position.filled);
        });
    }

    #[test]
    fn test_apply_fill_long_not_fillable() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Entry price below current - not fillable for long
            let mut position =
                create_test_position(&e, &user, false, true, BTC_PRICE - 1000 * SCALAR_7);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_fill(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, TradingError::LimitOrderNotFillable as u32);
            assert!(!position.filled);
        });
    }

    #[test]
    fn test_apply_fill_short_not_fillable() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Entry price above current - not fillable for short
            let mut position =
                create_test_position(&e, &user, false, false, BTC_PRICE + 1000 * SCALAR_7);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_fill(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, TradingError::LimitOrderNotFillable as u32);
        });
    }

    #[test]
    fn test_apply_fill_balancing_trade() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Setup market with short dominant
            let mut market_data = default_market_data();
            market_data.long_notional_size = 100_000 * SCALAR_7;
            market_data.short_notional_size = 200_000 * SCALAR_7;
            market_data.last_update = e.ledger().timestamp();
            market_data.long_interest_index = SCALAR_18;
            market_data.short_interest_index = SCALAR_18;
            storage::set_market_data(&e, 0, &market_data);

            // Long order will be balancing
            let mut position =
                create_test_position(&e, &user, false, true, BTC_PRICE + 1000 * SCALAR_7);
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller.clone());
            let mut result = ProcessingResult::new(&e);

            let code = apply_fill(&e, &mut result, &mut ctx, &mut position);
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
        let (address, _, _) = setup_contract(&e);
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

            let code = apply_stop_loss(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, 0);
        });
    }

    #[test]
    fn test_apply_stop_loss_not_triggered() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Long with SL way below current price
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.stop_loss = BTC_PRICE - 50000 * SCALAR_7;
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_stop_loss(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, TradingError::StopLossNotTriggered as u32);
        });
    }

    #[test]
    fn test_apply_stop_loss_short_triggered() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
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

            let code = apply_stop_loss(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, 0);
        });
    }

    // ==========================================
    // apply_take_profit Tests
    // ==========================================

    #[test]
    fn test_apply_take_profit_long_triggered() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
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

            let code = apply_take_profit(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, 0);
        });
    }

    #[test]
    fn test_apply_take_profit_not_triggered() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Long with TP way above current price
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.take_profit = BTC_PRICE + 50000 * SCALAR_7;
            storage::set_position(&e, 1, &position);

            let mut ctx = ExecuteContext::load(&e, caller);
            let mut result = ProcessingResult::new(&e);

            let code = apply_take_profit(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, TradingError::TakeProfitNotTriggered as u32);
        });
    }

    #[test]
    fn test_apply_take_profit_short_triggered() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
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

            let code = apply_take_profit(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, 0);
        });
    }

    // ==========================================
    // apply_liquidation Tests
    // ==========================================

    #[test]
    fn test_apply_liquidation_underwater() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Create position that's underwater
            // Entry at 100k, current at 100k, but with high interest
            let mut position = create_test_position(&e, &user, true, true, BTC_PRICE);
            position.collateral = 100 * SCALAR_7; // Very small collateral
            position.notional_size = 10_000 * SCALAR_7; // 100x leverage
            position.interest_index = SCALAR_18;

            // Set high interest index so position is underwater
            let mut market_data = default_market_data();
            market_data.long_interest_index = SCALAR_18 + SCALAR_18 / 10; // 10% interest accrued
            market_data.short_interest_index = SCALAR_18;
            market_data.last_update = e.ledger().timestamp();
            storage::set_market_data(&e, 0, &market_data);

            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            let mut ctx = ExecuteContext::load(&e, caller.clone());
            let mut result = ProcessingResult::new(&e);

            let code = apply_liquidation(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, 0);

            // Caller should receive fee
            assert!(result.transfers.get(caller).unwrap_or(0) > 0);
        });
    }

    #[test]
    fn test_apply_liquidation_not_liquidatable() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
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

            let code = apply_liquidation(&e, &mut result, &mut ctx, &mut position);
            assert_eq!(code, TradingError::PositionNotLiquidatable as u32);
        });
    }

    // ==========================================
    // process_execute_requests Tests
    // ==========================================

    #[test]
    fn test_process_filled_position_error() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
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
        let (address, _, _) = setup_contract(&e);
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
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        e.as_contract(&address, || {
            // Create positions
            let mut pos1 = create_test_position(&e, &user, false, true, BTC_PRICE + 1000 * SCALAR_7);
            pos1.id = 1;
            storage::set_position(&e, 1, &pos1);
            storage::add_user_position(&e, &user, 1);

            let mut pos2 = create_test_position(&e, &user, true, true, BTC_PRICE);
            pos2.id = 2;
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
        let (address, _, _) = setup_contract(&e);
        let caller = Address::generate(&e);
        let user = Address::generate(&e);

        // Advance time so the price has increased
        e.ledger().set(LedgerInfo {
            timestamp: 2000,
            protocol_version: 25,
            sequence_number: 200,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3110400,
        });

        e.as_contract(&address, || {
            // Entry at price below current (profitable long)
            let mut position =
                create_test_position(&e, &user, true, true, BTC_PRICE - 10000 * SCALAR_7);
            storage::set_position(&e, 1, &position);
            storage::add_user_position(&e, &user, 1);

            // Update market data timestamp
            let mut market_data = default_market_data();
            market_data.last_update = e.ledger().timestamp();
            market_data.long_interest_index = SCALAR_18;
            market_data.short_interest_index = SCALAR_18;
            storage::set_market_data(&e, 0, &market_data);

            let mut ctx = ExecuteContext::load(&e, caller.clone());
            let mut result = ProcessingResult::new(&e);

            let (price, pnl, _fees) = handle_close(&e, &mut result, &mut ctx, &mut position);

            assert_eq!(price, BTC_PRICE);
            assert!(pnl > 0);
            // User should receive payout
            assert!(result.transfers.get(user).unwrap_or(0) > 0);
        });
    }

    #[test]
    fn test_handle_close_loss() {
        let e = setup_env();
        let (address, _, _) = setup_contract(&e);
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

            let (price, pnl, _fees) = handle_close(&e, &mut result, &mut ctx, &mut position);

            assert_eq!(price, BTC_PRICE);
            assert!(pnl < 0);
        });
    }
}

