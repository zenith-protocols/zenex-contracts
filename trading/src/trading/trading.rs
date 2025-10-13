use sep_40_oracle::{Asset, PriceFeedClient};
use soroban_sdk::{map, panic_with_error, vec, Address, Env, Map, Vec};
use soroban_fixed_point_math::SorobanFixedPoint;
use crate::trading::market::Market;
use crate::constants::{MAX_PRICE_AGE, SCALAR_7};
use crate::errors::TradingError;
use crate::{storage, Position};
use crate::types::TradingConfig;

pub struct Trading {
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

impl Trading {

    pub fn load(e: &Env, caller: Address) -> Self {
        let config = storage::get_config(e);
        let vault = storage::get_vault(e);
        let token = storage::get_token(e);
        Trading {
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
            Market::load(e, asset)
        };
        market.update_borrowing_index(e);
        market
    }

    pub fn load_position(&mut self, e: &Env, position_id: u32) -> Position {
        if let Some(position) = self.positions.get(position_id) {
            position
        } else {
            Position::load(e, position_id)
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
            let reserve = self
                .markets
                .get(asset)
                .unwrap();
            reserve.store(e);
        }
    }

    pub fn store_cached_positions(&mut self, e: &Env) {
        for position_id in self.positions_to_update.iter() {
            let position = self
                .positions
                .get(position_id)
                .unwrap();
            position.store(e);
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

    pub fn calculate_caller_fee(&self, e: &Env, fee: i128) -> i128 {
        let caller_fee = fee.fixed_mul_floor(e, &self.config.caller_take_rate, &SCALAR_7);
        if caller_fee > 0 {
            caller_fee
        } else {
            0
        }
    }
    
    pub fn check_max_positions(&self, positions: Vec<u32>) -> bool {
        positions.len() < self.config.max_positions
    }
}