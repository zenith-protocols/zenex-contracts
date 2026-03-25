use soroban_sdk::contracterror;

/// All errors returned by the trading contract.
///
/// Error codes are grouped by domain (1xx access, 7xx domain logic, 78x ADL, 79x utilization).
/// On-chain, these appear as `Error(Contract, #code)`.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TradingError {
    // ── Access ──
    /// Caller is not the contract owner.
    Unauthorized = 1,

    // ── Configuration ──
    /// Config parameter out of valid range.
    InvalidConfig = 702,

    // ── Market ──
    /// No market registered for the given feed_id.
    MarketNotFound = 710,
    /// Market exists but is disabled (config.enabled == false).
    MarketDisabled = 712,

    // ── Price ──
    /// Price verification failed, feed_id mismatch, or missing feed in batch.
    InvalidPrice = 720,

    // ── Position ──
    /// Position ID not found in storage.
    PositionNotFound = 730,
    /// Position is filled; expected pending (e.g. cancel or re-fill).
    PositionNotPending = 733,
    /// User has MAX_ENTRIES (50) positions.
    MaxPositionsReached = 734,
    /// A parameter (collateral, notional, price) is <= 0 or negative.
    NegativeValueNotAllowed = 735,
    /// Notional below TradingConfig.min_notional.
    NotionalBelowMinimum = 736,
    /// Notional above TradingConfig.max_notional.
    NotionalAboveMaximum = 737,
    /// Effective leverage exceeds 1/margin.
    LeverageAboveMaximum = 739,
    /// modify_collateral called with unchanged amount.
    CollateralUnchanged = 740,
    /// Collateral withdrawal would breach margin requirement.
    WithdrawalBreaksMargin = 741,
    /// TP price on wrong side of entry (e.g. TP < entry for long).
    InvalidTakeProfitPrice = 742,
    /// SL price on wrong side of entry (e.g. SL > entry for long).
    InvalidStopLossPrice = 743,
    /// Keeper tried TP trigger but price hasn't reached TP.
    TakeProfitNotTriggered = 744,
    /// Keeper tried SL trigger but price hasn't reached SL.
    StopLossNotTriggered = 745,
    /// Position equity above liquidation threshold.
    PositionNotLiquidatable = 746,
    /// Limit order price not yet reached by market.
    LimitOrderNotFillable = 747,
    /// User close attempted before MIN_OPEN_TIME (30s).
    PositionTooNew = 748,
    /// Price data predates the position open time (stale-price liquidation guard).
    StalePrice = 749,

    // ── Action / Request ──
    /// Action not allowed for position status (e.g. close on pending).
    ActionNotAllowedForStatus = 750,
    /// Unknown ExecuteRequest type (not 0-3).
    InvalidRequestType = 751,

    // ── Status ──
    /// Invalid or disallowed contract status value.
    InvalidStatus = 760,
    /// New positions blocked: contract is OnIce, AdminOnIce, or Frozen.
    ContractOnIce = 761,
    /// All position management blocked: contract is Frozen.
    ContractFrozen = 762,

    // ── Market limits ──
    /// MAX_ENTRIES (50) markets already registered.
    MaxMarketsReached = 770,
    /// Cannot delete market with nonzero open interest.
    MarketHasOpenPositions = 771,

    // ── ADL / Circuit breaker ──
    /// Net PnL below ADL threshold; update_status is a no-op.
    ThresholdNotMet = 780,

    // ── Funding ──
    /// apply_funding called less than 1 hour since last call.
    FundingTooEarly = 790,

    // ── Utilization ──
    /// Position would exceed per-market or global notional/vault cap.
    UtilizationExceeded = 791,
}
