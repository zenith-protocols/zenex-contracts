use soroban_sdk::{contract, contractimpl, contractclient, token, panic_with_error, Address, Env, Vec, String, BytesN, };
use soroban_fixed_point_math::SorobanFixedPoint;

use crate::{
    storage::{self, WithdrawalRequest},
    errors::VaultError,
    token::create_share_token,
    events::VaultEvents,
};

const SCALAR_7: i128 = 10_000_000;

#[contract]
pub struct VaultContract;

#[contractclient(name = "VaultClient")]
pub trait Vault {
    /// Returns the address of the underlying token managed by this vault
    ///
    /// This is the token that users deposit and that strategies operate with.
    /// The vault's share value is denominated in terms of this token.
    ///
    /// # Returns
    /// Address of the underlying token contract
    fn token(e: Env) -> Address;

    /// Returns the address of the vault's share token contract
    ///
    /// Share tokens represent proportional ownership of the vault's assets.
    /// The vault contract is the admin of this token and can mint/burn shares.
    ///
    /// # Returns
    /// Address of the share token contract
    fn share_token(e: Env) -> Address;

    /// Returns the total number of share tokens in circulation
    ///
    /// Includes both shares held by users and shares locked in withdrawal requests.
    /// Used to calculate the share-to-token exchange rate.
    ///
    /// # Returns
    /// Total share token supply (with 7 decimal places)
    fn total_shares(e: Env) -> i128;

    /// Returns the net impact of a strategy on vault assets
    ///
    /// Tracks the net flow of tokens between vault and strategy:
    /// - Negative value: strategy has transferred more from vault than to vault
    /// - Positive value: strategy has transferred more to vault than from vault (profit)
    /// - Zero: strategy is even or hasn't transferred
    ///
    /// # Arguments
    /// * `strategy` - Address of the strategy contract
    ///
    /// # Returns
    /// Net token impact (positive = profit, negative = net outflow)
    fn net_impact(e: Env, strategy: Address) -> i128;

    /// Deposits underlying tokens and mints share tokens to receiver
    ///
    /// Transfers `tokens` from the caller to the vault and mints the equivalent
    /// amount of share tokens to `receiver` based on current exchange rate.
    ///
    /// # Arguments
    /// * `tokens` - Amount of underlying tokens to deposit (must be > 0)
    /// * `receiver` - Address to receive the minted share tokens
    ///
    /// # Returns
    /// Amount of share tokens minted to receiver
    ///
    /// # Panics
    /// - `ZeroAmount` if tokens <= 0
    fn deposit(e: Env, tokens: i128, receiver: Address) -> i128;

    /// Queues a withdrawal request by locking share tokens
    ///
    /// Transfers `shares` from `owner` to the vault contract and creates a withdrawal
    /// request with unlock time = current_time + withdrawal_lock_time. The locked
    /// shares are visible as the vault's share token balance.
    ///
    /// # Arguments
    /// * `shares` - Amount of share tokens to queue for withdrawal (must be > 0)
    /// * `owner` - Address that owns the shares (must authorize transaction)
    ///
    /// # Panics
    /// - `ZeroAmount` if shares <= 0
    /// - `WithdrawalInProgress` if owner already has a pending withdrawal
    /// - `InsufficientShares` if owner doesn't have enough shares
    fn queue_withdraw(e: Env, shares: i128, owner: Address);

    /// Executes a queued withdrawal after the delay period (permissionless)
    ///
    /// Burns the locked shares and transfers the equivalent tokens (calculated at
    /// current exchange rate) to the owner. Can be called by anyone after the withdrawal
    /// unlock time has passed.
    ///
    /// # Arguments
    /// * `user` - Address that queued the withdrawal
    ///
    /// # Returns
    /// Amount of underlying tokens transferred to user
    ///
    /// # Panics
    /// - `WithdrawalLocked` if unlock time hasn't been reached
    /// - Panics if no withdrawal request exists for user
    fn withdraw(e: Env, user: Address) -> i128;

    /// Emergency withdrawal with penalty before delay period ends
    ///
    /// Burns the locked shares and transfers tokens minus penalty to the owner.
    /// Penalty decreases linearly from max_penalty_rate to 0% over the lock period.
    /// Penalty tokens remain in vault, benefiting remaining shareholders.
    ///
    /// # Arguments
    /// * `owner` - Address that queued the withdrawal (must authorize transaction)
    ///
    /// # Returns
    /// Amount of underlying tokens transferred to owner (after penalty)
    ///
    /// # Panics
    /// - `InvalidAmount` if withdrawal amount after penalty <= 0
    /// - Panics if no withdrawal request exists for owner
    fn emergency_withdraw(e: Env, owner: Address) -> i128;

