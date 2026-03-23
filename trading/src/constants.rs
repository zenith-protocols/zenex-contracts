// Fixed-point precision constants
pub const SCALAR_7: i128 = 10_000_000; // 7 decimal places (rates, fees, ratios)
pub const SCALAR_18: i128 = 1_000_000_000_000_000_000; // 18 decimal places (interest precision)

// Storage limits
pub const MAX_ENTRIES: u32 = 50; // Maximum markets or positions per user

/// Circuit breaker: enter OnIce when net_pnl >= 95% of vault,
/// restore Active when net_pnl < 90%. Hysteresis prevents flapping.
pub const UTIL_ONICE: i128 = 9_500_000;  // 95% in SCALAR_7
pub const UTIL_ACTIVE: i128 = 9_000_000; // 90% in SCALAR_7

// Time constants
pub const ONE_HOUR_SECONDS: u64 = 3600;
pub const MIN_OPEN_TIME: u64 = 30; // Minimum seconds a position must be open before closing

// TradingConfig caps
pub const MAX_FEE_RATE: i128 = 100_000;                  // 1% of notional
pub const MAX_CALLER_RATE: i128 = 5_000_000;              // 50% of trading fees
pub const MAX_RATE_HOURLY: i128 = 100_000_000_000_000;    // 0.01%/hr (~88% annually) in SCALAR_18
pub const MAX_R_VAR: i128 = 100_000_000;                  // 10x multiplier at full util (10 * SCALAR_7)
pub const MAX_UTIL: i128 = 100_000_000;                    // 1000% (10x vault) — notional can exceed vault balance

// MarketConfig caps
pub const MIN_IMPACT: i128 = 100_000_000;                 // 10 * SCALAR_7 — caps impact fee at 10% of notional
pub const MAX_MARGIN: i128 = 5_000_000;                   // 50% — 2x min leverage
pub const MAX_LIQ_FEE: i128 = 2_500_000;                 // 25% liquidation threshold
pub const MAX_R_BORROW: i128 = 100_000_000;               // 10x weight (10 * SCALAR_7)
