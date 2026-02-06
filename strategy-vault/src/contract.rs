//! Vault Contract - ERC-4626 compliant tokenized vault with transfer locking
//!
//! This contract implements the OpenZeppelin FungibleVault trait with a transfer
//! lock mechanism: depositors cannot transfer their shares until lock_time seconds
//! after their last deposit. Withdrawals and redemptions are always allowed.

use soroban_sdk::{contract, contractimpl, Address, Env, MuxedAddress, String, Vec};
use stellar_tokens::{
    fungible::{Base, FungibleToken},
    vault::{FungibleVault, Vault},
};

use crate::{storage, strategy::StrategyVault};

#[contract]
pub struct StrategyVaultContract;

#[contractimpl]
impl StrategyVaultContract {
    /// Initializes the vault
    ///
    /// # Arguments
    /// * `name` - Name for the vault share token
    /// * `symbol` - Symbol for the vault share token
    /// * `asset` - Address of the underlying token contract
    /// * `decimals_offset` - Virtual offset for inflation attack protection (0-10)
    /// * `strategies` - List of authorized strategy contract addresses
    /// * `lock_time` - Delay in seconds before depositors can transfer their shares
    pub fn __constructor(
        e: Env,
        name: String,
        symbol: String,
        asset: Address,
        decimals_offset: u32,
        strategies: Vec<Address>,
        lock_time: u64,
    ) {
        Vault::set_asset(&e, asset);
        Vault::set_decimals_offset(&e, decimals_offset);
        Base::set_metadata(&e, Vault::decimals(&e), name, symbol);

        // Initialize custom storage
        storage::set_lock_time(&e, &lock_time);
        storage::set_strategies(&e, &strategies);
    }

    /// Returns the lock time in seconds
    pub fn lock_time(e: Env) -> u64 {
        storage::extend_instance(&e);
        storage::get_lock_time(&e)
    }

    /// Returns seconds remaining until user's shares unlock, or 0 if not locked
    pub fn lock_duration(e: Env, user: Address) -> u64 {
        storage::extend_instance(&e);
        StrategyVault::get_lock_time(&e, &user)
    }

    /// Strategy withdraws tokens from the vault (decreases total_assets and share price)
    pub fn strategy_withdraw(e: Env, strategy: Address, amount: i128) {
        strategy.require_auth();
        StrategyVault::withdraw(&e, &strategy, amount);
        storage::extend_instance(&e);
    }
}

// Implement FungibleToken trait for share token functionality
#[contractimpl(contracttrait)]
impl FungibleToken for StrategyVaultContract {
    type ContractType = Vault;

    /// Override: Depositors cannot transfer until lock expires
    fn transfer(e: &Env, from: Address, to: MuxedAddress, amount: i128) {
        StrategyVault::require_unlocked(e, &from);
        Base::transfer(e, &from, &to, amount);
    }

    /// Override: Depositors cannot transfer until lock expires
    fn transfer_from(e: &Env, spender: Address, from: Address, to: Address, amount: i128) {
        StrategyVault::require_unlocked(e, &from);
        Base::transfer_from(e, &spender, &from, &to, amount);
    }
}

// Implement FungibleVault trait for ERC-4626 functionality
// Override deposit/mint to track timestamps, and redeem/withdraw to check lock
#[contractimpl(contracttrait)]
impl FungibleVault for StrategyVaultContract {
    /// Override: Track deposit timestamp for the receiver (who gets the shares)
    fn deposit(e: &Env, assets: i128, receiver: Address, from: Address, operator: Address) -> i128 {
        let shares = Vault::deposit(e, assets, receiver.clone(), from, operator);
        storage::set_last_deposit_time(e, &receiver, e.ledger().timestamp());
        storage::extend_instance(e);
        shares
    }

    /// Override: Track mint timestamp for the receiver (who gets the shares)
    fn mint(e: &Env, shares: i128, receiver: Address, from: Address, operator: Address) -> i128 {
        let assets = Vault::mint(e, shares, receiver.clone(), from, operator);
        storage::set_last_deposit_time(e, &receiver, e.ledger().timestamp());
        storage::extend_instance(e);
        assets
    }

    fn redeem(e: &Env, shares: i128, receiver: Address, owner: Address, operator: Address) -> i128 {
        let assets = Vault::redeem(e, shares, receiver, owner, operator);
        storage::extend_instance(e);
        assets
    }

    fn withdraw(
        e: &Env,
        assets: i128,
        receiver: Address,
        owner: Address,
        operator: Address,
    ) -> i128 {
        let shares = Vault::withdraw(e, assets, receiver, owner, operator);
        storage::extend_instance(e);
        shares
    }

}
