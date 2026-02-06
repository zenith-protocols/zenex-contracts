#![no_std]

mod contract;
pub use contract::{WebauthnVerifierContract, WebauthnVerifierContractClient};

#[cfg(test)]
mod test;
