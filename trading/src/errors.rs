use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TradingError {
    //TODO: Figure out errors
    //Related to oracle
    StalePrice = 10,
    NoPrice = 11,

    //Setting up markets
    NotUnlocked = 66,
    InvalidConfig = 67,

    MaxPositions = 68,

    InvalidAction = 69,

    BadRequest = 20, // General error if the call is invalid
}
