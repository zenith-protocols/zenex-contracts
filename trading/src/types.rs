use sep_40_oracle::Asset;
use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone, Debug)]
pub struct TradingConfig {
    pub status: u32,            // Status of the trading contract
    pub oracle: Address,        // Address of the oracle contract
    pub caller_take_rate: i128, // Percentage of fee that a user gets for keeping the protocol running
    pub max_positions: u32,     // Maximum number of positions per user
}

/// Position status
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PositionStatus {
    Pending,            // Limit order not yet filled
    Open,               // Position is open and active
    UserClosed,         // Closed manually by the user
    StopLossClosed,     // Closed due to stop loss trigger
    TakeProfitClosed,   // Closed due to take profit trigger
    Liquidated,         // Position was force-closed due to insufficient collateral
    Cancelled,          // Pending order cancelled before filling
}
#[contracttype]
#[derive(Clone)]
pub struct MarketConfig {
    pub enabled: bool,             // Whether trading is enabled for this asset
    pub max_leverage: u32,         // Maximum leverage allowed for this asset
    pub max_payout: i128,          // Maximum payout percentage (with 7 decimals)
    pub min_collateral: i128,      // Minimum collateral required for a position
    pub max_collateral: i128,      // Maximum collateral allowed for a position
    pub liquidation_threshold: i128, // Liquidation threshold (with 2 decimals)

    pub total_available: i128,     // Total amount available from vault for this market percentage (with 7 decimals)

    pub base_fee: i128,              // 0.05% = 5_000 (in SCALAR_7)
    pub price_impact_scalar: i128,   // BTC: 8_000_000_000, XLM: 700_000_000
    pub min_hourly_rate: i128,       // 0.0003% = 30
    pub max_hourly_rate: i128,       // BTC: 0.009% = 900, XLM: 0.016% = 1_600
    pub target_hourly_rate: i128,    // BTC: 0.001% = 100, XLM: 0.002% = 200
    pub target_utilization: i128,    // 80% = 8_000_000
}

#[derive(Clone)]
#[contracttype]
pub struct QueuedMarketInit {
    pub config: MarketConfig,
    pub unlock_time: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct MarketData {
    // Long position data
    pub long_collateral: i128,     // Total collateral in long positions
    pub long_borrowed: i128,       // Total borrowed funds in long positions
    pub long_count: u32,           // Number of open long positions

    // Short position data
    pub short_collateral: i128,    // Total collateral in short positions
    pub short_borrowed: i128,      // Total borrowed funds in short positions
    pub short_count: u32,          // Number of open short positions

    // Interest rate tracking
    pub long_interest_index: i128,      // Cumulative interest rate index for longs (with 18 decimals, starting at 10^18)
    pub short_interest_index: i128,     // Cumulative interest rate index for shorts (with 18 decimals, starting at 10^18)
    pub last_update: u64,          // Last time the market data was updated
}

/// Structure to store information about a position
#[contracttype]
#[derive(Clone)]
pub struct Position {
    pub id: u32,                 // Unique identifier for the position
    pub user: Address,           // Address of the user who owns this position
    pub status: PositionStatus,  // Current status of the position
    pub asset: Asset,            // The asset being traded
    pub is_long: bool,           // Whether position is long (true) or short (false)
    pub stop_loss: i128,         // Stop loss price level, 0 if not set
    pub take_profit: i128,       // Take profit price level, 0 if not set
    pub entry_price: i128,       // Price at which position was opened
    pub leverage: u32,           // Leverage multiplier
    pub collateral: i128,        // Amount of collateral provided by user
    pub position_index: i128,    // Interest index value when position was created or last updated
    pub timestamp: u64,          // Timestamp when position was created
}