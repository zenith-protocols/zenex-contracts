// ── Fixed-point precision constants ──────────────────────────────────
//
// WHY: Two precision tiers -- SCALAR_7 for rates/fees/ratios that don't
// compound, SCALAR_18 for cumulative indices that accrue over time.
// Interest indices need 18 decimals because small hourly deltas (e.g. 0.001%)
// accumulate over weeks/months and would lose significant precision at 7 decimals.
// Rates and fees are bounded (< 1x) so 7 decimals provide sufficient resolution.

/// 7-decimal fixed-point scalar for rates, fees, ratios, utilization, margins.
pub const SCALAR_7: i128 = 10_000_000;
/// 18-decimal fixed-point scalar for cumulative interest indices (funding, borrowing, ADL).
pub const SCALAR_18: i128 = 1_000_000_000_000_000_000;

// ── Storage limits ──────────────────────────────────────────────────

/// Maximum markets or positions per user. Prevents unbounded iteration in storage.
pub const MAX_ENTRIES: u32 = 50;

// ── Circuit breaker thresholds ──────────────────────────────────────
//
// WHY: Hysteresis (5% gap between enter/exit thresholds) prevents the contract
// from flapping between Active and OnIce when PnL hovers near a single threshold.
// 95% enter, 90% exit means the system must meaningfully recover before re-enabling.

/// Enter OnIce when net trader PnL >= 95% of vault balance (SCALAR_7).
pub const UTIL_ONICE: i128 = 9_500_000;
/// Restore Active when net trader PnL < 90% of vault balance (SCALAR_7).
pub const UTIL_ACTIVE: i128 = 9_000_000;

// ── Time constants ──────────────────────────────────────────────────

/// Seconds per hour -- used to convert hourly rates to per-second accrual.
pub const ONE_HOUR_SECONDS: u64 = 3600;
/// Minimum seconds a position must be open before user-initiated close.
/// WHY: Prevents same-block open+close price arbitrage. A user could otherwise
/// open and close with the same oracle price, extracting risk-free profit from
/// fee asymmetries or rounding. 30 seconds ensures at least one ledger gap.
pub const MIN_OPEN_TIME: u64 = 30;

// ── TradingConfig upper bounds ──────────────────────────────────────

/// Max keeper caller_rate: 50% of trading fees (SCALAR_7). Caps keeper incentive.
pub const MAX_CALLER_RATE: i128 = 5_000_000;
/// Max base/impact fee rate: 1% of notional (SCALAR_7).
pub const MAX_FEE_RATE: i128 = 100_000;
/// Max hourly rate for r_base or r_funding: 0.01%/hr (~88% APR) in SCALAR_18.
pub const MAX_RATE_HOURLY: i128 = 100_000_000_000_000;
/// Max variable borrowing multiplier: 10x at full util (10 * SCALAR_7).
pub const MAX_R_VAR: i128 = 100_000_000;
/// Max global utilization cap: 1000% (10x vault, SCALAR_7). Notional can exceed vault.
pub const MAX_UTIL: i128 = 100_000_000;

// ── MarketConfig upper bounds ───────────────────────────────────────

/// Min impact divisor: 10 * SCALAR_7. Caps impact fee at 10% of notional.
pub const MIN_IMPACT: i128 = 100_000_000;
/// Max initial margin: 50% (SCALAR_7) = 2x minimum leverage.
pub const MAX_MARGIN: i128 = 5_000_000;
/// Max liquidation fee/threshold: 25% (SCALAR_7).
pub const MAX_LIQ_FEE: i128 = 2_500_000;
/// Max per-market borrowing weight: 10x (10 * SCALAR_7).
pub const MAX_R_BORROW: i128 = 100_000_000;
