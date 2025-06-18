use soroban_sdk::{Address, Env, Symbol};

pub struct VaultEvents {}

impl VaultEvents {
    /// Emitted when tokens are deposited into the vault
    ///
    /// - topics - `["deposit"]`
    /// - data - `[receiver: Address, tokens: i128, shares: i128]`
    ///
    /// ### Arguments
    /// * receiver - The address receiving the minted shares
    /// * tokens - The amount of tokens deposited
    /// * shares - The amount of shares minted
    pub fn deposit(e: &Env, receiver: Address, tokens: i128, shares: i128) {
        let topics = (Symbol::new(e, "deposit"),);
        e.events().publish(topics, (receiver, tokens, shares));
    }

    /// Emitted when a withdrawal is queued
    ///
    /// - topics - `["queue_withdraw"]`
    /// - data - `[owner: Address, shares: i128, unlock_time: u64]`
    ///
    /// ### Arguments
    /// * owner - The address that owns the shares
    /// * shares - The amount of shares queued for withdrawal
    /// * unlock_time - The time when withdrawal can be executed
    pub fn queue_withdraw(e: &Env, owner: Address, shares: i128, unlock_time: u64) {
        let topics = (Symbol::new(e, "queue_withdraw"),);
        e.events().publish(topics, (owner, shares, unlock_time));
    }

    /// Emitted when a queued withdrawal is executed
    ///
    /// - topics - `["withdraw"]`
    /// - data - `[user: Address, shares: i128, tokens: i128]`
    ///
    /// ### Arguments
    /// * user - The address whose withdrawal is being executed
    /// * shares - The amount of shares burned
    /// * tokens - The amount of tokens withdrawn
    pub fn withdraw(e: &Env, user: Address, shares: i128, tokens: i128) {
        let topics = (Symbol::new(e, "withdraw"),);
        e.events().publish(topics, (user, shares, tokens));
    }

    /// Emitted when an emergency withdrawal is executed with penalty
    ///
    /// - topics - `["emergency_withdraw"]`
    /// - data - `[owner: Address, shares: i128, tokens: i128, penalty: i128]`
    ///
    /// ### Arguments
    /// * owner - The address whose withdrawal is being executed
    /// * shares - The amount of shares burned
    /// * tokens - The amount of tokens withdrawn (after penalty)
    /// * penalty - The penalty amount that stays in the vault
    pub fn emergency_withdraw(e: &Env, owner: Address, shares: i128, tokens: i128, penalty: i128) {
        let topics = (Symbol::new(e, "emergency_withdraw"),);
        e.events().publish(topics, (owner, shares, tokens, penalty));
    }

    /// Emitted when a withdrawal request is cancelled
    ///
    /// - topics - `["cancel_withdraw"]`
    /// - data - `[owner: Address, shares: i128]`
    ///
    /// ### Arguments
    /// * owner - The address whose withdrawal is being cancelled
    /// * shares - The amount of shares returned to the owner
    pub fn cancel_withdraw(e: &Env, owner: Address, shares: i128) {
        let topics = (Symbol::new(e, "cancel_withdraw"),);
        e.events().publish(topics, (owner, shares));
    }

    /// Emitted when a strategy transfers tokens from the vault
    ///
    /// - topics - `["transfer_to"]`
    /// - data - `[strategy: Address, amount: i128, new_net_impact: i128]`
    ///
    /// ### Arguments
    /// * strategy - The strategy contract address
    /// * amount - The amount of tokens transferred to the strategy
    /// * new_net_impact - The strategy's new net impact after transfer
    pub fn transfer_to(e: &Env, strategy: Address, amount: i128, new_net_impact: i128) {
        let topics = (Symbol::new(e, "transfer_to"),);
        e.events().publish(topics, (strategy, amount, new_net_impact));
    }

    /// Emitted when a strategy transfers tokens to the vault
    ///
    /// - topics - `["transfer_from"]`
    /// - data - `[strategy: Address, amount: i128, new_net_impact: i128]`
    ///
    /// ### Arguments
    /// * strategy - The strategy contract address
    /// * amount - The amount of tokens transferred from the strategy
    /// * new_net_impact - The strategy's new net impact after transfer
    pub fn transfer_from(e: &Env, strategy: Address, amount: i128, new_net_impact: i128) {
        let topics = (Symbol::new(e, "transfer_from"),);
        e.events().publish(topics, (strategy, amount, new_net_impact));
    }
}