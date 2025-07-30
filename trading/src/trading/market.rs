// Updated contracts/trading/src/trading/market.rs
use sep_40_oracle::Asset;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{contracttype, Env};
use soroban_sdk::token::TokenClient;
use crate::constants::SCALAR_7;
use crate::dependencies::VaultClient;
use crate::{storage, Position};
use crate::types::{MarketConfig, MarketData};
use crate::trading::fees::update_borrowing_indices;
use crate::trading::interest::base_hourly_interest_rate;

#[derive(Clone)]
#[contracttype]
pub struct Market {
    pub asset: Asset,         // the asset for this market
    pub config: MarketConfig, // the reserve configuration
    pub data: MarketData,     // the reserve data
}

/// Implementation of methods and functionality for Market
impl Market {

    pub fn load(e: &Env, asset: &Asset) -> Market {
        let market_config = storage::get_market_config(e, asset);
        let market_data = storage::get_market_data(e, asset);
        Market {
            asset: asset.clone(),
            config: market_config,
            data: market_data,
        }
    }

    pub fn store(&self, e: &Env) {
        storage::set_market_data(e, &self.asset, &self.data);
    }

    pub fn utilization(&self, e: &Env) -> i128 {
        let total_borrowed = self.data.long_notional_size + self.data.short_notional_size -
            self.data.long_collateral - self.data.short_collateral;

        if total_borrowed == 0 {
            return 0;
        }

        // Get vault's total token balance
        let vault_balance = VaultClient::new(e, &storage::get_vault(e)).total_tokens();

        // allocated_liquidity = vault_balance × total_available_percentage
        let allocated_liquidity = vault_balance.fixed_mul_floor(e, &self.config.total_available, &SCALAR_7);

        if allocated_liquidity == 0 {
            return SCALAR_7; // If no liquidity allocated, utilization is 100%
        }


        // Calculate utilization as borrowed/allocated
        let utilization = total_borrowed.fixed_div_floor(e, &allocated_liquidity, &SCALAR_7);
        if utilization > SCALAR_7 {
            SCALAR_7
        } else {
            utilization
        }
    }

    /// Updates borrowing rates and indexes for the market
    pub fn update_borrowing_index(&mut self, e: &Env) {
        let current_time = e.ledger().timestamp();
        let time_delta_seconds = current_time - self.data.last_update;

        // Skip update if no time has passed
        if time_delta_seconds == 0 {
            return;
        }

        // Calculate current utilization
        let utilization = self.utilization(e);

        let base_hourly_rate = base_hourly_interest_rate(
            e,
            utilization,
            self.config.min_hourly_rate,
            self.config.target_hourly_rate,
            self.config.max_hourly_rate,
            self.config.target_utilization,
        );

        let long_average_leverage = self.data.long_notional_size.fixed_div_floor(e, &self.data.long_collateral, &SCALAR_7);
        let short_average_leverage = self.data.short_notional_size.fixed_div_floor(e, &self.data.short_collateral, &SCALAR_7);

        let short_leverage_multiplier = if short_average_leverage > 0 {
            pow_int(e, &I256::from_i128(e, 101), &short_average_leverage as u32)
        } else {
            I256::from_i128(e, 100) // 1.0 in SCALAR_18
        };


        let base_hourly_rate = 0;
        let leveraage_multiplier = 1;
        let short_ratio = 0;
        let long_ratio = 1 - short_ratio;

        let interest_base = base_hourly_rate * time_delta_seconds * leveraage_multiplier;
        let long_interest = interest_base * long_ratio;
        let short_interest = interest_base * short_ratio;




        // Update indices using the main function
        let (new_long_index, new_short_index) = update_borrowing_indices(
            e,
            time_delta_seconds,
            utilization,
            self.data.long_collateral,
            self.data.long_borrowed,
            self.data.short_collateral,
            self.data.short_borrowed,
            self.data.long_interest_index,
            self.data.short_interest_index,
            self.config.min_hourly_rate,
            self.config.max_hourly_rate,
            self.config.target_hourly_rate,
            self.config.target_utilization,
        );

        // Update market data
        self.data.long_interest_index = new_long_index;
        self.data.short_interest_index = new_short_index;
        self.data.last_update = current_time;
    }

    /// Updates open interest statistics for an asset
    /// Use positive values to add, negative values to subtract
    pub fn update_stats(&mut self, e: &Env, collateral: i128, notional_size: i128, is_long: bool) {
        if is_long {
            self.data.long_notional_size += notional_size;
            self.data.long_collateral += collateral;

            // If notional size is 0 user is adjusting position we dont adjust counts
            // If notional size is positive, user is opening a new position
            // If notional size is negative, user is closing a position
            if notional_size > 0 {
                self.data.long_count += 1;
            } else if notional_size < 0 {
                self.data.long_count -= 1;
            }
        } else {
            self.data.short_notional_size += notional_size;
            self.data.short_collateral += collateral;
            if notional_size > 0 {
                self.data.short_count += 1;
            } else if notional_size < 0 {
                self.data.short_count -= 1;
            }
        }

        self.update_borrowing_index(e);
    }

    /// Check if position size is within allowed range
    pub fn is_position_valid(&self, collateral: i128) -> bool {
        collateral >= self.config.min_collateral && collateral <= self.config.max_collateral
    }
}