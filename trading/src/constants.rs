// Fixed-point precision constants
pub const SCALAR_7: i128 = 10_000_000; // 7 decimal places (rates, fees, ratios)
pub const SCALAR_18: i128 = 1_000_000_000_000_000_000; // 18 decimal places (interest precision)

// Trading limits
pub const MAINTENANCE_MARGIN_DIVISOR: i128 = 200; // 0.5% = SCALAR_7 / 200
pub const MIN_LEVERAGE: i128 = 2; // 2x minimum leverage (multiply by SCALAR_7)
pub const MAX_MARKETS: u32 = 32; // Maximum number of markets
pub const MAX_POSITIONS: u32 = 25; // Maximum number of positions per user
/// Circuit breaker: freeze when net_pnl >= 95%, reactivate when < 90%.
/// Hysteresis prevents flapping between Active and OnIce.
pub const UTIL_FREEZE: i128 = 9_500_000;   // 95% in SCALAR_7
pub const UTIL_UNFREEZE: i128 = 9_000_000; // 90% in SCALAR_7

// Time constants
pub const ONE_HOUR_SECONDS: u64 = 3600;

// Staleness thresholds (seconds)
pub const MAX_STALENESS_USER: u64 = 10;     // user-facing actions (close, modify, triggers)
pub const MAX_STALENESS_KEEPER: u64 = 300;  // keeper/automated actions (execute, ADL, circuit breaker)
