#![no_std]

mod contract;
#[cfg(test)]
mod test;

pub use contract::*;

// Re-export types from stellar-accounts for convenience
pub use stellar_accounts::smart_account::{ContextRule, ContextRuleType, Signer};
