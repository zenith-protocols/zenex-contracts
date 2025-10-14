use sep_40_oracle::Asset;
use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone, Debug)]
pub struct TradingConfig {
    pub oracle: Address,        // Address of the oracle contract
    pub caller_take_rate: i128, // Percentage of fee that a user gets for keeping the protocol running
    pub max_positions: u32,     // Maximum number of positions per user
}

/// Position status
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PositionStatus {
    Pending, // Limit order not yet filled
    Open,    // Position is open
    Closed,  // Position closed
}

#[contracttype]
#[derive(Clone)]
pub struct MarketConfig {
    pub enabled: bool,        // Whether trading is enabled for this asset
    pub max_payout: i128,     // Maximum payout percentage (with 7 decimals)
    pub min_collateral: i128, // Minimum collateral required for a position
    pub max_collateral: i128, // Maximum collateral allowed for a position

    pub init_margin: i128,        // Initial margin percentage (with 7 decimals)
    pub maintenance_margin: i128, // Maintenance margin percentage (with 7 decimals)

    pub base_fee: i128,            // 0.05% = 5_000 (in SCALAR_7)
    pub price_impact_scalar: i128, // BTC: 8_000_000_000, XLM: 700_000_000
    pub base_hourly_rate: i128,    // 0.001% = 10000000000000 (in SCALAR_18)
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
    pub id: u32,                // Unique identifier for the position
    pub user: Address,          // Address of the user who owns this position
    pub status: PositionStatus, // Current status of the position
    pub asset: Asset,           // The asset being traded
    pub is_long: bool,          // Whether position is long (true) or short (false)
    pub stop_loss: i128,        // Stop loss price level, 0 if not set
    pub take_profit: i128,      // Take profit price level, 0 if not set
    pub entry_price: i128,      // Price at which position was opened
    pub collateral: i128,       // Amount of collateral provided by user
    pub notional_size: i128,    // Notional size of the position
    pub interest_index: i128,   // Interest index value when position was created or last updated
    pub created_at: u64,        // Timestamp when position was created
}
