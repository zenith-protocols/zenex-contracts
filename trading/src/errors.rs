use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TradingError {
    // Access
    Unauthorized = 1,

    // Configuration
    AlreadyInitialized = 300,
    NotInitialized = 301,
    InvalidConfig = 302,
    UpdateNotQueued = 303,
    UpdateNotUnlocked = 304,

    // Market
    MarketNotFound = 310,
    MarketInitNotQueued = 311,
    MarketDisabled = 312,

    // Oracle/Price
    PriceNotFound = 320,
    PriceStale = 321,

    // Position
    PositionNotFound = 325,
    PositionAlreadyClosed = 326,
    PositionNotOpen = 327,
    PositionNotPending = 328,
    MaxPositionsReached = 329,
    NegativeValueNotAllowed = 330,
    CollateralBelowMinimum = 331,
    CollateralAboveMaximum = 332,
    LeverageBelowMinimum = 333,
    InvalidEntryPrice = 334,
    WithdrawalBreaksMargin = 337,
    InvalidTakeProfitPrice = 340,
    InvalidStopLossPrice = 341,
    TakeProfitNotTriggered = 342,
    StopLossNotTriggered = 343,
    PositionNotLiquidatable = 345,
    LimitOrderNotFillable = 346,

    // Action/Request
    ActionNotAllowedForStatus = 351,
    InvalidRequestType = 352,

    // Status
    InvalidStatus = 381,
    ContractPaused = 380,

    // Utilization
    UtilizationLimitExceeded = 390,
}
