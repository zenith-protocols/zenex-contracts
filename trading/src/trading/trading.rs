use sep_40_oracle::{Asset, PriceFeedClient};
use soroban_sdk::{map, panic_with_error, vec, Address, Env, Map, Vec};
use soroban_fixed_point_math::SorobanFixedPoint;
use crate::trading::market::Market;
use crate::constants::{MAX_PRICE_AGE, SCALAR_7};
use crate::errors::TradingError;
use crate::events::TradingEvents;
use crate::storage;
use crate::types::TradingConfig;

pub struct Trading {
    pub config: TradingConfig,
    pub markets: Map<Asset, Market>,
    pub markets_to_update: Vec<Asset>,
    prices: Map<Asset, i128>,
}

impl Trading {

    pub fn load(e: &Env) -> Self {
        let config = storage::get_config(e);
        Trading {
            config,
            markets: map![e],
            markets_to_update: vec![e],
            prices: map![e],
        }
    }
    
    pub fn update_status(&mut self, e: &Env, status: u32) {
        self.config.status = status;
        storage::set_config(e, &self.config);
    }

    pub fn load_market(&mut self, e: &Env, asset: &Asset, store: bool) -> Market {
        if store && !self.markets_to_update.contains(asset) {
            self.markets_to_update.push_back(asset.clone());
        }

        if let Some(market) = self.markets.get(asset.clone()) {
            market
        } else {
            Market::load(e, asset)
        }
    }

    pub fn cache_market(&mut self, market: &Market) {
        self.markets.set(market.asset.clone(), market.clone());
        if !self.markets_to_update.contains(&market.asset) {
            self.markets_to_update.push_back(market.asset.clone());
        }
    }

    pub fn store_cached_markets(&mut self, e: &Env) {
        for asset in self.markets_to_update.iter() {
            let reserve = self
                .markets
                .get(asset)
                .unwrap();
            reserve.store(e);
        }
    }

    pub fn load_price(&mut self, e: &Env, asset: &Asset) -> i128 {
        if let Some(price) = self.prices.get(asset.clone()) {
            return price;
        }
        let price_data = match PriceFeedClient::new(e, &self.config.oracle).lastprice(asset) {
            Some(price) => price,
            None => panic_with_error!(e, TradingError::NoPrice),
        };
        if price_data.timestamp + MAX_PRICE_AGE < e.ledger().timestamp() {
            panic_with_error!(e, TradingError::StalePrice);
        }

        self.prices.set(asset.clone(), price_data.price);
        price_data.price
    }

    pub fn calculate_spender_fee(&self, e: &Env, fee: i128) -> i128 {
        let spender_fee = fee.fixed_mul_floor(e, &self.config.caller_take_rate, &SCALAR_7);
        if spender_fee > 0 {
            spender_fee
        } else {
            0
        }
    }
    
    pub fn check_max_positions(&self, positions: Vec<u32>) -> bool {
        positions.len() < self.config.max_positions
    }
}

pub fn execute_set_status(
    e: &Env,
    admin: &Address, // Admin is not used in this function, but can be used for authorization if needed
    status: u32,
) {
    let mut trading = Trading::load(e);
    //TODO: Implement status update logic to check if status is allowed
    trading.update_status(e, status);
    TradingEvents::set_status(e, admin.clone(), status);
}