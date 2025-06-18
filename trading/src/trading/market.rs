// Updated contracts/trading/src/trading/market.rs
use sep_40_oracle::Asset;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{contracttype, Env};
use soroban_sdk::token::TokenClient;
use crate::constants::SCALAR_7;
use crate::storage;
use crate::types::{MarketConfig, MarketData};
use crate::trading::fees::update_borrowing_indices;

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
        let total_borrowed = self.data.long_borrowed + self.data.short_borrowed;

        // Get vault's total token balance
        let vault_balance = TokenClient::new(e, &storage::get_token(e)).balance(&storage::get_vault(e));

        // allocated_liquidity = vault_balance × total_available_percentage
        let allocated_liquidity = vault_balance.fixed_mul_floor(e, &self.config.total_available, &SCALAR_7);

        if allocated_liquidity == 0 {
            return SCALAR_7; // If no liquidity allocated, utilization is 100%
        }
        if total_borrowed == 0 {
            return 0;
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
    pub fn update_stats(&mut self, e: &Env, collateral: i128, borrowed: i128, is_long: bool) {
        // First update the market stats
        if is_long {
            self.data.long_borrowed += borrowed;
            self.data.long_collateral += collateral;
            if borrowed > 0 {
                self.data.long_count += 1;
            } else if borrowed < 0 {
                // Closing position
                self.data.long_count -= 1;
            }
        } else {
            self.data.short_borrowed += borrowed;
            self.data.short_collateral += collateral;
            if borrowed > 0 {
                self.data.short_count += 1;
            } else if borrowed < 0 {
                self.data.short_count -= 1;
            }
        }

        self.update_borrowing_index(e);
    }

    /// Check if position size is within allowed range
    pub fn is_collateral_valid(&self, collateral: i128) -> bool {
        collateral >= self.config.min_collateral && collateral <= self.config.max_collateral
    }

    /// Check if leverage is within allowed range
    pub fn is_leverage_valid(&self, leverage: u32) -> bool {
        leverage > 100 && leverage <= self.config.max_leverage
    }

    pub fn can_liquidate(&self, e: &Env, collateral: i128, pnl: i128) -> bool {
        if pnl >= 0 {
            return false; // Can't liquidate profitable positions
        }

        let remaining_value = collateral + pnl;
        let minimum_required = collateral.fixed_mul_floor(e, &self.config.liquidation_threshold, &SCALAR_7);

        remaining_value < minimum_required
    }
}