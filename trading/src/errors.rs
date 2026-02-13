use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TradingError {
    // Access
    Unauthorized = 1,

    // Configuration
    AlreadyInitialized = 700,
    NotInitialized = 701,
    InvalidConfig = 702,
    UpdateNotQueued = 703,
    UpdateNotUnlocked = 704,

    // Market
    MarketNotFound = 710,
    MarketInitNotQueued = 711,
    MarketDisabled = 712,

    // Oracle/Price
    PriceNotFound = 720,
    PriceStale = 721,

    // Position
    PositionNotFound = 730,
    PositionAlreadyClosed = 731,
    PositionNotOpen = 732,
    PositionNotPending = 733,
    MaxPositionsReached = 734,
    NegativeValueNotAllowed = 735,
    CollateralBelowMinimum = 736,
    CollateralAboveMaximum = 737,
    LeverageBelowMinimum = 738,
    InvalidEntryPrice = 739,
    CollateralUnchanged = 740,
    WithdrawalBreaksMargin = 741,
    InvalidTakeProfitPrice = 742,
    InvalidStopLossPrice = 743,
    TakeProfitNotTriggered = 744,
    StopLossNotTriggered = 745,
    PositionNotLiquidatable = 746,
    LimitOrderNotFillable = 747,
    PositionTooNew = 748,

    // Action/Request
    ActionNotAllowedForStatus = 750,
    InvalidRequestType = 751,

    // Status
    InvalidStatus = 760,
    ContractOnIce = 761,
    ContractFrozen = 762,

    // Utilization
    UtilizationLimitExceeded = 770,
}
