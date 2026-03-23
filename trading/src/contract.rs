#![allow(clippy::too_many_arguments)]

use crate::errors::TradingError;
use crate::interface::Trading;
use crate::dependencies::PriceVerifierClient;
use crate::trading::ExecuteRequest;
use crate::types::{MarketConfig, MarketData, Position, TradingConfig};
use crate::{storage, trading, ContractStatus};
use crate::validation::require_valid_config;
use soroban_sdk::{contract, contractimpl, panic_with_error, Address, Bytes, Env, Vec};
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_contract_utils::upgradeable::UpgradeableInternal;
use stellar_macros::{only_owner, Upgradeable};

use crate::dependencies::PriceData;

#[derive(Upgradeable)]
#[contract]
pub struct TradingContract;

fn verify_price(e: &Env, price: &Bytes) -> PriceData {
    PriceVerifierClient::new(e, &storage::get_price_verifier(e))
        .verify_prices(price)
        .get(0)
        .unwrap()
}

fn verify_prices(e: &Env, price: &Bytes) -> Vec<PriceData> {
    PriceVerifierClient::new(e, &storage::get_price_verifier(e)).verify_prices(price)
}

#[contractimpl]
impl TradingContract {
    pub fn __constructor(
        e: Env,
        owner: Address,
        token: Address,
        vault: Address,
        price_verifier: Address,
        treasury: Address,
        config: TradingConfig,
    ) {
        require_valid_config(&e, &config);
        ownable::set_owner(&e, &owner);
        storage::set_vault(&e, &vault);
        storage::set_token(&e, &token);
        storage::set_price_verifier(&e, &price_verifier);
        storage::set_treasury(&e, &treasury);
        storage::set_config(&e, &config);
        storage::set_status(&e, ContractStatus::Active as u32);
    }
}

#[contractimpl]
impl Trading for TradingContract {
    #[only_owner]
    fn set_config(e: Env, config: TradingConfig) {
        storage::extend_instance(&e);
        trading::execute_set_config(&e, &config);
    }

    #[only_owner]
    fn set_market(e: Env, feed_id: u32, config: MarketConfig) {
        storage::extend_instance(&e);
        trading::execute_set_market(&e, feed_id, &config);
    }

    #[only_owner]
    fn del_market(e: Env, feed_id: u32) {
        storage::extend_instance(&e);
        trading::execute_del_market(&e, feed_id);
    }

    #[only_owner]
    fn set_status(e: Env, status: u32) {
        storage::extend_instance(&e);
        trading::execute_set_status(&e, status);
    }

    fn update_status(e: Env, price: Bytes) {
        storage::extend_instance(&e);
        trading::execute_update_status(&e, &verify_prices(&e, &price));
    }

    fn place_limit(
        e: Env,
        user: Address,
        feed_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
        take_profit: i128,
        stop_loss: i128,
    ) -> u32 {
        storage::extend_instance(&e);
        trading::execute_create_limit(
            &e, &user, feed_id, collateral, notional_size, is_long,
            entry_price, take_profit, stop_loss,
        )
    }

    fn open_market(
        e: Env,
        user: Address,
        feed_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        take_profit: i128,
        stop_loss: i128,
        price: Bytes,
    ) -> u32 {
        storage::extend_instance(&e);
        let pd = verify_price(&e, &price);
        if pd.feed_id != feed_id {
            panic_with_error!(e, TradingError::InvalidPrice);
        }
        trading::execute_create_market(
            &e, &user, collateral, notional_size, is_long,
            take_profit, stop_loss, &pd,
        )
    }

    fn cancel_limit(e: Env, position_id: u32) -> i128 {
        storage::extend_instance(&e);
        trading::execute_cancel_limit(&e, position_id)
    }

    fn close_position(e: Env, position_id: u32, price: Bytes) -> i128 {
        storage::extend_instance(&e);
        trading::execute_close_position(&e, position_id, &verify_price(&e, &price))
    }

    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128, price: Bytes) {
        storage::extend_instance(&e);
        trading::execute_modify_collateral(&e, position_id, new_collateral, &verify_price(&e, &price));
    }

    fn set_triggers(e: Env, position_id: u32, take_profit: i128, stop_loss: i128) {
        storage::extend_instance(&e);
        trading::execute_set_triggers(&e, position_id, take_profit, stop_loss);
    }

    fn execute(e: Env, caller: Address, requests: Vec<ExecuteRequest>, price: Bytes) {
        storage::extend_instance(&e);
        trading::execute_trigger(&e, &caller, requests, &verify_price(&e, &price));
    }

    fn apply_funding(e: Env) {
        storage::extend_instance(&e);
        trading::execute_apply_funding(&e);
    }

    fn get_position(e: Env, position_id: u32) -> Position {
        storage::get_position(&e, position_id)
    }

    fn get_user_positions(e: Env, user: Address) -> Vec<u32> {
        storage::get_user_positions(&e, &user)
    }

    fn get_market_config(e: Env, feed_id: u32) -> MarketConfig {
        storage::get_market_config(&e, feed_id)
    }

    fn get_market_data(e: Env, feed_id: u32) -> MarketData {
        storage::get_market_data(&e, feed_id)
    }

    fn get_markets(e: Env) -> Vec<u32> {
        storage::get_markets(&e)
    }

    fn get_config(e: Env) -> TradingConfig {
        storage::get_config(&e)
    }

    fn get_status(e: Env) -> u32 {
        storage::get_status(&e)
    }

    fn get_vault(e: Env) -> Address {
        storage::get_vault(&e)
    }

    fn get_price_verifier(e: Env) -> Address {
        storage::get_price_verifier(&e)
    }

    fn get_treasury(e: Env) -> Address {
        storage::get_treasury(&e)
    }

    fn get_token(e: Env) -> Address {
        storage::get_token(&e)
    }
}

#[contractimpl(contracttrait)]
impl Ownable for TradingContract {}

impl UpgradeableInternal for TradingContract {
    fn _require_auth(e: &Env, operator: &Address) {
        operator.require_auth();
        let owner = ownable::get_owner(e).expect("owner not set");
        if *operator != owner {
            panic_with_error!(e, TradingError::Unauthorized)
        }
    }
}
