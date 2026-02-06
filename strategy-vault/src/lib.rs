#![no_std]

mod contract;
mod storage;
mod strategy;
pub use contract::{StrategyVaultContract, StrategyVaultContractClient};

#[cfg(test)]
mod test;
