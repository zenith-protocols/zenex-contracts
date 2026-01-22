#![allow(clippy::too_many_arguments)]

use soroban_sdk::{contractclient, Address, Env};

/// Vault client interface - manually defined to avoid duplicate type conflicts
/// from OpenZeppelin's stellar-tokens library
#[allow(dead_code)] // Trait is used by the generated Client via #[contractclient]
#[contractclient(name = "Client")]
pub trait VaultInterface {
    /// Returns the address of the underlying asset that the vault manages
    fn query_asset(e: Env) -> Address;

    /// Returns the total amount of underlying assets held by the vault
    fn total_assets(e: Env) -> i128;

    /// Strategy withdraws tokens from the vault (decreases total_assets and share price)
    fn strategy_withdraw(e: Env, strategy: Address, amount: i128);
}
