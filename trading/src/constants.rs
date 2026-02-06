// Fixed-point precision constants
#[cfg(any(test, feature = "testutils"))]
pub const SCALAR_7: i128 = 10_000_000; // 7 decimal places (used in tests)
pub const SCALAR_18: i128 = 1_000_000_000_000_000_000; // 18 decimal places (interest precision)

// Trading limits
pub const MIN_LEVERAGE: i128 = 2; // 2x minimum leverage (multiply by token_scalar)

// Time constants
pub const ONE_HOUR_SECONDS: u64 = 3600;
pub const ONE_DAY_SECONDS: u64 = ONE_HOUR_SECONDS * 24; // 24 hours
pub const SECONDS_PER_WEEK: u64 = ONE_DAY_SECONDS * 7; // 7 days
