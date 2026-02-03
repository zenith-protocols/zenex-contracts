use crate::errors::TradingError;
use sep_40_oracle::Asset;
use soroban_sdk::{contracttype, panic_with_error, Address, Env};

#[contracttype]
#[derive(Clone, Debug)]
pub struct TradingConfig {
    pub oracle: Address,        // Address of the oracle contract
    pub caller_take_rate: i128, // Percentage of fee that a user gets for keeping the protocol running
    pub max_positions: u32,     // Maximum number of positions per user
    pub max_utilization: i128,  // Max leverage: total_notional / vault_assets (SCALAR_7). E.g., 20_000_000 = 2x, 50_000_000 = 5x. 0 = disabled
    pub max_price_age: u32,     // Maximum age of oracle price in seconds. Must be > oracle resolution
}

#[contracttype]
#[derive(Clone)]
pub struct MarketConfig {
    pub asset: Asset,         // The asset this market trades (immutable once set)
    pub enabled: bool,        // Whether trading is enabled for this asset
    pub max_payout: i128,     // Maximum payout percentage (with 7 decimals)
    pub min_collateral: i128, // Minimum collateral required for a position
    pub max_collateral: i128, // Maximum collateral allowed for a position

    pub init_margin: i128,        // Initial margin percentage (with 7 decimals)
    pub maintenance_margin: i128, // Maintenance margin percentage (with 7 decimals)

    pub base_fee: i128,            // 0.05% = 5_000 (in SCALAR_7)
    pub price_impact_scalar: i128, // BTC: 8_000_000_000, XLM: 700_000_000
    pub base_hourly_rate: i128,    // 0.001% = 10000000000000 (in SCALAR_18)
    pub ratio_cap: i128,           // Maximum long/short ratio for interest calculations (in SCALAR_18). E.g., 3 * SCALAR_18 = 3x cap
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
pub struct MarketData {
    // Long position data
    pub long_collateral: i128,    // Total collateral in long positions
    pub long_notional_size: i128, // Total notional size of long positions

    // Short position data
    pub short_collateral: i128,    // Total collateral in short positions
    pub short_notional_size: i128, // Total notional size of short positions

    // Interest rate tracking
    pub long_interest_index: i128, // Cumulative interest rate index for longs (with 18 decimals, starting at 10^18)
    pub short_interest_index: i128, // Cumulative interest rate index for shorts (with 18 decimals, starting at 10^18)
    pub last_update: u64,           // Last time the market data was updated
}

/// Structure to store information about a position
#[contracttype]
#[derive(Clone)]
pub struct Position {
    pub id: u32,         // Unique identifier for the position
    pub user: Address,   // Address of the user who owns this position
    pub filled: bool,    // Whether the position has been filled (false = pending limit order, true = open)
    pub asset_index: u32, // Index of the asset being traded (references market list)
    pub is_long: bool,          // Whether position is long (true) or short (false)
    pub stop_loss: i128,        // Stop loss price level, 0 if not set
    pub take_profit: i128,      // Take profit price level, 0 if not set
    pub entry_price: i128,      // Price at which position was opened
    pub collateral: i128,       // Amount of collateral provided by user
    pub notional_size: i128,    // Notional size of the position
    pub interest_index: i128,   // Interest index value when position was created or last updated
    pub created_at: u64,        // Timestamp when position was created
}

/// Request for keeper execution
#[contracttype]
#[derive(Clone)]
pub struct ExecuteRequest {
    pub request_type: u32,
    pub position_id: u32,
}

/// Types of keeper actions (permissionless)
#[derive(Clone, PartialEq)]
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

/// Contract operational status
#[derive(Clone, PartialEq)]
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

