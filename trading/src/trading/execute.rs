use crate::constants::{MAX_STALENESS_KEEPER, SCALAR_7};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{FillLimit, Liquidation, StopLoss, TakeProfit};
use crate::storage;
use crate::trading::actions::calculate_protocol_fee;
use crate::trading::position::Position;
use crate::trading::price_verifier::{check_staleness, scalar_from_exponent, PriceData};
use crate::types::{ExecuteRequest, ExecuteRequestType, MarketData, TradingConfig};
use crate::validation::{require_min_open_time, require_not_frozen};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{map, panic_with_error, vec, Address, Env, Map, Vec};

/// Context for batch execution operations (keeper triggers)
/// Caches market data and prices to minimize storage reads and price-verifier calls
pub struct ExecuteContext<'a> {
    pub config: TradingConfig,
    pub vault: Address,
    pub treasury: Address,
    pub token_client: TokenClient<'a>,
    pub caller: Address,
    data_cache: Map<u32, MarketData>,
    markets_to_update: Vec<u32>,
    price_map: Map<u32, PriceData>,
}

impl<'a> ExecuteContext<'a> {
    pub fn load(e: &'a Env, caller: Address, feeds: &Vec<PriceData>) -> Self {
        let config = storage::get_config(e);
        let vault = storage::get_vault(e);
        let treasury = storage::get_treasury(e);
        let token = storage::get_token(e);
        let token_client = TokenClient::new(e, &token);

        let mut price_map: Map<u32, PriceData> = map![e];
        for feed in feeds.iter() {
            check_staleness(e, feed.publish_time, MAX_STALENESS_KEEPER);
            price_map.set(feed.feed_id, feed);
        }

        ExecuteContext {
            config,
            vault,
            treasury,
            token_client,
            caller,
            data_cache: map![e],
            markets_to_update: vec![e],
            price_map,
        }
    }

    pub fn load_data(&mut self, e: &Env, feed_id: u32) -> MarketData {
        if let Some(d) = self.data_cache.get(feed_id) {
            return d;
        }
        let mut data = storage::get_market_data(e, feed_id);
        data.accrue(e);
        self.data_cache.set(feed_id, data.clone());
        if !self.markets_to_update.contains(&feed_id) {
            self.markets_to_update.push_back(feed_id);
        }
        data
    }

    pub fn cache_data(&mut self, feed_id: u32, data: &MarketData) {
        self.data_cache.set(feed_id, data.clone());
        if !self.markets_to_update.contains(&feed_id) {
            self.markets_to_update.push_back(feed_id);
        }
    }

    pub fn store_cached_data(&self, e: &Env) {
        for feed_id in self.markets_to_update.iter() {
            let data = self.data_cache.get(feed_id).unwrap();
            storage::set_market_data(e, feed_id, &data);
        }
    }

    /// Get the verified price for a market's feed_id.
    /// Returns (price, price_scalar).
    pub fn get_price(&self, e: &Env, feed_id: u32) -> (i128, i128) {
        if let Some(feed) = self.price_map.get(feed_id) {
            return (feed.price, scalar_from_exponent(feed.exponent));
        }
        panic_with_error!(e, TradingError::PriceNotFound)
    }

    pub fn calculate_caller_fee(&self, e: &Env, fee: i128) -> i128 {
        let caller_fee = fee.fixed_mul_floor(e, &self.config.caller_take_rate, &SCALAR_7);
        if caller_fee > 0 { caller_fee } else { 0 }
    }
}

/// Internal processing result that tracks transfers for execution
pub(crate) struct ProcessingResult {
    pub transfers: Map<Address, i128>,
}