    /// Cancels a pending withdrawal request
    ///
    /// Returns the locked shares to the owner and removes the withdrawal request.
    /// Can be called at any time before withdrawal is executed.
    ///
    /// # Arguments
    /// * `owner` - Address that queued the withdrawal (must authorize transaction)
    ///
    /// # Panics
    /// - Panics if no withdrawal request exists for owner
    fn cancel_withdraw(e: Env, owner: Address);

    /// Allows a registered strategy to transfer tokens from the vault
    ///
    /// Transfers `amount` tokens from vault to the calling strategy contract.
    /// Updates the strategy's net_impact by subtracting the transferred amount.
    /// Only pre-registered strategies can call this function.
    ///
    /// # Arguments
    /// * `strategy` - Address of the strategy contract (must match caller)
    /// * `amount` - Amount of tokens to transfer (must be > 0)
    ///
    /// # Authorization
    /// Requires authorization from the strategy contract
    ///
    /// # Panics
    /// - `ZeroAmount` if amount <= 0
    /// - `UnauthorizedStrategy` if strategy not registered at deployment
    /// - `InsufficientVaultBalance` if vault doesn't have enough tokens
    fn transfer_to(e: Env, strategy: Address, amount: i128);

    /// Allows a strategy to transfer tokens to the vault
    ///
    /// Transfers `amount` tokens from the calling strategy to the vault.
    /// Updates the strategy's net_impact by adding the transferred amount.
    /// Used for both returning capital and distributing profits.
    ///
    /// # Arguments
    /// * `strategy` - Address of the strategy contract (must match caller)
    /// * `amount` - Amount of tokens to transfer (must be > 0)
    ///
    /// # Panics
    /// - `ZeroAmount` if amount <= 0
    /// - `UnauthorizedStrategy` if strategy not registered at deployment
    fn transfer_from(e: Env, strategy: Address, amount: i128);
}

#[contractimpl]
impl VaultContract {
    /// Initializes the immutable vault
    ///
    /// Creates a new vault with fixed parameters that cannot be changed after deployment.
    /// Deploys a new share token contract and registers the provided strategies.
    ///
    /// # Arguments
    /// * `token` - Address of the underlying token contract
    /// * `token_wasm_hash` - WASM hash for deploying the share token contract
    /// * `name` - Name for the share token
    /// * `symbol` - Symbol for the share token
    /// * `strategies` - List of strategy contract addresses
    /// * `lock_time` - Delay in seconds before withdrawals can be executed
    /// * `penalty_rate` - Penalty rate in SCALAR_7 format
    ///
    /// # Panics
    /// - `InvalidAmount` if penalty_rate < 0 or > 100%
    pub fn __constructor(
        e: Env,
        token: Address,
        token_wasm_hash: BytesN<32>,
        name: String,
        symbol: String,
        strategies: Vec<Address>,
        lock_time: u64,
        penalty_rate: i128,
    ) {
        // Validate penalty rate (0-100% in SCALAR_7)
        if penalty_rate < 0 || penalty_rate > SCALAR_7 {
            panic_with_error!(e, VaultError::InvalidAmount);
        }
        let share_token = create_share_token(&e, token_wasm_hash, &token, &name, &symbol);

        // Store immutable vault configuration
        storage::set_token(&e, &token);
        storage::set_share_token(&e, &share_token);
        storage::set_total_shares(&e, &0);
        storage::set_lock_time(&e, &lock_time);
        storage::set_penalty_rate(&e, &penalty_rate);
        storage::set_strategies(&e, &strategies);

        // Initialize all strategies with zero impact
        for strategy_addr in strategies.iter() {
            storage::set_strategy_net_impact(&e, &strategy_addr, &0);
        }

        storage::extend_instance(&e);
    }
}

#[contractimpl]
impl Vault for VaultContract {
    fn token(e: Env) -> Address {
        storage::extend_instance(&e);
        storage::get_token(&e)
    }

    fn share_token(e: Env) -> Address {
        storage::extend_instance(&e);
        storage::get_share_token(&e)
    }

