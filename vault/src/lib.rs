#![no_std]

mod errors;
mod storage;
mod contract;
pub use contract::{VaultContract, VaultContractClient, VaultClient};
mod token;
mod events;