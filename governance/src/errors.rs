use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GovernanceError {
    Unauthorized = 1,    // caller is not the contract owner
    NotQueued = 770,     // queue entry not found or expired
    NotUnlocked = 771,   // timelock delay not yet passed
    InvalidDelay = 772,  // delay value is zero or unreasonable
}
