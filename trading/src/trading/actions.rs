use crate::constants::{MAX_STALENESS_USER, MIN_LEVERAGE, ONE_HOUR_SECONDS, SCALAR_7};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{ApplyFunding, CancelLimit, ClosePosition, ModifyCollateral, OpenMarket, PlaceLimit, SetTriggers};
use crate::storage;
use crate::trading::position::Position;
use crate::trading::price_verifier::{check_staleness, scalar_from_exponent, PriceData};
use crate::types::{MarketConfig, TradingConfig};
use crate::validation::{require_active, require_min_open_time, require_not_frozen};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::token::TokenClient;
use soroban_sdk::{panic_with_error, Address, Env};
use crate::dependencies::TreasuryClient;


// ── Limit order (pending, no price needed) ──────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn execute_create_limit(
    e: &Env,
    user: &Address,
    feed_id: u32,
    collateral: i128,
    notional_size: i128,
    is_long: bool,
    entry_price: i128,
    take_profit: i128,
    stop_loss: i128,
) -> (u32, i128) {
    require_active(e);
    user.require_auth();

    let config = storage::get_config(e);
    let token_client = TokenClient::new(e, &storage::get_token(e));
    let market_config = storage::get_market_config(e, feed_id);
    if !market_config.enabled {
        panic_with_error!(e, TradingError::MarketDisabled);
    }

    let (id, position) = Position::create(e, user.clone(), feed_id, is_long, entry_price, collateral, notional_size, stop_loss, take_profit);
    validate_collateral_and_leverage(e, collateral, notional_size, &config, &market_config);

    let open_fee = notional_size.fixed_mul_ceil(e, &config.base_fee_dominant, &SCALAR_7);
    let price_impact_fee =
        notional_size.fixed_div_ceil(e, &market_config.price_impact_scalar, &SCALAR_7);

    token_client.transfer(
        user,
        &e.current_contract_address(),
        &(collateral + open_fee + price_impact_fee),
    );

    storage::set_position(e, id, &position);

    PlaceLimit {
        feed_id,
        user: user.clone(),
        position_id: id,
        base_fee: open_fee,
        impact_fee: price_impact_fee,
    }
    .publish(e);

    (id, open_fee + price_impact_fee)
}

// ── Market order (filled immediately, needs price) ──────────────────

#[allow(clippy::too_many_arguments)]
pub fn execute_create_market(
    e: &Env,
    user: &Address,
    feed_id: u32,
    collateral: i128,
    notional_size: i128,
    is_long: bool,
    take_profit: i128,
    stop_loss: i128,
    price_data: &PriceData,
) -> (u32, i128) {
    require_active(e);
    user.require_auth();
    check_staleness(e, price_data.publish_time, MAX_STALENESS_USER);

    let entry_price = price_data.price;
    let price_scalar = scalar_from_exponent(price_data.exponent);

    let config = storage::get_config(e);
    let token_client = TokenClient::new(e, &storage::get_token(e));
    let market_config = storage::get_market_config(e, feed_id);
    let mut data = storage::get_market_data(e, feed_id);
    data.accrue(e);
    if !market_config.enabled {
        panic_with_error!(e, TradingError::MarketDisabled);
    }

    let (id, mut position) = Position::create(e, user.clone(), feed_id, is_long, entry_price, collateral, notional_size, stop_loss, take_profit);
    validate_collateral_and_leverage(e, collateral, notional_size, &config, &market_config);
    let fill = position.fill(e, &data, &market_config, &config);

    data.update_stats(e, notional_size, is_long, entry_price, price_scalar);

    let base_fee = if fill.is_dominant { fill.fee_dominant } else { fill.fee_non_dominant };
    let total_fee = base_fee + fill.price_impact_fee;

    token_client.transfer(user, &e.current_contract_address(), &(collateral + total_fee));

    let treasury = storage::get_treasury(e);
    let protocol_fee = calculate_protocol_fee(e, &treasury, total_fee);

    let vault = storage::get_vault(e);
    token_client.transfer(&e.current_contract_address(), &vault, &(total_fee - protocol_fee));
    if protocol_fee > 0 {
        token_client.transfer(&e.current_contract_address(), &treasury, &protocol_fee);
    }

    storage::set_market_data(e, feed_id, &data);
    storage::set_position(e, id, &position);

    OpenMarket {
        feed_id,
        user: user.clone(),
        position_id: id,
        base_fee,
        impact_fee: fill.price_impact_fee,
    }
    .publish(e);

    (id, total_fee)
}

// ── Cancel pending limit order (no price needed) ────────────────────

