use soroban_sdk::{contract, contractimpl, Address, Env, MuxedAddress, String};
use stellar_tokens::{
    fungible::{Base, FungibleToken},
    vault::{FungibleVault, Vault},
};

use crate::{storage, strategy::StrategyVault};

/// ERC-4626 tokenized vault with share-aware deposit locking. Backs trader
/// positions with depositor collateral. Only recently deposited shares are
/// locked; previously deposited shares remain freely available.
#[contract]
pub struct StrategyVaultContract;

#[contractimpl]
impl StrategyVaultContract {
    pub fn __constructor(
        e: Env,
        name: String,
        symbol: String,
        asset: Address,
        decimals_offset: u32,
        strategy: Address,
        lock_time: u64,
    ) {
        Vault::set_asset(&e, asset);
        Vault::set_decimals_offset(&e, decimals_offset);
        Base::set_metadata(&e, Vault::decimals(&e), name, symbol);

        storage::set_lock_time(&e, &lock_time);
        storage::set_strategy(&e, &strategy);
    }

    /// Returns the lock time in seconds.
    pub fn lock_time(e: Env) -> u64 {
        storage::extend_instance(&e);
        storage::get_lock_time(&e)
    }

    /// Returns seconds remaining until user's lock expires, or 0 if unlocked.
    pub fn lock_duration(e: Env, user: Address) -> u64 {
        storage::extend_instance(&e);
        StrategyVault::lock_time_remaining(&e, &user)
    }

    /// Returns the number of shares the user can currently withdraw/transfer.
    pub fn available_shares(e: Env, user: Address) -> i128 {
        storage::extend_instance(&e);
        StrategyVault::available_shares(&e, &user)
    }

    /// Strategy (trading contract) withdraws tokens from the vault to pay
    /// winning positions. Decreases `total_assets` and thus share price.
    pub fn strategy_withdraw(e: Env, strategy: Address, amount: i128) {
        strategy.require_auth();
        StrategyVault::withdraw(&e, &strategy, amount);
        storage::extend_instance(&e);
    }
}

// Override transfer/transfer_from to enforce share-aware lock.
#[contractimpl(contracttrait)]
impl FungibleToken for StrategyVaultContract {
    type ContractType = Vault;

    fn transfer(e: &Env, from: Address, to: MuxedAddress, amount: i128) {
        StrategyVault::require_available(e, &from, amount);
        Base::transfer(e, &from, &to, amount);
    }

    fn transfer_from(e: &Env, spender: Address, from: Address, to: Address, amount: i128) {
        StrategyVault::require_available(e, &from, amount);
        Base::transfer_from(e, &spender, &from, &to, amount);
    }
}

// Override deposit/mint to record locked shares.
// Override withdraw/redeem to enforce share-aware lock.
#[contractimpl(contracttrait)]
impl FungibleVault for StrategyVaultContract {
    fn deposit(e: &Env, assets: i128, receiver: Address, from: Address, operator: Address) -> i128 {
        let shares = Vault::deposit(e, assets, receiver.clone(), from, operator);
        StrategyVault::record_deposit(e, &receiver, shares);
        storage::extend_instance(e);
        shares
    }

    fn mint(e: &Env, shares: i128, receiver: Address, from: Address, operator: Address) -> i128 {
        let assets = Vault::mint(e, shares, receiver.clone(), from, operator);
        StrategyVault::record_deposit(e, &receiver, shares);
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
        let shares_needed = Vault::preview_withdraw(e, assets);
        StrategyVault::require_available(e, &owner, shares_needed);
        let shares = Vault::withdraw(e, assets, receiver, owner, operator);
        storage::extend_instance(e);
        shares
    }

    fn redeem(e: &Env, shares: i128, receiver: Address, owner: Address, operator: Address) -> i128 {
        StrategyVault::require_available(e, &owner, shares);
        let assets = Vault::redeem(e, shares, receiver, owner, operator);
        storage::extend_instance(e);
        assets
    }
}
