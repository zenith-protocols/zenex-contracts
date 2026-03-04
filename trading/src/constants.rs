// Fixed-point precision constants
#[cfg(any(test, feature = "testutils"))]
pub const SCALAR_7: i128 = 10_000_000; // 7 decimal places (used in tests)
pub const SCALAR_18: i128 = 1_000_000_000_000_000_000; // 18 decimal places (interest precision)

// Trading limits
pub const MAINTENANCE_MARGIN_DIVISOR: i128 = 200; // 0.5% = token_scalar / 200
pub const MIN_LEVERAGE: i128 = 2; // 2x minimum leverage (multiply by token_scalar)
pub const MAX_MARKETS: u32 = 32; // Maximum number of markets
pub const MAX_POSITIONS: u32 = 25; // Maximum number of positions per user
pub const MAX_PRICE_AGE: u32 = 900; // Maximum oracle price age in seconds (15 minutes)
pub const UTILIZATION_THRESHOLD_NUM: i128 = 9; // 90% threshold: net_pnl * 10 >= vault * 9
pub const UTILIZATION_THRESHOLD_DEN: i128 = 10;

// Time constants
pub const ONE_HOUR_SECONDS: u64 = 3600;
