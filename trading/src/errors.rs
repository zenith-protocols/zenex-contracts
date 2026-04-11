use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TradingError {
    Unauthorized = 1, // caller is not the contract owner

    // 700: Config & Market
    InvalidConfig = 700, // config parameter out of valid range
    MarketNotFound = 701, // no market registered for the given market_id
    MarketDisabled = 702, // market is disabled or deleted
    MaxMarketsReached = 703, // MAX_ENTRIES (50) markets already registered

    // 710: Price
    InvalidPrice = 710, // price verification failed, feed_id mismatch, or missing feed
    StalePrice = 711, // price data predates position open time

    // 720: Position
    PositionNotFound = 720, // position ID not found in storage
    PositionNotPending = 721, // position is filled; expected pending
    NegativeValueNotAllowed = 723, // a parameter is <= 0 or negative
    NotionalBelowMinimum = 724, // notional below TradingConfig.min_notional
    NotionalAboveMaximum = 725, // notional above TradingConfig.max_notional
    LeverageAboveMaximum = 726, // effective leverage exceeds 1/margin
    CollateralUnchanged = 727, // modify_collateral called with unchanged amount
    WithdrawalBreaksMargin = 728, // collateral withdrawal would breach margin requirement
    NotActionable = 731, // no valid action for this position
    PositionTooNew = 732, // close attempted before MIN_OPEN_TIME (30s)
    ActionNotAllowedForStatus = 733, // action not allowed for position status
    InvalidInput = 734, // malformed input (e.g. mismatched parallel vec lengths)

    // 740: Contract Status
    InvalidStatus = 740, // invalid or disallowed contract status value
    ContractOnIce = 741, // new positions blocked (OnIce, AdminOnIce, or Frozen)
    ContractFrozen = 742, // all position management blocked (Frozen)

    // 750: Utilization & Funding
    ThresholdNotMet = 750, // net PnL below ADL threshold
    UtilizationExceeded = 751, // position would exceed notional/vault cap
    FundingTooEarly = 752, // apply_funding called < 1 hour since last call

    // 760-769: reserved for trading growth
}
