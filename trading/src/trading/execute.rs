use crate::constants::SCALAR_7;
use crate::errors::TradingError;
use crate::events::{FillLimit, Liquidation, StopLoss, TakeProfit};
use crate::storage;
use crate::trading::market::Market;
use crate::trading::position::{Position, Settlement};
use crate::dependencies::PriceData;
use crate::types::{ExecuteRequest, ExecuteRequestType};
use crate::validation::require_can_manage;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

pub(crate) struct Transfers {
    pub map: Map<Address, i128>,
}

impl Transfers {
    pub fn new(e: &Env) -> Self {
        Transfers { map: Map::new(e) }
    }

    pub fn add(&mut self, address: &Address, amount: i128) {
        self.map.set(
            address.clone(),
            amount + self.map.get(address.clone()).unwrap_or(0),
        );
    }
}

/// Execute keeper triggers (Fill, StopLoss, TakeProfit, Liquidate) for a single market.
pub fn execute_trigger(
    e: &Env,
    caller: &Address,
    requests: Vec<ExecuteRequest>,
    price_data: &PriceData,
) {
    require_can_manage(e);

    let mut market = Market::load(e, price_data);
    let transfers = process_execute_requests(e, &mut market, caller, requests);

    let token_client = TokenClient::new(e, &market.token);
    let vault_client = crate::dependencies::VaultClient::new(e, &market.vault);

    // STEP 1: Vault pays to contract (if needed)
    let vault_transfer = transfers.map.get(market.vault.clone()).unwrap_or(0);
    if vault_transfer < 0 {
        vault_client.strategy_withdraw(&e.current_contract_address(), &vault_transfer.abs());
    }

    // STEP 2: Handle all other transfers
    for (address, amount) in transfers.map.iter() {
        if address != market.vault {
            if amount > 0 {
                token_client.transfer(&e.current_contract_address(), &address, &amount);
            }
        }
    }

    // STEP 3: Contract pays to vault if needed
    if vault_transfer > 0 {
        token_client.transfer(&e.current_contract_address(), &market.vault, &vault_transfer);
    }

    market.store(e);
}

fn process_execute_requests(
    e: &Env,
    market: &mut Market,
    caller: &Address,
    requests: Vec<ExecuteRequest>,
) -> Transfers {
    let mut t = Transfers::new(e);

    for request in requests.iter() {
        let request_type = ExecuteRequestType::from_u32(e, request.request_type);
        let mut position = storage::get_position(e, request.position_id);

        if position.feed != market.feed_id {
            panic_with_error!(e, TradingError::InvalidPrice);
        }

        let position_id = request.position_id;
        match request_type {
            ExecuteRequestType::Fill => apply_fill(e, &mut t, market, caller, &mut position, position_id),
            ExecuteRequestType::StopLoss => apply_stop_loss(e, &mut t, market, caller, &mut position, position_id),
            ExecuteRequestType::TakeProfit => apply_take_profit(e, &mut t, market, caller, &mut position, position_id),
            ExecuteRequestType::Liquidate => apply_liquidation(e, &mut t, market, caller, &mut position, position_id),
        };
    }

    t
}

/// Handle position close logic shared by stop loss and take profit.
fn handle_close(
    e: &Env,
    t: &mut Transfers,
    market: &mut Market,
    caller: &Address,
    position: &mut Position,
    position_id: u32,
) -> Settlement {
    if !position.filled {
        panic_with_error!(e, TradingError::ActionNotAllowedForStatus);
    }
    position.require_closable(e);

    let col = position.col;
    let s = market.close(e, position, position_id);

    let user_payout = s.equity(col).max(0);
    let treasury_fee = market.treasury_fee(e, s.protocol_fee());
    let caller_fee = s.trading_fee()
        .fixed_mul_floor(e, &market.trading_config.caller_rate, &SCALAR_7);
    let vault_transfer = col - user_payout - treasury_fee - caller_fee;

    if user_payout > 0 { t.add(&position.user, user_payout); }
    if vault_transfer != 0 { t.add(&market.vault, vault_transfer); }
    if treasury_fee > 0 { t.add(&market.treasury, treasury_fee); }
    if caller_fee > 0 { t.add(caller, caller_fee); }

    s
}

/// Fill a pending limit order
fn apply_fill(
    e: &Env,
    t: &mut Transfers,
    market: &mut Market,
    caller: &Address,
    position: &mut Position,
    position_id: u32,
) {
    if position.filled {
        panic_with_error!(e, TradingError::PositionNotPending);
    }

    let can_fill = if position.long {
        market.price <= position.entry_price
    } else {
        market.price >= position.entry_price
    };
    if !can_fill {
        panic_with_error!(e, TradingError::LimitOrderNotFillable);
    }

    position.entry_price = market.price;

    let (base_fee, impact_fee) = market.open(e, position, position_id);
    let total_fee = base_fee + impact_fee;
    let treasury_fee = market.treasury_fee(e, total_fee);
    let caller_fee = total_fee
        .fixed_mul_floor(e, &market.trading_config.caller_rate, &SCALAR_7);
    let vault_fee = total_fee - treasury_fee - caller_fee;

    t.add(&market.vault, vault_fee);
    if treasury_fee > 0 { t.add(&market.treasury, treasury_fee); }
    if caller_fee > 0 { t.add(caller, caller_fee); }

    FillLimit {
        feed_id: position.feed,
        user: position.user.clone(),
        position_id,
        base_fee,
        impact_fee,
    }
    .publish(e);
}