    fn total_shares(e: Env) -> i128 {
        storage::extend_instance(&e);
        storage::get_total_shares(&e)
    }

    fn net_impact(e: Env, strategy: Address) -> i128 {
        storage::extend_instance(&e);
        storage::get_strategy_net_impact(&e, &strategy)
    }

    fn deposit(e: Env, tokens: i128, receiver: Address) -> i128 {
        receiver.require_auth();
        if tokens <= 0 {
            panic_with_error!(e, VaultError::ZeroAmount);
        }

        let token = storage::get_token(&e);
        let share_token = storage::get_share_token(&e);

        let token_client = token::Client::new(&e, &token);

        let total_shares = storage::get_total_shares(&e);

        // Calculate shares to mint: shares = tokens * (total shares / total tokens)
        let shares = {
            let total_tokens = token_client.balance(&e.current_contract_address());

            if total_shares == 0 || total_tokens == 0 {
                // First deposit gets 1:1 ratio
                tokens
            } else {
                let ratio = total_shares.fixed_div_floor(&e, &total_tokens, &SCALAR_7);
                tokens.fixed_mul_floor(&e, &ratio, &SCALAR_7)
            }
        };

        // Transfer tokens from caller to vault (receiver authorizes, not vault)
        token_client.transfer(&receiver, &e.current_contract_address(), &tokens);

        // Mint shares to receiver
        token::StellarAssetClient::new(&e, &share_token).mint(&receiver, &shares);

        // Update total shares
        storage::set_total_shares(&e, &(total_shares + shares));

        // Emit deposit event
        VaultEvents::deposit(&e, receiver.clone(), tokens, shares);

        storage::extend_instance(&e);
        shares
    }

    fn queue_withdraw(e: Env, shares: i128, owner: Address) {
        owner.require_auth();

        if shares <= 0 {
            panic_with_error!(e, VaultError::ZeroAmount);
        }

        // Check if user already has pending withdrawal
        if storage::has_withdrawal_request(&e, &owner) {
            panic_with_error!(e, VaultError::WithdrawalInProgress);
        }

        // Verify user has enough shares
        let share_token = storage::get_share_token(&e);
        let share_client = token::Client::new(&e, &share_token);
        share_client.transfer(&owner, &e.current_contract_address(), &shares);

        // Create withdrawal request
        let lock_time = storage::get_lock_time(&e);
        let unlock_time = e.ledger().timestamp() + lock_time;
        let request = WithdrawalRequest {
            shares,
            unlock_time,
        };

        storage::set_withdrawal_request(&e, &owner, &request);

        // Emit queue withdraw event
        VaultEvents::queue_withdraw(&e, owner.clone(), shares, unlock_time);

        storage::extend_instance(&e);
    }

    fn withdraw(e: Env, user: Address) -> i128 {
        let request = storage::get_withdrawal_request(&e, &user);
        if e.ledger().timestamp() < request.unlock_time {
            panic_with_error!(e, VaultError::WithdrawalLocked);
        }

        let token = storage::get_token(&e);
        let token_client = token::Client::new(&e, &token);
        let share_token = storage::get_share_token(&e);
        let share_client = token::Client::new(&e, &share_token);
        let total_shares = storage::get_total_shares(&e);

        // tokens = shares * (total tokens / total shares)
        let total_tokens = token_client.balance(&e.current_contract_address());
        let ratio = total_tokens.fixed_div_floor(&e, &total_shares, &SCALAR_7);
        let tokens = request.shares.fixed_mul_floor(&e, &ratio, &SCALAR_7);

        share_client.burn(&e.current_contract_address(), &request.shares);
        token_client.transfer(&e.current_contract_address(), &user, &tokens);
        storage::set_total_shares(&e, &(total_shares - request.shares));
        storage::remove_withdrawal_request(&e, &user);

        // Emit withdraw event
        VaultEvents::withdraw(&e, user.clone(), request.shares, tokens);

        storage::extend_instance(&e);
        tokens
    }