pub fn execute_cancel_limit(e: &Env, position_id: u32) {
    require_not_frozen(e);
    let position = storage::get_position(e, position_id);
    position.user.require_auth();

    if position.filled {
        panic_with_error!(e, TradingError::PositionNotPending);
    }

    let config = storage::get_config(e);
    let token_client = TokenClient::new(e, &storage::get_token(e));
    let market_config = storage::get_market_config(e, position.feed_id);

    let base_fee = position
        .notional_size
        .fixed_mul_ceil(e, &config.base_fee_dominant, &SCALAR_7);
    let price_impact = position
        .notional_size
        .fixed_div_ceil(e, &market_config.price_impact_scalar, &SCALAR_7);

    let total_refund = position.collateral + base_fee + price_impact;
    token_client.transfer(&e.current_contract_address(), &position.user, &total_refund);

    storage::remove_user_position(e, &position.user, position_id);
    storage::remove_position(e, position_id);

    CancelLimit {
        feed_id: position.feed_id,
        user: position.user.clone(),
        position_id,
        base_fee,
        impact_fee: price_impact,
    }
    .publish(e);
}

// ── Close filled position (needs price for PnL) ────────────────────

pub fn execute_close_position(e: &Env, position_id: u32, price_data: &PriceData) -> (i128, i128) {
    require_not_frozen(e);
    check_staleness(e, price_data.publish_time, MAX_STALENESS_USER);

    let price = price_data.price;
    let price_scalar = scalar_from_exponent(price_data.exponent);

    let mut position = storage::get_position(e, position_id);
    position.user.require_auth();

    if !position.filled {
        panic_with_error!(e, TradingError::ActionNotAllowedForStatus);
    }

    let config = storage::get_config(e);
    let token_client = TokenClient::new(e, &storage::get_token(e));
    let market_config = storage::get_market_config(e, position.feed_id);
    let mut data = storage::get_market_data(e, position.feed_id);
    data.accrue(e);

    position.notional_size = position.effective_notional(e, &data);
    require_min_open_time(e, &position, config.min_open_time);

    let result = position.close(e, &data, &market_config, &config, price, price_scalar);

    let skim = result.fees.vault_skim.min(result.user_payout);
    let user_payout = result.user_payout - skim;
    let mut vault_transfer = result.vault_transfer + skim;

    let treasury = storage::get_treasury(e);
    let protocol_fee = calculate_protocol_fee(e, &treasury, result.fees.total_fee()).min(vault_transfer.max(0));
    vault_transfer -= protocol_fee;

    let vault = storage::get_vault(e);
    let vault_client = VaultClient::new(e, &vault);

    if vault_transfer < 0 {
        vault_client.strategy_withdraw(&e.current_contract_address(), &(-vault_transfer));
    } else if vault_transfer > 0 {
        token_client.transfer(&e.current_contract_address(), &vault, &vault_transfer);
    }

    if protocol_fee > 0 {
        token_client.transfer(&e.current_contract_address(), &treasury, &protocol_fee);
    }

    if user_payout > 0 {
        token_client.transfer(&e.current_contract_address(), &position.user, &user_payout);
    }

    data.update_stats(e, -position.notional_size, position.is_long, position.entry_price, price_scalar);

    storage::remove_user_position(e, &position.user, position_id);
    storage::remove_position(e, position_id);
    storage::set_market_data(e, position.feed_id, &data);

    ClosePosition {
        feed_id: position.feed_id,
        user: position.user.clone(),
        position_id,
        price,
        pnl: result.pnl,
        base_fee: result.fees.base_fee,
        impact_fee: result.fees.impact_fee,
        funding: result.fees.funding,
    }
    .publish(e);

    (result.pnl, result.fees.total_fee())
}

// ── Modify collateral ───────────────────────────────────────────────

