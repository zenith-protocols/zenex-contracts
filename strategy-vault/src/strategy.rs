//! Strategy integration and custom vault extensions

use soroban_sdk::{contracterror, contractevent, panic_with_error, token, Address, Env};
use stellar_tokens::vault::Vault;

use crate::storage;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StrategyVaultError {
    InvalidAmount = 420,
    SharesLocked = 421,
    UnauthorizedStrategy = 422,
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
    /// Returns seconds remaining until user's shares unlock, or 0 if unlocked.
    /// Users without deposit history (received shares via transfer) are never locked.
    pub fn get_lock_time(e: &Env, user: &Address) -> u64 {
        let Some(last_deposit_time) = storage::get_last_deposit_time(e, user) else {
            return 0;
        };
        let unlock_time = last_deposit_time.saturating_add(storage::get_lock_time(e));
        unlock_time.saturating_sub(e.ledger().timestamp())
    }

    /// Panics if user's shares are currently locked
    pub fn require_unlocked(e: &Env, user: &Address) {
        if Self::get_lock_time(e, user) > 0 {
            panic_with_error!(e, StrategyVaultError::SharesLocked);
        }
    }

    /// Strategy withdraws tokens from the vault
    /// This decreases total_assets and thus the share price
    pub fn withdraw(env: &Env, strategy: &Address, amount: i128) {
        if amount <= 0 {
            panic_with_error!(env, StrategyVaultError::InvalidAmount);
        }
        if !storage::get_strategies(env).contains(strategy) {
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
