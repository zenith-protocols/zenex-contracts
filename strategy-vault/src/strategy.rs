//! Strategy integration and share-aware deposit locking.

use soroban_sdk::{contracterror, contractevent, panic_with_error, token, Address, Env};
use stellar_tokens::{fungible::Base, vault::Vault};

use crate::storage::{self, DepositLock};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StrategyVaultError {
    InvalidAmount = 790,
    SharesLocked = 791,
    UnauthorizedStrategy = 792,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrategyWithdraw {
    #[topic]
    pub strategy: Address,
    pub amount: i128,
}

pub struct StrategyVault;

impl StrategyVault {
    /// Returns the number of shares that are currently available (unlocked)
    /// for the given address to transfer, withdraw, or redeem.
    pub fn available_shares(e: &Env, user: &Address) -> i128 {
        let balance = Base::balance(e, user);
        let Some(lock) = storage::get_deposit_lock(e, user) else {
            return balance; // no deposit history → all available
        };
        let lock_time = storage::get_lock_time(e);
        if e.ledger().timestamp() >= lock.timestamp + lock_time {
            return balance; // lock expired → all available
        }
        // Only recently deposited shares are locked
        let available = balance - lock.shares;
        if available > 0 { available } else { 0 }
    }

    /// Panics if `amount` shares exceed the user's available (unlocked) balance.
    pub fn require_available(e: &Env, user: &Address, amount: i128) {
        if amount > Self::available_shares(e, user) {
            panic_with_error!(e, StrategyVaultError::SharesLocked);
        }
    }

    /// Record newly minted shares into the deposit lock for the receiver.
    /// If the previous lock expired, resets to only the new shares.
    /// If still active, accumulates onto the existing locked shares.
    pub fn record_deposit(e: &Env, receiver: &Address, new_shares: i128) {
        let now = e.ledger().timestamp();
        let lock_time = storage::get_lock_time(e);

        let locked = match storage::get_deposit_lock(e, receiver) {
            Some(lock) if now < lock.timestamp + lock_time => lock.shares,
            _ => 0, // no lock or expired
        };

        storage::set_deposit_lock(
            e,
            receiver,
            &DepositLock {
                timestamp: now,
                shares: locked + new_shares,
            },
        );
    }

    /// Strategy withdraws tokens from the vault.
    /// This decreases total_assets and thus the share price.
    pub fn withdraw(env: &Env, strategy: &Address, amount: i128) {
        if amount <= 0 {
            panic_with_error!(env, StrategyVaultError::InvalidAmount);
        }
        if storage::get_strategy(env) != *strategy {
            panic_with_error!(env, StrategyVaultError::UnauthorizedStrategy);
        }

        let asset = Vault::query_asset(env);
        let token_client = token::Client::new(env, &asset);
        token_client.transfer(&env.current_contract_address(), strategy, &amount);

        StrategyWithdraw {
            strategy: strategy.clone(),
            amount,
        }
        .publish(env);
    }
}