pub fn execute_modify_collateral(e: &Env, position_id: u32, new_collateral: i128, price_data: &PriceData) {
    require_not_frozen(e);
    let mut position = storage::get_position(e, position_id);
    position.user.require_auth();

    let config = storage::get_config(e);
    let token_client = TokenClient::new(e, &storage::get_token(e));
    let market_config = storage::get_market_config(e, position.feed_id);

    let collateral_diff = new_collateral - position.collateral;
    if collateral_diff == 0 {
        panic_with_error!(e, TradingError::CollateralUnchanged);
    }

    // Reuse shared bounds + leverage validation
    validate_collateral_and_leverage(e, new_collateral, position.notional_size, &config, &market_config);

    if position.filled {
        let mut data = storage::get_market_data(e, position.feed_id);
        data.accrue(e);

        position.notional_size = position.effective_notional(e, &data);
        position.entry_adl_index = if position.is_long {
            data.long_adl_index
        } else {
            data.short_adl_index
        };

        if collateral_diff > 0 {
            token_client.transfer(&position.user, &e.current_contract_address(), &collateral_diff);
        } else {
            check_staleness(e, price_data.publish_time, MAX_STALENESS_USER);
            let price_scalar = scalar_from_exponent(price_data.exponent);
            let pnl = position.calculate_pnl(e, price_data.price, price_scalar);
            let fees = position.calculate_fee_breakdown(e, &data, &market_config, &config);
            let equity = new_collateral + pnl - fees.total_fee() - fees.vault_skim;
            let required_margin = position
                .notional_size
                .fixed_mul_floor(e, &market_config.init_margin, &SCALAR_7);

            if equity < required_margin {
                panic_with_error!(e, TradingError::WithdrawalBreaksMargin);
            }

            token_client.transfer(&e.current_contract_address(), &position.user, &-collateral_diff);
        }

        storage::set_market_data(e, position.feed_id, &data);
    } else {
        if collateral_diff > 0 {
            token_client.transfer(&position.user, &e.current_contract_address(), &collateral_diff);
        } else {
            token_client.transfer(&e.current_contract_address(), &position.user, &-collateral_diff);
        }
    }

    position.collateral = new_collateral;
    storage::set_position(e, position_id, &position);

    ModifyCollateral {
        feed_id: position.feed_id,
        user: position.user.clone(),
        position_id,
        amount: collateral_diff,
    }
    .publish(e);
}

// ── Set triggers (no price needed) ──────────────────────────────────

pub fn execute_set_triggers(e: &Env, position_id: u32, take_profit: i128, stop_loss: i128) {
    if take_profit < 0 || stop_loss < 0 {
        panic_with_error!(e, TradingError::NegativeValueNotAllowed);
    }

    require_not_frozen(e);
    let mut position = storage::get_position(e, position_id);
    position.user.require_auth();

    position.take_profit = take_profit;
    position.stop_loss = stop_loss;

    SetTriggers {
        feed_id: position.feed_id,
        user: position.user.clone(),
        position_id,
        take_profit,
        stop_loss,
    }
    .publish(e);

    storage::set_position(e, position_id, &position);
}

// ── Apply funding rates ─────────────────────────────────────────────

pub fn execute_apply_funding(e: &Env) {
    let last_funding_update = storage::get_last_funding_update(e);
    let elapsed = e.ledger().timestamp() - last_funding_update;
    if elapsed < ONE_HOUR_SECONDS {
        panic_with_error!(e, TradingError::FundingTooEarly);
    }

    let markets = storage::get_markets(e);

    for feed_id in markets.iter() {
        let market_config = storage::get_market_config(e, feed_id);
        let mut data = storage::get_market_data(e, feed_id);
        data.accrue(e);
        data.update_funding_rate(e, market_config.base_hourly_rate);
        storage::set_market_data(e, feed_id, &data);
    }

    (ApplyFunding {}).publish(e);

    storage::set_last_funding_update(e, e.ledger().timestamp());
}

// ── Treasury fee ────────────────────────────────────────────────────

pub(crate) fn calculate_protocol_fee(e: &Env, treasury: &Address, total_fee: i128) -> i128 {
    let rate = TreasuryClient::new(e, treasury).get_rate();
    if rate > 0 && total_fee > 0 {
        total_fee.fixed_mul_floor(e, &rate, &SCALAR_7)
    } else {
        0
    }
}

// ── Shared validation ───────────────────────────────────────────────

fn validate_collateral_and_leverage(
    e: &Env,
    collateral: i128,
    notional_size: i128,
    config: &TradingConfig,
    market_config: &MarketConfig,
) {
    if collateral < config.min_collateral {
        panic_with_error!(e, TradingError::CollateralBelowMinimum);
    }
    if collateral > config.max_collateral {
        panic_with_error!(e, TradingError::CollateralAboveMaximum);
    }
    if notional_size / MIN_LEVERAGE < collateral {
        panic_with_error!(e, TradingError::LeverageBelowMinimum);
    }
    // Max leverage = 1 / init_margin. Check: notional * init_margin <= collateral
    let required_margin = notional_size.fixed_mul_ceil(e, &market_config.init_margin, &SCALAR_7);
    if required_margin > collateral {
        panic_with_error!(e, TradingError::LeverageAboveMaximum);
    }
}