    fn emergency_withdraw(e: Env, owner: Address) -> i128 {
        owner.require_auth();
        let request = storage::get_withdrawal_request(&e, &owner);

        let token = storage::get_token(&e);
        let token_client = token::Client::new(&e, &token);
        let share_token = storage::get_share_token(&e);
        let share_client = token::Client::new(&e, &share_token);
        let total_shares = storage::get_total_shares(&e);

        // tokens = shares * (total tokens / total shares)
        let total_tokens = token_client.balance(&e.current_contract_address());
        let ratio = total_tokens.fixed_div_floor(&e, &total_shares, &SCALAR_7);
        let current_tokens = request.shares.fixed_mul_floor(&e, &ratio, &SCALAR_7);

        // Calculate penalty - inlined logic
        let current_time = e.ledger().timestamp();
        let penalty_amount = if current_time >= request.unlock_time {
            0
        } else {
            let lock_time = storage::get_lock_time(&e);
            let time_remaining = request.unlock_time - current_time;
            let penalty_rate = storage::get_penalty_rate(&e);

            // Linear penalty: current_penalty_rate = max_penalty * (time_remaining / total_lock_time)
            let current_penalty_rate = penalty_rate.fixed_mul_floor(&e, &(time_remaining as i128), &(lock_time as i128));

            // Apply penalty to current token value
            current_tokens.fixed_mul_floor(&e, &current_penalty_rate, &SCALAR_7)
        };

        let withdrawal_amount = current_tokens - penalty_amount;

        if withdrawal_amount <= 0 {
            panic_with_error!(e, VaultError::InvalidAmount);
        }

        // Execute withdrawal - inlined logic (penalty stays in vault)
        share_client.burn(&e.current_contract_address(), &request.shares);
        token_client.transfer(&e.current_contract_address(), &owner, &withdrawal_amount);
        storage::set_total_shares(&e, &(total_shares - request.shares));
        storage::remove_withdrawal_request(&e, &owner);

        // Emit emergency withdraw event
        VaultEvents::emergency_withdraw(&e, owner.clone(), request.shares, withdrawal_amount, penalty_amount);

        storage::extend_instance(&e);
        withdrawal_amount
    }

    fn cancel_withdraw(e: Env, owner: Address) {
        owner.require_auth();
        let request = storage::get_withdrawal_request(&e, &owner);

        let share_token = storage::get_share_token(&e);
        token::Client::new(&e, &share_token).transfer(&e.current_contract_address(), &owner, &request.shares);

        storage::remove_withdrawal_request(&e, &owner);

        // Emit cancel withdraw event
        VaultEvents::cancel_withdraw(&e, owner.clone(), request.shares);

        storage::extend_instance(&e);
    }

    fn transfer_to(e: Env, strategy: Address, amount: i128) {
        strategy.require_auth();
        if amount <= 0 {
            panic_with_error!(e, VaultError::ZeroAmount);
        }

        // Check if strategy is authorized
        let strategies = storage::get_strategies(&e);
        if !strategies.contains(&strategy) {
            panic_with_error!(e, VaultError::UnauthorizedStrategy);
        }

        // Transfer tokens to strategy
        let token = storage::get_token(&e);
        token::Client::new(&e, &token).transfer(&e.current_contract_address(), &strategy, &amount);

        // Update strategy net impact (negative = net outflow to strategy)
        let current_impact = storage::get_strategy_net_impact(&e, &strategy);
        let new_impact = current_impact - amount;
        storage::set_strategy_net_impact(&e, &strategy, &new_impact);

        // Emit transfer to event
        VaultEvents::transfer_to(&e, strategy.clone(), amount, new_impact);

        storage::extend_instance(&e);
    }

    fn transfer_from(e: Env, strategy: Address, amount: i128) {
        strategy.require_auth();
        if amount <= 0 {
            panic_with_error!(e, VaultError::ZeroAmount);
        }

        // Check if strategy is authorized
        let strategies = storage::get_strategies(&e);
        if !strategies.contains(&strategy) {
            panic_with_error!(e, VaultError::UnauthorizedStrategy);
        }

        // Transfer tokens from strategy to vault
        let token = storage::get_token(&e);
        token::Client::new(&e, &token).transfer(&strategy, &e.current_contract_address(), &amount);

        // Update strategy net impact (positive = net inflow from strategy)
        let current_impact = storage::get_strategy_net_impact(&e, &strategy);
        let new_impact = current_impact + amount;
        storage::set_strategy_net_impact(&e, &strategy, &new_impact);

        // Emit transfer from event
        VaultEvents::transfer_from(&e, strategy.clone(), amount, new_impact);
        storage::extend_instance(&e);
    }
}