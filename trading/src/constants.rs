pub const SCALAR_7: i128 = 10_000_000; // 7-decimal scalar: rates, fees, ratios, utilization, margins
pub const SCALAR_18: i128 = 1_000_000_000_000_000_000; // 18-decimal scalar: cumulative indices (funding, borrowing, ADL)

pub const MAX_ENTRIES: u32 = 50; // max markets or positions per user

pub const UTIL_ONICE: i128 = 9_500_000; // enter OnIce when net PnL >= 95% of vault (SCALAR_7)
pub const UTIL_ACTIVE: i128 = 9_000_000; // restore Active when net PnL < 90% of vault (SCALAR_7)

pub const ONE_HOUR_SECONDS: u64 = 3600; // seconds per hour, for rate accrual conversion
pub const MIN_OPEN_TIME: u64 = 30; // min seconds before user-initiated close (prevents same-block arbitrage)
pub const MAX_CALLER_RATE: i128 = 5_000_000; // 50% of trading fees (SCALAR_7)
pub const MAX_FEE_RATE: i128 = 100_000; // 1% of notional (SCALAR_7)
pub const MAX_RATE_HOURLY: i128 = 100_000_000_000_000; // 0.01%/hr (~88% APR, SCALAR_18)
pub const MAX_R_VAR: i128 = 100_000_000_000_000; // max vault/market variable rate: 0.01%/hr (SCALAR_18)
pub const MAX_UTIL: i128 = 100_000_000; // 1000% global util cap (10 * SCALAR_7)
pub const MIN_IMPACT: i128 = 100_000_000; // impact divisor floor: caps impact fee at 10% (10 * SCALAR_7)
pub const MAX_MARGIN: i128 = 5_000_000; // 50% init margin = 2x min leverage (SCALAR_7)
pub const MAX_LIQ_FEE: i128 = 2_500_000; // 25% max liquidation fee/threshold (SCALAR_7)
pub const MAX_R_VAR_MARKET: i128 = 100_000_000_000_000; // max per-market variable rate: 0.01%/hr (SCALAR_18)
