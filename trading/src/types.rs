use crate::constants::SCALAR_18;
use crate::errors::TradingError;
use soroban_sdk::{contracttype, panic_with_error, Address, Env};

#[contracttype]
#[derive(Clone, Debug)]
pub struct TradingConfig {
    pub caller_take_rate: i128,      // Percentage of fee given to caller (SCALAR_7)
    pub min_open_time: u64,          // Minimum seconds a position must be open before closing (0 = disabled)
    pub vault_skim: i128,            // Vault's cut of funding spread (SCALAR_7). 0_2000000 = 20%
    pub min_collateral: i128,        // Minimum collateral required (token_decimals)
    pub max_collateral: i128,        // Maximum collateral allowed (token_decimals)
    pub max_payout: i128,            // Maximum payout as ratio of collateral (SCALAR_7)
    pub base_fee_dominant: i128,     // Fee rate for dominant side (SCALAR_7)
    pub base_fee_non_dominant: i128, // Fee rate for non-dominant side (SCALAR_7)
}

#[contracttype]
#[derive(Clone)]
pub struct MarketConfig {
    pub enabled: bool,             // Whether trading is enabled for this asset
    pub init_margin: i128,         // Initial margin requirement, determines max leverage (1/init_margin) (SCALAR_7)
    pub base_hourly_rate: i128,    // Base hourly interest rate (SCALAR_18)
    pub price_impact_scalar: i128, // Divisor for price impact calculation (SCALAR_7)
}

#[contracttype]
#[derive(Clone)]
pub struct MarketData {
    pub long_notional_size: i128,  // Total notional size of long positions (token_decimals)
    pub short_notional_size: i128, // Total notional size of short positions (token_decimals)
    pub long_funding_index: i128,  // Cumulative funding index for longs (SCALAR_18)
    pub short_funding_index: i128, // Cumulative funding index for shorts (SCALAR_18)
    pub long_entry_weighted: i128,  // Σ(notional_i / entry_price_i) for longs (token_decimals)
    pub short_entry_weighted: i128, // Σ(notional_i / entry_price_i) for shorts (token_decimals)
    pub funding_rate: i128,        // Current signed funding rate (SCALAR_18), positive=longs pay, negative=shorts pay
    pub last_update: u64,          // Last update timestamp
    pub long_adl_index: i128,      // Cumulative long-side ADL reduction index (SCALAR_18)
    pub short_adl_index: i128,     // Cumulative short-side ADL reduction index (SCALAR_18)
}

impl Default for MarketData {
    fn default() -> Self {
        Self {
            long_notional_size: 0,
            short_notional_size: 0,
            long_funding_index: 0,
            short_funding_index: 0,
            long_entry_weighted: 0,
            short_entry_weighted: 0,
            funding_rate: 0,
            last_update: 0,
            long_adl_index: SCALAR_18,
            short_adl_index: SCALAR_18,
        }
    }
}

#[contracttype]
#[derive(Clone)]
pub struct Position {
    pub user: Address,        // Owner address
    pub filled: bool,         // Whether filled (false = pending limit order)
    pub feed_id: u32,         // Pyth feed ID (e.g. 1=BTC, 2=ETH)
    pub is_long: bool,        // Long (true) or short (false)
    pub stop_loss: i128,      // Stop loss price, 0 if not set (price_decimals)
    pub take_profit: i128,    // Take profit price, 0 if not set (price_decimals)
    pub entry_price: i128,    // Entry price (price_decimals)
    pub collateral: i128,     // Collateral amount (token_decimals)
    pub notional_size: i128,  // Notional size (token_decimals)
    pub entry_funding_index: i128, // Funding index at creation (SCALAR_18)
    pub entry_adl_index: i128, // Market's side ADL index at position entry (SCALAR_18)
    pub created_at: u64,      // Creation timestamp
}

#[contracttype]
#[derive(Clone)]
pub struct ExecuteRequest {
    pub request_type: u32,
    pub position_id: u32,
}

#[derive(Clone, PartialEq, Debug)]
#[repr(u32)]
pub enum ExecuteRequestType {
    Fill = 0,
    StopLoss = 1,
    TakeProfit = 2,
    Liquidate = 3,
}

impl ExecuteRequestType {
    pub fn from_u32(e: &Env, value: u32) -> Self {
        match value {
            0 => ExecuteRequestType::Fill,
            1 => ExecuteRequestType::StopLoss,
            2 => ExecuteRequestType::TakeProfit,
            3 => ExecuteRequestType::Liquidate,
            _ => panic_with_error!(e, TradingError::InvalidRequestType),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
#[repr(u32)]
pub enum ContractStatus {
    Active = 0,
    OnIce = 1,       // Permissionless circuit breaker (PnL threshold)
    AdminOnIce = 2,  // Admin-set on ice (only admin can lift)
    Frozen = 3,      // Admin-set full freeze
}

impl ContractStatus {
    pub fn from_u32(e: &Env, value: u32) -> Self {
        match value {
            0 => ContractStatus::Active,
            1 => ContractStatus::OnIce,
            2 => ContractStatus::AdminOnIce,
            3 => ContractStatus::Frozen,
            _ => panic_with_error!(e, TradingError::InvalidStatus),
        }
    }
}