/// Trigger stop loss on a position
fn apply_stop_loss(
    e: &Env,
    t: &mut Transfers,
    market: &mut Market,
    caller: &Address,
    position: &mut Position,
    position_id: u32,
) {
    if !position.check_stop_loss(market.price) {
        panic_with_error!(e, TradingError::StopLossNotTriggered);
    }

    let s = handle_close(e, t, market, caller, position, position_id);

    StopLoss {
        feed_id: position.feed,
        user: position.user.clone(),
        position_id,
        price: market.price,
        pnl: s.net_pnl(position.col),
        base_fee: s.base_fee,
        impact_fee: s.impact_fee,
        funding: s.funding,
        borrowing_fee: s.borrowing_fee,
    }
    .publish(e);
}

/// Trigger take profit on a position
fn apply_take_profit(
    e: &Env,
    t: &mut Transfers,
    market: &mut Market,
    caller: &Address,
    position: &mut Position,
    position_id: u32,
) {
    if !position.check_take_profit(market.price) {
        panic_with_error!(e, TradingError::TakeProfitNotTriggered);
    }

    let s = handle_close(e, t, market, caller, position, position_id);

    TakeProfit {
        feed_id: position.feed,
        user: position.user.clone(),
        position_id,
        price: market.price,
        pnl: s.net_pnl(position.col),
        base_fee: s.base_fee,
        impact_fee: s.impact_fee,
        funding: s.funding,
        borrowing_fee: s.borrowing_fee,
    }
    .publish(e);
}

/// Liquidate an underwater position
fn apply_liquidation(
    e: &Env,
    t: &mut Transfers,
    market: &mut Market,
    caller: &Address,
    position: &mut Position,
    position_id: u32,
) {
    if !position.filled {
        panic_with_error!(e, TradingError::ActionNotAllowedForStatus);
    }

    let col = position.col;
    let s = market.close(e, position, position_id);
    let liq_threshold = position.notional.fixed_mul_floor(e, &market.config.liq_fee, &SCALAR_7);
    let equity = s.equity(col);

    if equity >= liq_threshold {
        panic_with_error!(e, TradingError::PositionNotLiquidatable);
    }

    // User gets nothing — liq bonus is whatever equity remains
    let liq_fee = equity.max(0);
    let revenue = (s.protocol_fee() + liq_fee).min(col);
    let treasury_fee = market.treasury_fee(e, revenue);
    let caller_fee = (s.trading_fee() + liq_fee).min(col)
        .fixed_mul_floor(e, &market.trading_config.caller_rate, &SCALAR_7);

    t.add(&market.vault, col - treasury_fee - caller_fee);
    if treasury_fee > 0 { t.add(&market.treasury, treasury_fee); }
    if caller_fee > 0 { t.add(caller, caller_fee); }

    Liquidation {
        feed_id: position.feed,
        user: position.user.clone(),
        position_id,
        price: market.price,
        base_fee: s.base_fee,
        impact_fee: s.impact_fee,
        funding: s.funding,
        borrowing_fee: s.borrowing_fee,
        liq_fee,
    }
    .publish(e);
}

#[cfg(test)]
mod tests {
    use crate::constants::SCALAR_7;
    use crate::storage;
    use crate::testutils::{
        setup_contract, setup_env, BTC_FEED_ID, BTC_PRICE, PRICE_SCALAR,
    };
    use crate::dependencies::PriceData;
    use crate::types::{ExecuteRequest, ExecuteRequestType};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{vec, Address};

    fn btc_price_data(e: &soroban_sdk::Env, price: i128) -> PriceData {
        PriceData {
            feed_id: BTC_FEED_ID,
            price,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        }
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
            let id = crate::trading::execute_create_limit(
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
            let id = crate::trading::execute_create_limit(
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

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &pd);

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

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &pd);
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

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &pd);

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

        let id = create_pending_long(&e, &contract, &user, 1_100 * SCALAR_7, 100_000 * SCALAR_7, BTC_PRICE);

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &pd);

            let crash_pd = btc_price_data(&e, 9_800_000_000_000_i128);
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Liquidate as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &crash_pd);
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

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &pd);

            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Liquidate as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &pd);
        });
    }

    #[test]
    fn test_stop_loss_triggered() {
        use crate::testutils::jump;
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = e.as_contract(&contract, || {
            let id = crate::trading::execute_create_limit(
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

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &pd);

            jump(&e, 1000 + 31); // advance past MIN_OPEN_TIME

            let sl_pd = btc_price_data(&e, 9_400_000_000_000_i128);
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::StopLoss as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &sl_pd);
        });
    }

    #[test]
    fn test_take_profit_triggered() {
        use crate::testutils::jump;
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        let caller = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = e.as_contract(&contract, || {
            let id = crate::trading::execute_create_limit(
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

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &pd);

            jump(&e, 1000 + 31); // advance past MIN_OPEN_TIME

            let tp_pd = btc_price_data(&e, 11_500_000_000_000_i128);
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::TakeProfit as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &tp_pd);
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

        let pd = btc_price_data(&e, BTC_PRICE);
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
            super::execute_trigger(&e, &caller, requests, &pd);
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

        let pd = btc_price_data(&e, BTC_PRICE);
        e.as_contract(&contract, || {
            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &pd);

            let requests = vec![
                &e,
                ExecuteRequest {
                    request_type: ExecuteRequestType::Fill as u32,
                    position_id: id,
                },
            ];
            super::execute_trigger(&e, &caller, requests, &pd);
        });
    }
}
