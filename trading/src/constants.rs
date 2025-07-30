// Token precision
pub const SCALAR_7: i128 = 10_000_000; // 7 decimal places
pub const SCALAR_14: i128 = 100_000_000_000_000; // 14 decimal places
pub const SCALAR_18: i128 = 1_000_000_000_000_000_000; // 18 decimal places
// Time constants
pub const ONE_HOUR_SECONDS: u64 = 3600;
pub const ONE_DAY_SECONDS: u64 = ONE_HOUR_SECONDS * 24; // 24 hours
pub const SECONDS_PER_WEEK: u64 = ONE_DAY_SECONDS * 7; // 7 days

// Limits
pub const MAX_ACTIONABLE_POSITIONS: u32 = 50; // Max positions to track for keeper actions
// Oracle
pub const MAX_PRICE_AGE: u64 = 300; // Max price age in seconds (5 minutes)