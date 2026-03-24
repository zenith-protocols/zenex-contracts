use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TradingError {
    // Access
    Unauthorized = 1,

    // Configuration
    InvalidConfig = 702,
    // Market
    MarketNotFound = 710,
    MarketDisabled = 712,

    // Price
    InvalidPrice = 720,

    // Position
    PositionNotFound = 730,
    PositionNotPending = 733,
    MaxPositionsReached = 734,
    NegativeValueNotAllowed = 735,
    NotionalBelowMinimum = 736,
    NotionalAboveMaximum = 737,
    LeverageAboveMaximum = 739,
    CollateralUnchanged = 740,
    WithdrawalBreaksMargin = 741,
    InvalidTakeProfitPrice = 742,
    InvalidStopLossPrice = 743,
    TakeProfitNotTriggered = 744,
    StopLossNotTriggered = 745,
    PositionNotLiquidatable = 746,
    LimitOrderNotFillable = 747,
    PositionTooNew = 748,
    StalePrice = 749,

    // Action/Request
    ActionNotAllowedForStatus = 750,
    InvalidRequestType = 751,

    // Status
    InvalidStatus = 760,
    ContractOnIce = 761,
    ContractFrozen = 762,

    // Market limits
    MaxMarketsReached = 770,
    MarketHasOpenPositions = 771,

    // Funding
    FundingTooEarly = 790,

    // Utilization
    UtilizationExceeded = 791,


    // ADL / Circuit breaker
    ThresholdNotMet = 780,
}
