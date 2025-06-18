use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultError {
    // Amount validation errors
    ZeroAmount = 4041,
    InsufficientShares = 4042,
    InvalidAmount = 4043,

    // Vault capacity errors
    InsufficientVaultBalance = 4045,

    // Withdrawal errors
    WithdrawalInProgress = 4046,
    WithdrawalLocked = 4047,

    // Strategy errors
    UnauthorizedStrategy = 4048,
}