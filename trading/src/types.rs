use crate::errors::TradingError;
use sep_40_oracle::Asset;
use soroban_sdk::{contracttype, panic_with_error, Address, Env};

#[contracttype]
#[derive(Clone, Debug)]
pub struct TradingConfig {
    pub oracle: Address,        // Address of the oracle contract
    pub max_price_age: u32,     // Maximum age of oracle price in seconds
    pub caller_take_rate: i128, // Percentage of fee given to caller (SCALAR_7)
    pub max_positions: u32,     // Maximum number of positions per user
    pub max_utilization: i128,  // Max total_notional / vault_assets ratio (SCALAR_7)
}

#[contracttype]
#[derive(Clone)]
pub struct ConfigUpdate {
    pub config: TradingConfig,
    pub unlock_time: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct QueuedMarketInit {
    pub config: MarketConfig,
    pub unlock_time: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct MarketConfig {
    pub asset: Asset,             // The asset this market trades (immutable once set)
    pub enabled: bool,            // Whether trading is enabled for this asset
    pub max_payout: i128,         // Maximum payout as ratio of collateral (SCALAR_7)
    pub min_collateral: i128,     // Minimum collateral required (SCALAR_7)
    pub max_collateral: i128,     // Maximum collateral allowed (SCALAR_7)
    pub init_margin: i128,        // Initial margin requirement, determines max leverage (1/init_margin) (SCALAR_7)
    pub maintenance_margin: i128, // Maintenance margin threshold, determines liquidation price (SCALAR_7)
    pub base_fee: i128,           // Base trading fee percentage (SCALAR_7)
    pub price_impact_scalar: i128, // Divisor for price impact calculation (SCALAR_7)
    pub base_hourly_rate: i128,   // Base hourly interest rate (SCALAR_18)
    pub ratio_cap: i128,          // Maximum long/short ratio for interest (SCALAR_18)
}

#[contracttype]
#[derive(Clone)]
pub struct MarketData {
    pub long_notional_size: i128,   // Total notional size of long positions (SCALAR_7)
    pub short_notional_size: i128,  // Total notional size of short positions (SCALAR_7)
    pub long_interest_index: i128,  // Cumulative interest index for longs (SCALAR_18)
    pub short_interest_index: i128, // Cumulative interest index for shorts (SCALAR_18)
    pub last_update: u64,           // Last update timestamp
}

#[contracttype]
#[derive(Clone)]
pub struct Position {
    pub id: u32,              // Unique identifier
    pub user: Address,        // Owner address
    pub filled: bool,         // Whether filled (false = pending limit order)
    pub asset_index: u32,     // Index of the traded asset
    pub is_long: bool,        // Long (true) or short (false)
    pub stop_loss: i128,      // Stop loss price, 0 if not set (SCALAR_7)
    pub take_profit: i128,    // Take profit price, 0 if not set (SCALAR_7)
    pub entry_price: i128,    // Entry price (SCALAR_7)
    pub collateral: i128,     // Collateral amount (SCALAR_7)
    pub notional_size: i128,  // Notional size (SCALAR_7)
    pub interest_index: i128, // Interest index at creation (SCALAR_18)
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
    OnIce = 1,
    Frozen = 2,
    Setup = 99,
}

impl ContractStatus {
    pub fn from_u32(e: &Env, value: u32) -> Self {
        match value {
            0 => ContractStatus::Active,
            1 => ContractStatus::OnIce,
            2 => ContractStatus::Frozen,
            99 => ContractStatus::Setup,
            _ => panic_with_error!(e, TradingError::InvalidStatus),
        }
    }
}