#![no_std]

mod contract;
pub use contract::{Ed25519VerifierContract, Ed25519VerifierContractClient};

#[cfg(test)]
mod test;
