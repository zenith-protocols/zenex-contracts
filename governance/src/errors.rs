use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GovernanceError {
    Unauthorized = 1,    // caller is not the contract owner
    NotQueued = 601,     // queue entry not found or expired
    NotUnlocked = 602,   // timelock delay not yet passed
    InvalidDelay = 603,  // delay value is zero or unreasonable
}
