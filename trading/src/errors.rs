use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TradingError {
    Unauthorized = 1, // caller is not the contract owner

    InvalidConfig = 702, // config parameter out of valid range

    MarketNotFound = 710, // no market registered for the given feed_id
    MarketNotActive = 712, // market is not Active (Halted or Delisting)

    InvalidPrice = 720, // price verification failed, feed_id mismatch, or missing feed

    PositionNotFound = 730, // position ID not found in storage
    PositionNotPending = 733, // position is filled; expected pending
    MaxPositionsReached = 734, // user has MAX_ENTRIES (50) positions
    NegativeValueNotAllowed = 735, // a parameter is <= 0 or negative
    NotionalBelowMinimum = 736, // notional below TradingConfig.min_notional
    NotionalAboveMaximum = 737, // notional above TradingConfig.max_notional
    LeverageAboveMaximum = 739, // effective leverage exceeds 1/margin
    CollateralUnchanged = 740, // modify_collateral called with unchanged amount
    WithdrawalBreaksMargin = 741, // collateral withdrawal would breach margin requirement
    InvalidTakeProfitPrice = 742, // TP price on wrong side of entry
    InvalidStopLossPrice = 743, // SL price on wrong side of entry
    NotActionable = 747, // no valid action for this position
    PositionTooNew = 748, // close attempted before MIN_OPEN_TIME (30s)
    StalePrice = 749, // price data predates position open time

    ActionNotAllowedForStatus = 750, // action not allowed for position status

    InvalidStatus = 760, // invalid or disallowed contract status value
    ContractOnIce = 761, // new positions blocked (OnIce, AdminOnIce, or Frozen)
    ContractFrozen = 762, // all position management blocked (Frozen)

    MaxMarketsReached = 770, // MAX_ENTRIES (50) markets already registered
    MarketHasOpenPositions = 771, // cannot delete market with nonzero open interest

    ThresholdNotMet = 780, // net PnL below ADL threshold

    FundingTooEarly = 790, // apply_funding called < 1 hour since last call

    UtilizationExceeded = 791, // position would exceed notional/vault cap
}
