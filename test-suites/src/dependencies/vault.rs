use soroban_sdk::{contractclient, Address, Env};

/// Vault WASM bytes
pub const VAULT_WASM: &[u8] = include_bytes!("../../../wasm/strategy_vault.wasm");

/// Vault client interface - manually defined to avoid duplicate type conflicts
/// from OpenZeppelin's stellar-tokens library
#[contractclient(name = "VaultClient")]
pub trait VaultInterface {
    /// Returns the address of the underlying asset that the vault manages
    fn query_asset(e: Env) -> Address;

    /// Strategy withdraws tokens from the vault (decreases total_assets and share price)
    fn strategy_withdraw(e: Env, strategy: Address, amount: i128);

    /// Strategy deposits tokens to the vault (increases total_assets and share price)
    fn strategy_deposit(e: Env, strategy: Address, amount: i128);

    /// ERC-4626 deposit: deposits assets and mints shares to receiver
    fn deposit(e: Env, assets: i128, receiver: Address, from: Address, operator: Address) -> i128;

    /// Returns the total amount of underlying assets held by the vault
    fn total_assets(e: Env) -> i128;

    /// Returns the balance of shares for a given address
    fn balance(e: Env, account: Address) -> i128;
}