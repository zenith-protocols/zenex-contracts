use soroban_sdk::{contract, contractimpl, Address, Env, MuxedAddress, String};
use stellar_tokens::{
    fungible::{Base, FungibleToken},
    vault::{FungibleVault, Vault},
};

use crate::{storage, strategy::StrategyVault};

/// ERC-4626 tokenized vault with deposit locking. Backs trader positions with
/// depositor collateral. Depositors withdraw via ERC-4626 after the lock period.
/// The strategy (trading contract) withdraws via `strategy_withdraw`.
#[contract]
pub struct StrategyVaultContract;

#[contractimpl]
impl StrategyVaultContract {
    /// Initialize the strategy vault with share token metadata and locking parameters.
    ///
    /// # Parameters
    /// - `name` - Human-readable name for the vault share token (e.g. "Zenex BTC Vault")
    /// - `symbol` - Symbol for the vault share token (e.g. "zBTC")
    /// - `asset` - Address of the underlying collateral token (e.g. USDC)
    /// - `decimals_offset` - Virtual decimal offset for vault inflation attack protection (0-10).
    /// - `strategy` - Authorized strategy contract (trading contract). Only this address
    ///   can call `strategy_withdraw`.
    /// - `lock_time` - Seconds depositors must wait after their last deposit before
    ///   transferring, withdrawing, or redeeming shares.
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

        // Initialize custom storage
        storage::set_lock_time(&e, &lock_time);
        storage::set_strategy(&e, &strategy);
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

    /// Strategy (trading contract) withdraws tokens from the vault to pay winning positions.
    /// Decreases `total_assets` and thus the share price for all depositors.
    ///
    /// # Parameters
    /// - `strategy` - Must match the stored strategy address and provide auth
    /// - `amount` - Token amount to withdraw (token_decimals)
    pub fn strategy_withdraw(e: Env, strategy: Address, amount: i128) {
        strategy.require_auth();
        StrategyVault::withdraw(&e, &strategy, amount);
        storage::extend_instance(&e);
    }
}

// Implement FungibleToken trait for share token functionality.
// transfer and transfer_from are overridden to enforce the deposit lock.
// Without this, a depositor could transfer shares to a fresh address to bypass
// the lock_time check on withdraw/redeem.
#[contractimpl(contracttrait)]
impl FungibleToken for StrategyVaultContract {
    type ContractType = Vault;

    /// Override: Depositors cannot transfer shares until lock_time expires after last deposit.
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

// Implement FungibleVault trait for ERC-4626 functionality.
// Overrides: deposit/mint record the deposit timestamp for lock tracking.
// redeem/withdraw enforce the lock before allowing share redemption.
#[contractimpl(contracttrait)]
impl FungibleVault for StrategyVaultContract {
    /// Override: Records deposit timestamp for lock tracking.
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

    /// Override: Depositors cannot withdraw until lock expires
    fn withdraw(
        e: &Env,
        assets: i128,
        receiver: Address,
        owner: Address,
        operator: Address,
    ) -> i128 {
        StrategyVault::require_unlocked(e, &owner);
        let shares = Vault::withdraw(e, assets, receiver, owner, operator);
        storage::extend_instance(e);
        shares
    }

    /// Override: Depositors cannot redeem until lock expires
    fn redeem(e: &Env, shares: i128, receiver: Address, owner: Address, operator: Address) -> i128 {
        StrategyVault::require_unlocked(e, &owner);
        let assets = Vault::redeem(e, shares, receiver, owner, operator);
        storage::extend_instance(e);
        assets
    }

}
