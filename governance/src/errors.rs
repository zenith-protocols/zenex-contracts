use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum TimelockError {
    /// Queue entry not found or expired
    NotQueued = 1,
    /// Timelock delay not yet passed
    NotUnlocked = 2,
    /// Caller is not owner
    Unauthorized = 3,
    /// Delay value is zero or unreasonable
    InvalidDelay = 4,
}