#[cfg(test)]
mod tests {
    use crate::constants::{SCALAR_7, SCALAR_18};
    use crate::storage;
    use crate::testutils::{
        setup_contract, setup_env, BTC_FEED_ID, BTC_PRICE,
    };
    use crate::trading::price_verifier::PriceData;
    use crate::types::ContractStatus;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;

    /// Helper: create a pending long limit order
    fn place_limit_long(e: &soroban_sdk::Env, contract: &Address, user: &Address, collateral: i128, notional: i128) -> u32 {
        e.as_contract(contract, || {
            let (id, _fees) = super::execute_create_limit(
                e,
                user,
                BTC_FEED_ID,
                collateral,
                notional,
                true,
                BTC_PRICE,
                0, 0,
            );
            id
        })
    }

    fn place_limit_short(e: &soroban_sdk::Env, contract: &Address, user: &Address, collateral: i128, notional: i128) -> u32 {
        e.as_contract(contract, || {
            let (id, _fees) = super::execute_create_limit(
                e,
                user,
                BTC_FEED_ID,
                collateral,
                notional,
                false,
                BTC_PRICE,
                0, 0,
            );
            id
        })
    }

    #[test]
    fn test_create_limit_long() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let collateral = 1_000 * SCALAR_7;
        let notional = 10_000 * SCALAR_7;
        let id = place_limit_long(&e, &contract, &user, collateral, notional);

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, id);
            assert_eq!(pos.collateral, collateral);
            assert_eq!(pos.notional_size, notional);
            assert!(pos.is_long);
            assert!(!pos.filled);
            assert_eq!(pos.entry_price, BTC_PRICE);

            let positions = storage::get_user_positions(&e, &user);
            assert_eq!(positions.len(), 1);
            assert_eq!(positions.get(0).unwrap(), id);
        });
    }

    #[test]
    fn test_create_limit_short() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let id = place_limit_short(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, id);
            assert!(!pos.is_long);
            assert!(!pos.filled);
        });
    }

    #[test]
    fn test_create_market_long() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let collateral = 1_000 * SCALAR_7;
        let notional = 10_000 * SCALAR_7;

        let price_data = PriceData {
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        };

        let id = e.as_contract(&contract, || {
            let (id, _fees) = super::execute_create_market(
                &e, &user, BTC_FEED_ID, collateral, notional, true, 0, 0, &price_data,
            );
            id
        });

        e.as_contract(&contract, || {
            let pos = storage::get_position(&e, id);
            assert_eq!(pos.collateral, collateral);
            assert_eq!(pos.notional_size, notional);
            assert!(pos.is_long);
            assert!(pos.filled); // market order is filled immediately
            assert_eq!(pos.entry_price, BTC_PRICE);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #736)")]
    fn test_create_limit_zero_collateral() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        place_limit_long(&e, &contract, &user, 0, 10_000 * SCALAR_7);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #736)")]
    fn test_create_limit_below_min_collateral() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        place_limit_long(&e, &contract, &user, SCALAR_7 - 1, 10_000 * SCALAR_7);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #738)")]
    fn test_create_limit_below_min_leverage() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let collateral = 1_000 * SCALAR_7;
        let notional = collateral; // 1x leverage — too low
        place_limit_long(&e, &contract, &user, collateral, notional);
    }

    #[test]
    fn test_apply_funding_rate() {
        use crate::testutils::jump;

        let e = setup_env();
        let (contract, _token_client) = setup_contract(&e);

        jump(&e, 1000 + 3601);

        e.as_contract(&contract, || {
            super::execute_apply_funding(&e);
            let last = storage::get_last_funding_update(&e);
            assert_eq!(last, 1000 + 3601);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #790)")]
    fn test_apply_funding_too_early() {
        use crate::testutils::jump;

        let e = setup_env();
        let (contract, _token_client) = setup_contract(&e);

        jump(&e, 1000 + 1800);

        e.as_contract(&contract, || {
            super::execute_apply_funding(&e);
        });
    }

    #[test]
    fn test_cancel_limit() {
        let e = setup_env();
        let (contract, token_client) = setup_contract(&e);
        let user = Address::generate(&e);
        token_client.mint(&user, &(100_000 * SCALAR_7));

        let balance_before = token_client.balance(&user);
        let id = place_limit_long(&e, &contract, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7);

        e.as_contract(&contract, || {
            super::execute_cancel_limit(&e, id);

            let positions = storage::get_user_positions(&e, &user);
            assert_eq!(positions.len(), 0);
        });

        // User should get full refund (collateral + fees)
        let balance_after = token_client.balance(&user);
        assert_eq!(balance_after, balance_before);
    }

}