impl ProcessingResult {
    pub fn new(e: &Env) -> Self {
        ProcessingResult {
            transfers: Map::new(e),
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
pub fn execute_trigger(
    e: &Env,
    caller: &Address,
    requests: Vec<ExecuteRequest>,
    feeds: &Vec<PriceData>,
) {
    require_not_frozen(e);

    let mut ctx = ExecuteContext::load(e, caller.clone(), feeds);
    let processing_result = process_execute_requests(e, &mut ctx, requests);

    let token_client = &ctx.token_client;
    let vault_client = VaultClient::new(e, &ctx.vault);

    // STEP 1: Vault pays to contract (if needed)
    let vault_transfer = processing_result.transfers.get(ctx.vault.clone()).unwrap_or(0);
    if vault_transfer < 0 {
        vault_client.strategy_withdraw(&e.current_contract_address(), &vault_transfer.abs());
    }

    // STEP 2: Handle all other transfers
    for (address, amount) in processing_result.transfers.iter() {
        if address != ctx.vault {
            if amount > 0 {
                token_client.transfer(&e.current_contract_address(), &address, &amount);
            }
        }
    }

    // STEP 3: Contract pays to vault if needed
    if vault_transfer > 0 {
        token_client.transfer(&e.current_contract_address(), &ctx.vault, &vault_transfer);
    }
}

/// Process batch of keeper requests
fn process_execute_requests(
    e: &Env,
    ctx: &mut ExecuteContext,
    requests: Vec<ExecuteRequest>,
) -> ProcessingResult {
    let mut result = ProcessingResult::new(e);

    for request in requests.iter() {
        let request_type = ExecuteRequestType::from_u32(e, request.request_type);
        let mut position = storage::get_position(e, request.position_id);

        match request_type {
            ExecuteRequestType::Fill => {
                if position.filled {
                    panic_with_error!(e, TradingError::PositionNotPending);
                }
            }
            ExecuteRequestType::StopLoss
            | ExecuteRequestType::TakeProfit
            | ExecuteRequestType::Liquidate => {
                if !position.filled {
                    panic_with_error!(e, TradingError::ActionNotAllowedForStatus);
                }
                let data = ctx.load_data(e, position.feed_id);
                position.notional_size = position.effective_notional(e, &data);
            }
        };

        let position_id = request.position_id;
        match request_type {
            ExecuteRequestType::Fill => apply_fill(e, &mut result, ctx, &mut position, position_id),
            ExecuteRequestType::StopLoss => apply_stop_loss(e, &mut result, ctx, &mut position, position_id),
            ExecuteRequestType::TakeProfit => apply_take_profit(e, &mut result, ctx, &mut position, position_id),
            ExecuteRequestType::Liquidate => apply_liquidation(e, &mut result, ctx, &mut position, position_id),
        };
    }

    ctx.store_cached_data(e);

    result
}

/// Handle position close logic shared by stop loss and take profit.
fn handle_close(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
    position_id: u32,
) -> (i128, i128, i128, i128, i128) {
    let mut data = ctx.load_data(e, position.feed_id);
    let market_config = storage::get_market_config(e, position.feed_id);
    let (price, price_scalar) = ctx.get_price(e, position.feed_id);
    let close = position.close(e, &data, &market_config, &ctx.config, price, price_scalar);

    let skim = close.fees.vault_skim.min(close.user_payout);
    let user_payout = close.user_payout - skim;
    let mut vault_transfer = close.vault_transfer + skim;

    let total_fee = close.fees.total_fee();
    let protocol_fee = calculate_protocol_fee(e, &ctx.treasury, total_fee).min(vault_transfer.max(0));
    vault_transfer -= protocol_fee;

    let caller_fee = if vault_transfer > 0 {
        ctx.calculate_caller_fee(e, total_fee - protocol_fee).min(vault_transfer)
    } else {
        0
    };
    vault_transfer -= caller_fee;

    if user_payout > 0 {
        result.add_transfer(&position.user, user_payout);
    }
    if vault_transfer != 0 {
        result.add_transfer(&ctx.vault, vault_transfer);
    }
    if protocol_fee > 0 {
        result.add_transfer(&ctx.treasury, protocol_fee);
    }
    if caller_fee > 0 {
        result.add_transfer(&ctx.caller, caller_fee);
    }

    storage::remove_user_position(e, &position.user, position_id);
    storage::remove_position(e, position_id);

    data.update_stats(e, -position.notional_size, position.is_long, position.entry_price, price_scalar);
    ctx.cache_data(position.feed_id, &data);

    (price, close.pnl, close.fees.base_fee, close.fees.impact_fee, close.fees.funding)
}

/// Fill a pending limit order
fn apply_fill(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
    position_id: u32,
) {
    let mut data = ctx.load_data(e, position.feed_id);
    let market_config = storage::get_market_config(e, position.feed_id);
    if !market_config.enabled {
        panic_with_error!(e, TradingError::MarketDisabled);
    }
    let (current_price, price_scalar) = ctx.get_price(e, position.feed_id);

    let can_fill = if position.is_long {
        current_price <= position.entry_price
    } else {
        current_price >= position.entry_price
    };

    if !can_fill {
        panic_with_error!(e, TradingError::LimitOrderNotFillable);
    }

    position.entry_price = current_price;
    let fill = position.fill(e, &data, &market_config, &ctx.config);

    data.update_stats(e, position.notional_size, position.is_long, position.entry_price, price_scalar);

    let actual_base_fee = if fill.is_dominant {
        let total_fee = fill.fee_dominant + fill.price_impact_fee;
        let protocol_fee = calculate_protocol_fee(e, &ctx.treasury, total_fee);
        let caller_fee = ctx.calculate_caller_fee(e, total_fee - protocol_fee);
        if protocol_fee > 0 {
            result.add_transfer(&ctx.treasury, protocol_fee);
        }
        result.add_transfer(&ctx.caller, caller_fee);
        result.add_transfer(&ctx.vault, total_fee - protocol_fee - caller_fee);
        fill.fee_dominant
    } else {
        let refund = fill.fee_dominant - fill.fee_non_dominant;
        if refund > 0 {
            result.add_transfer(&position.user, refund);
        }
        let total_fee = fill.fee_non_dominant + fill.price_impact_fee;
        let protocol_fee = calculate_protocol_fee(e, &ctx.treasury, total_fee);
        let caller_fee = ctx.calculate_caller_fee(e, total_fee - protocol_fee);
        if protocol_fee > 0 {
            result.add_transfer(&ctx.treasury, protocol_fee);
        }
        result.add_transfer(&ctx.caller, caller_fee);
        result.add_transfer(&ctx.vault, total_fee - protocol_fee - caller_fee);
        fill.fee_non_dominant
    };

    ctx.cache_data(position.feed_id, &data);
    storage::set_position(e, position_id, position);

    FillLimit {
        feed_id: position.feed_id,
        user: position.user.clone(),
        position_id,
        base_fee: actual_base_fee,
        impact_fee: fill.price_impact_fee,
    }
    .publish(e);
}

/// Trigger stop loss on a position
fn apply_stop_loss(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
    position_id: u32,
) {
    require_min_open_time(e, position, ctx.config.min_open_time);
    let (current_price, _) = ctx.get_price(e, position.feed_id);
    if !position.check_stop_loss(current_price) {
        panic_with_error!(e, TradingError::StopLossNotTriggered);
    }

    let (price, pnl, base_fee, impact_fee, funding) = handle_close(e, result, ctx, position, position_id);

    StopLoss {
        feed_id: position.feed_id,
        user: position.user.clone(),
        position_id,
        price,
        pnl,
        base_fee,
        impact_fee,
        funding,
    }
    .publish(e);
}

/// Trigger take profit on a position
fn apply_take_profit(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
    position_id: u32,
) {
    require_min_open_time(e, position, ctx.config.min_open_time);
    let (current_price, _) = ctx.get_price(e, position.feed_id);
    if !position.check_take_profit(current_price) {
        panic_with_error!(e, TradingError::TakeProfitNotTriggered);
    }

    let (price, pnl, base_fee, impact_fee, funding) = handle_close(e, result, ctx, position, position_id);

    TakeProfit {
        feed_id: position.feed_id,
        user: position.user.clone(),
        position_id,
        price,
        pnl,
        base_fee,
        impact_fee,
        funding,
    }
    .publish(e);
}

/// Liquidate an underwater position
fn apply_liquidation(
    e: &Env,
    result: &mut ProcessingResult,
    ctx: &mut ExecuteContext,
    position: &mut Position,
    position_id: u32,
) {
    let mut data = ctx.load_data(e, position.feed_id);
    let market_config = storage::get_market_config(e, position.feed_id);
    let (current_price, price_scalar) = ctx.get_price(e, position.feed_id);

    let check = position.check_liquidation(e, &data, &market_config, &ctx.config, current_price, price_scalar);

    if !check.is_liquidatable {
        panic_with_error!(e, TradingError::PositionNotLiquidatable);
    }

    let caller_fee = ctx.calculate_caller_fee(e, check.fees.total_fee()).min(position.collateral);
    result.add_transfer(&ctx.caller, caller_fee);

    let vault_amount = position.collateral - caller_fee;
    result.add_transfer(&ctx.vault, vault_amount);

    Liquidation {
        feed_id: position.feed_id,
        user: position.user.clone(),
        position_id,
        price: current_price,
        pnl: check.pnl,
        base_fee: check.fees.base_fee,
        impact_fee: check.fees.impact_fee,
        funding: check.fees.funding,
    }
    .publish(e);

    data.update_stats(e, -position.notional_size, position.is_long, position.entry_price, price_scalar);

    storage::remove_user_position(e, &position.user, position_id);
    storage::remove_position(e, position_id);
    ctx.cache_data(position.feed_id, &data);
}

#[cfg(test)]
mod tests {
    use crate::constants::SCALAR_7;
    use crate::storage;
    use crate::testutils::{
        setup_contract, setup_env, BTC_FEED_ID, BTC_PRICE, PRICE_SCALAR,
    };
    use crate::trading::price_verifier::PriceData;
    use crate::types::{ExecuteRequest, ExecuteRequestType};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{vec, Address};

    fn btc_feeds(e: &soroban_sdk::Env, price: i128) -> soroban_sdk::Vec<PriceData> {
        vec![e, PriceData {
            feed_id: BTC_FEED_ID,
            price,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        }]
    }

    /// Helper: create a pending long position and return its id
    fn create_pending_long(
        e: &soroban_sdk::Env,
        contract: &Address,
        user: &Address,
        collateral: i128,
        notional: i128,
        entry_price: i128,
    ) -> u32 {
        e.as_contract(contract, || {
            let (id, _) = crate::trading::execute_create_limit(
                e, user, BTC_FEED_ID, collateral, notional, true, entry_price, 0, 0,
            );
            id
        })
    }

    fn create_pending_short(
        e: &soroban_sdk::Env,
        contract: &Address,
        user: &Address,
        collateral: i128,
        notional: i128,
        entry_price: i128,
    ) -> u32 {
        e.as_contract(contract, || {
            let (id, _) = crate::trading::execute_create_limit(
                e, user, BTC_FEED_ID, collateral, notional, false, entry_price, 0, 0,
            );
            id
        })
    }

    #[test]
    fn test_fill_long_limit_order() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);

        let feeds = btc_feeds(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);

            let pos = storage::get_position(&e, id);
            assert!(pos.filled);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #747)")]
    fn test_fill_long_limit_not_fillable() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_long(
            &e, &contract, &user,
            1_000 * SCALAR_7, 10_000 * SCALAR_7,
            90_000 * PRICE_SCALAR,
        );

        let feeds = btc_feeds(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);
        });
    }

    #[test]
    fn test_fill_short_limit_order() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_short(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);

        let feeds = btc_feeds(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);

            let pos = storage::get_position(&e, id);
            assert!(pos.filled);
        });
    }

    #[test]
    fn test_liquidation_underwater_position() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_long(&e, &contract, &user, 1_000 * SCALAR_7, 100_000 * SCALAR_7, BTC_PRICE);

        let feeds = btc_feeds(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);

            let crash_feeds = btc_feeds(&e, 9_900_000_000_000_i128);
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Liquidate as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &crash_feeds);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #746)")]
    fn test_liquidation_healthy_position() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);

        let feeds = btc_feeds(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);

            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Liquidate as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);
        });
    }

    #[test]
    fn test_stop_loss_triggered() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = e.as_contract(&contract, || {
            let (id, _) = crate::trading::execute_create_limit(
                &e, &user, BTC_FEED_ID,
                1_000 * SCALAR_7,
                10_000 * SCALAR_7,
                true,
                BTC_PRICE,
                0,
                95_000 * PRICE_SCALAR,
            );
            id
        });

        let feeds = btc_feeds(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);

            let sl_feeds = btc_feeds(&e, 9_400_000_000_000_i128);
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::StopLoss as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &sl_feeds);
        });
    }

    #[test]
    fn test_take_profit_triggered() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = e.as_contract(&contract, || {
            let (id, _) = crate::trading::execute_create_limit(
                &e, &user, BTC_FEED_ID,
                1_000 * SCALAR_7,
                10_000 * SCALAR_7,
                true,
                BTC_PRICE,
                110_000 * PRICE_SCALAR,
                0,
            );
            id
        });

        let feeds = btc_feeds(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);

            let tp_feeds = btc_feeds(&e, 11_500_000_000_000_i128);
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::TakeProfit as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &tp_feeds);
        });
    }

    #[test]
    fn test_batch_multiple_requests() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(1_000_000 * SCALAR_7));

        let id1 = create_pending_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);
        let id2 = create_pending_short(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);

        let feeds = btc_feeds(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id1,
                },
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id2,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #733)")]
    fn test_fill_already_filled_panics() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = create_pending_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, BTC_PRICE);

        let feeds = btc_feeds(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);

            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &feeds);
        });
    }
}
