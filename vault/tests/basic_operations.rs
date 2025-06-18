//! Basic vault operations integration tests
//!
//! Tests core functionality: deposits, withdrawals, share calculations,
//! single strategy operations, and error conditions.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Env, Address, Vec, String, BytesN, token
};
use soroban_sdk::testutils::StellarAssetContract;
use vault::{VaultContract, VaultContractClient};

mod token_contract_wasm {
    soroban_sdk::contractimport!(file = "../wasm/soroban_token_contract.wasm");
}

const SCALAR_7: i128 = 10_000_000;

// ================================
// Test Setup Utilities
// ================================

fn create_token_contract(env: &Env, admin: &Address) -> Address {
    let token = env.register_stellar_asset_contract_v2(admin.clone());
    token.address()
}

fn setup_vault<'a>() -> (Env, Address, Address, Address, Address, VaultContractClient<'a>) {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.mock_all_auths();

    env.ledger().set_min_temp_entry_ttl(17280);
    env.ledger().set_min_persistent_entry_ttl(2073600);

    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let strategy1 = Address::generate(&env);

    let token = create_token_contract(&env, &admin);
    let token_wasm_hash = env.deployer().upload_contract_wasm(token_contract_wasm::WASM);

    let strategies = Vec::from_array(&env, [strategy1.clone()]);
    let vault_address = env.register(
        VaultContract,
        (
            token.clone(),
            token_wasm_hash,
            String::from_str(&env, "Test Vault Shares"),
            String::from_str(&env, "TVS"),
            strategies,
            300u64, // 5 minutes lock time
            SCALAR_7 / 10, // 10% penalty rate
        )
    );
    let vault = VaultContractClient::new(&env, &vault_address);

    // Fund users
    let token_client = StellarAssetClient::new(&env, &token);
    token_client.mint(&user1, &(50_000 * SCALAR_7));
    token_client.mint(&user2, &(30_000 * SCALAR_7));

    (env, user1, user2, strategy1, token, vault)
}

fn advance_time_past_lock(env: &Env) {
    env.ledger().set_timestamp(env.ledger().timestamp() + 301);
}

// ================================
// Basic Functionality Tests
// ================================

#[test]
fn test_vault_initialization_and_getters() {
    let (env, _, _, strategy1, token, vault) = setup_vault();

    // Test all basic getter functions
    let vault_token = vault.token();
    let share_token = vault.share_token();
    let total_shares = vault.total_shares();
    let strategy_impact = vault.net_impact(&strategy1);

    assert_eq!(vault_token, token);
    assert_ne!(share_token, token); // Share token should be different
    assert_eq!(total_shares, 0);
    assert_eq!(strategy_impact, 0);

    println!("✅ Vault initialization successful");
    println!("  Underlying token: {:?}", vault_token);
    println!("  Share token: {:?}", share_token);
    println!("  Initial total shares: {}", total_shares);
    println!("  Initial strategy impact: {}", strategy_impact);
}

#[test]
fn test_first_deposit_one_to_one_ratio() {
    let (env, user1, _, _, token, vault) = setup_vault();

    let token_client = TokenClient::new(&env, &token);
    let initial_balance = token_client.balance(&user1);

    println!("Initial balance: {}", initial_balance);

    // First deposit should get 1:1 ratio
    let deposit_amount = 1000 * SCALAR_7;
    let shares_received = vault.deposit(&deposit_amount, &user1);

    assert_eq!(shares_received, deposit_amount);
    assert_eq!(vault.total_shares(), deposit_amount);

    // Check balances
    let new_user_balance = token_client.balance(&user1);
    let vault_balance = token_client.balance(&vault.address);

    assert_eq!(new_user_balance, initial_balance - deposit_amount);
    assert_eq!(vault_balance, deposit_amount);

    println!("✅ First deposit works with 1:1 ratio");
    println!("  Deposited: {} tokens", deposit_amount);
    println!("  Received: {} shares", shares_received);
    println!("  Vault balance: {}", vault_balance);
}

#[test]
fn test_multiple_deposits_different_users() {
    let (env, user1, user2, _, token, vault) = setup_vault();

    let token_client = TokenClient::new(&env, &token);

    // User1 deposits first
    let deposit1 = 2000 * SCALAR_7;
    let shares1 = vault.deposit(&deposit1, &user1);
    assert_eq!(shares1, deposit1); // 1:1 for first deposit

    // User2 deposits immediately after (should also be 1:1 since no profit yet)
    let deposit2 = 1500 * SCALAR_7;
    let shares2 = vault.deposit(&deposit2, &user2);
    assert_eq!(shares2, deposit2); // 1:1 for second deposit

    // Check total state
    let total_shares = vault.total_shares();
    let vault_balance = token_client.balance(&vault.address);

    assert_eq!(total_shares, shares1 + shares2);
    assert_eq!(vault_balance, deposit1 + deposit2);

    println!("✅ Multiple user deposits work correctly");
    println!("  User1: {} tokens -> {} shares", deposit1, shares1);
    println!("  User2: {} tokens -> {} shares", deposit2, shares2);
    println!("  Total shares: {}", total_shares);
    println!("  Total vault balance: {}", vault_balance);
}

#[test]
fn test_deposit_after_vault_appreciation() {
    let (env, user1, user2, _, token, vault) = setup_vault();

    let token_client = TokenClient::new(&env, &token);

    // User1 makes initial deposit
    let initial_deposit = 1000 * SCALAR_7;
    let initial_shares = vault.deposit(&initial_deposit, &user1);

    // Simulate vault appreciation by transferring extra tokens to vault
    let profit = 200 * SCALAR_7; // 20% profit
    token_client.transfer(&user1, &vault.address, &profit);

    // User2 deposits after appreciation
    let second_deposit = 600 * SCALAR_7;
    let second_shares = vault.deposit(&second_deposit, &user2);

    // Calculate expected shares for user2
    // Total vault value: 1000 + 200 = 1200 tokens
    // Total shares: 1000 shares
    // Share price: 1200/1000 = 1.2 tokens per share
    // User2 should get: 600/1.2 = 500 shares
    let expected_shares = second_deposit * initial_shares / (initial_deposit + profit);

    // Allow small rounding differences
    let diff = if second_shares > expected_shares {
        second_shares - expected_shares
    } else {
        expected_shares - second_shares
    };
    assert!(diff < SCALAR_7 / 100); // Less than 0.01 difference

    println!("✅ Deposit after vault appreciation works correctly");
    println!("  Initial: {} tokens -> {} shares", initial_deposit, initial_shares);
    println!("  Added profit: {} tokens", profit);
    println!("  Second deposit: {} tokens -> {} shares", second_deposit, second_shares);
    println!("  Expected shares for user2: {}", expected_shares);
    println!("  Share price after profit: {}", (initial_deposit + profit) * SCALAR_7 / initial_shares);
}

// ================================
// Withdrawal Tests
// ================================

#[test]
fn test_withdrawal_queue_and_execution() {
    let (env, user1, _, _, token, vault) = setup_vault();

    let token_client = TokenClient::new(&env, &token);
    let initial_balance = token_client.balance(&user1);

    // Deposit first
    let deposit_amount = 1000 * SCALAR_7;
    let shares = vault.deposit(&deposit_amount, &user1);

    // Queue withdrawal for half
    let withdraw_shares = shares / 2;
    vault.queue_withdraw(&withdraw_shares, &user1);

    // Check shares are locked in vault
    let share_token = vault.share_token();
    let share_client = TokenClient::new(&env, &share_token);
    let user_shares = share_client.balance(&user1);
    let vault_shares = share_client.balance(&vault.address);

    assert_eq!(user_shares, shares - withdraw_shares);
    assert_eq!(vault_shares, withdraw_shares);

    // Execute withdrawal after lock time
    advance_time_past_lock(&env);
    let withdrawal_amount = vault.withdraw(&user1);

    // Check final balances
    let final_balance = token_client.balance(&user1);
    let expected_withdrawal = deposit_amount / 2; // Half of deposit

    // Allow small tolerance for rounding
    let diff = if withdrawal_amount > expected_withdrawal {
        withdrawal_amount - expected_withdrawal
    } else {
        expected_withdrawal - withdrawal_amount
    };
    assert!(diff < SCALAR_7); // Less than 1 token difference

    println!("✅ Withdrawal queue and execution works");
    println!("  Deposited: {} tokens", deposit_amount);
    println!("  Queued withdrawal: {} shares", withdraw_shares);
    println!("  Withdrew: {} tokens", withdrawal_amount);
    println!("  Expected: ~{} tokens", expected_withdrawal);
}

#[test]
fn test_withdrawal_cancellation() {
    let (env, user1, _, _, _, vault) = setup_vault();

    // Deposit and queue withdrawal
    let shares = vault.deposit(&(2000 * SCALAR_7), &user1);
    let withdraw_shares = shares / 3;
    vault.queue_withdraw(&withdraw_shares, &user1);

    // Cancel withdrawal
    vault.cancel_withdraw(&user1);

    // Check shares are returned to user
    let share_token = vault.share_token();
    let share_client = TokenClient::new(&env, &share_token);
    let user_shares = share_client.balance(&user1);
    let vault_shares = share_client.balance(&vault.address);

    assert_eq!(user_shares, shares);
    assert_eq!(vault_shares, 0);

    println!("✅ Withdrawal cancellation returns shares correctly");
}

#[test]
fn test_full_withdrawal_cycle() {
    let (env, user1, _, _, token, vault) = setup_vault();

    let token_client = TokenClient::new(&env, &token);
    let initial_balance = token_client.balance(&user1);

    // Complete cycle: deposit -> queue -> withdraw all
    let deposit_amount = 1500 * SCALAR_7;
    let shares = vault.deposit(&deposit_amount, &user1);

    vault.queue_withdraw(&shares, &user1);
    advance_time_past_lock(&env);
    let withdrawal_amount = vault.withdraw(&user1);

    // User should get back approximately what they put in
    let final_balance = token_client.balance(&user1);
    let net_change = final_balance - initial_balance + deposit_amount;

    assert_eq!(net_change, withdrawal_amount);
    assert!(withdrawal_amount >= deposit_amount * 9 / 10); // At least 90% back

    // Vault should have minimal remaining balance
    let vault_balance = token_client.balance(&vault.address);
    assert!(vault_balance < SCALAR_7); // Less than 1 token remaining

    println!("✅ Full withdrawal cycle works");
    println!("  Deposited: {} tokens", deposit_amount);
    println!("  Withdrew: {} tokens", withdrawal_amount);
    println!("  Recovery rate: {}%", withdrawal_amount * 100 / deposit_amount);
}

// ================================
// Strategy Operation Tests
// ================================

#[test]
fn test_strategy_borrow_and_repay() {
    let (env, user1, _, strategy1, token, vault) = setup_vault();

    let token_client = TokenClient::new(&env, &token);

    // Provide vault with liquidity
    let liquidity = 5000 * SCALAR_7;
    vault.deposit(&liquidity, &user1);

    let initial_vault_balance = token_client.balance(&vault.address);
    let initial_strategy_balance = token_client.balance(&strategy1);

    // Strategy borrows from vault
    let borrow_amount = 2000 * SCALAR_7;
    vault.transfer_to(&strategy1, &borrow_amount);

    // Check state after borrow
    let vault_balance = token_client.balance(&vault.address);
    let strategy_balance = token_client.balance(&strategy1);
    let net_impact = vault.net_impact(&strategy1);

    assert_eq!(vault_balance, initial_vault_balance - borrow_amount);
    assert_eq!(strategy_balance, initial_strategy_balance + borrow_amount);
    assert_eq!(net_impact, -borrow_amount); // Negative = vault lent money

    // Strategy repays exact amount
    vault.transfer_from(&strategy1, &borrow_amount);

    // Check final state
    let final_vault_balance = token_client.balance(&vault.address);
    let final_strategy_balance = token_client.balance(&strategy1);
    let final_net_impact = vault.net_impact(&strategy1);

    assert_eq!(final_vault_balance, initial_vault_balance);
    assert_eq!(final_strategy_balance, initial_strategy_balance);
    assert_eq!(final_net_impact, 0); // Back to even

    println!("✅ Strategy borrow and exact repay works");
    println!("  Borrowed: {} tokens", borrow_amount);
    println!("  Repaid: {} tokens", borrow_amount);
    println!("  Final net impact: {}", final_net_impact);
}

#[test]
fn test_strategy_generates_profit() {
    let (env, user1, _, strategy1, token, vault) = setup_vault();

    let token_client = TokenClient::new(&env, &token);

    // Provide liquidity
    vault.deposit(&(3000 * SCALAR_7), &user1);
    let initial_vault_balance = token_client.balance(&vault.address);

    // Strategy borrows and returns with profit
    let borrow_amount = 1000 * SCALAR_7;
    let profit = 100 * SCALAR_7; // 10% profit
    let return_amount = borrow_amount + profit;

    vault.transfer_to(&strategy1, &borrow_amount);
    vault.transfer_from(&strategy1, &return_amount);

    // Check final state
    let final_vault_balance = token_client.balance(&vault.address);
    let net_impact = vault.net_impact(&strategy1);

    assert_eq!(final_vault_balance, initial_vault_balance + profit);
    assert_eq!(net_impact, profit); // Positive = profit for vault

    println!("✅ Strategy profit generation works");
    println!("  Borrowed: {} tokens", borrow_amount);
    println!("  Returned: {} tokens", return_amount);
    println!("  Profit: {} tokens", profit);
    println!("  Net impact: {}", net_impact);
}

#[test]
fn test_strategy_incurs_loss() {
    let (env, user1, _, strategy1, token, vault) = setup_vault();

    let token_client = TokenClient::new(&env, &token);

    // Provide liquidity
    vault.deposit(&(4000 * SCALAR_7), &user1);
    let initial_vault_balance = token_client.balance(&vault.address);

    // Strategy borrows and returns with loss
    let borrow_amount = 1500 * SCALAR_7;
    let loss = 150 * SCALAR_7; // 10% loss
    let return_amount = borrow_amount - loss;

    vault.transfer_to(&strategy1, &borrow_amount);
    vault.transfer_from(&strategy1, &return_amount);

    // Check final state
    let final_vault_balance = token_client.balance(&vault.address);
    let net_impact = vault.net_impact(&strategy1);

    assert_eq!(final_vault_balance, initial_vault_balance - loss);
    assert_eq!(net_impact, -loss); // Negative = loss for vault

    println!("✅ Strategy loss handling works");
    println!("  Borrowed: {} tokens", borrow_amount);
    println!("  Returned: {} tokens", return_amount);
    println!("  Loss: {} tokens", loss);
    println!("  Net impact: {}", net_impact);
}

#[test]
fn test_multiple_strategy_operations() {
    let (env, user1, _, strategy1, token, vault) = setup_vault();

    // Provide ample liquidity
    vault.deposit(&(10000 * SCALAR_7), &user1);

    // Multiple borrow/return cycles
    let operations = [
        (500 * SCALAR_7, 520 * SCALAR_7),  // 4% profit
        (300 * SCALAR_7, 285 * SCALAR_7),  // 5% loss
        (800 * SCALAR_7, 840 * SCALAR_7),  // 5% profit
    ];

    let mut running_impact = 0i128;

    for (i, (borrow, return_amt)) in operations.iter().enumerate() {
        vault.transfer_to(&strategy1, borrow);
        vault.transfer_from(&strategy1, return_amt);

        let pnl = return_amt - borrow;
        running_impact += pnl;

        let current_impact = vault.net_impact(&strategy1);
        assert_eq!(current_impact, running_impact);

        println!("  Operation {}: borrowed {}, returned {}, P&L {}",
                 i + 1, borrow, return_amt, pnl);
    }

    let final_impact = vault.net_impact(&strategy1);
    println!("✅ Multiple strategy operations work");
    println!("  Final net impact: {}", final_impact);
    println!("  Expected impact: {}", running_impact);
}

// ================================
// Error Condition Tests
// ================================

#[test]
#[should_panic(expected = "Error(Contract, #4041)")]
fn test_zero_deposit_fails() {
    let (env, user1, _, _, _, vault) = setup_vault();

    vault.deposit(&0, &user1);
}

#[test]
#[should_panic(expected = "Error(Contract, #4041)")]
fn test_zero_withdrawal_queue_fails() {
    let (env, user1, _, _, _, vault) = setup_vault();

    vault.queue_withdraw(&0, &user1);
}

#[test]
#[should_panic(expected = "Error(Contract, #4041)")]
fn test_zero_strategy_transfer_fails() {
    let (env, _, _, strategy1, _, vault) = setup_vault();

    vault.transfer_to(&strategy1, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #4047)")]
fn test_early_withdrawal_fails() {
    let (env, user1, _, _, _, vault) = setup_vault();

    let shares = vault.deposit(&(1000 * SCALAR_7), &user1);
    vault.queue_withdraw(&shares, &user1);

    // Try to withdraw immediately (before lock time expires)
    vault.withdraw(&user1);
}

#[test]
#[should_panic(expected = "Error(Contract, #4046)")]
fn test_double_withdrawal_queue_fails() {
    let (env, user1, _, _, _, vault) = setup_vault();

    let shares = vault.deposit(&(2000 * SCALAR_7), &user1);

    // Queue first withdrawal
    vault.queue_withdraw(&(shares / 2), &user1);

    // Try to queue second withdrawal (should fail)
    vault.queue_withdraw(&(shares / 4), &user1);
}

#[test]
#[should_panic] // Should panic due to insufficient balance
fn test_strategy_overdraw_fails() {
    let (env, user1, _, strategy1, _, vault) = setup_vault();

    // Only deposit small amount
    vault.deposit(&(500 * SCALAR_7), &user1);

    // Try to withdraw more than available
    vault.transfer_to(&strategy1, &(1000 * SCALAR_7));
}

// ================================
// Edge Cases and Precision Tests
// ================================

#[test]
fn test_minimal_amounts() {
    let (env, user1, _, _, _, vault) = setup_vault();

    // Test smallest possible deposit
    let tiny_deposit = 1; // 1 stroop
    let shares = vault.deposit(&tiny_deposit, &user1);

    assert_eq!(shares, tiny_deposit);
    assert_eq!(vault.total_shares(), tiny_deposit);

    println!("✅ Minimal amount deposit works");
    println!("  Deposited: {} stroop", tiny_deposit);
    println!("  Received: {} shares", shares);
}

#[test]
fn test_large_amounts() {
    let (env, user1, _, _, token, vault) = setup_vault();

    // Fund user with large amount
    let token_client = StellarAssetClient::new(&env, &token);
    let large_amount = 1_000_000 * SCALAR_7; // 1M tokens
    token_client.mint(&user1, &large_amount);

    // Test large deposit
    let shares = vault.deposit(&large_amount, &user1);
    assert_eq!(shares, large_amount);

    println!("✅ Large amount deposit works");
    println!("  Deposited: {} tokens", large_amount);
    println!("  Received: {} shares", shares);
}

#[test]
fn test_withdrawal_at_exact_unlock_time() {
    let (env, user1, _, _, _, vault) = setup_vault();

    let shares = vault.deposit(&(1000 * SCALAR_7), &user1);
    vault.queue_withdraw(&shares, &user1);

    // Advance to exactly the unlock time (not past it)
    env.ledger().set_timestamp(env.ledger().timestamp() + 300);

    let withdrawal_amount = vault.withdraw(&user1);
    assert!(withdrawal_amount > 0);

    println!("✅ Withdrawal at exact unlock time works");
    println!("  Withdrawal amount: {}", withdrawal_amount);
}

#[test]
fn test_share_price_precision() {
    let (env, user1, user2, _, token, vault) = setup_vault();
    let token_client = TokenClient::new(&env, &token);

    // Large initial deposit
    let large_deposit = 100_000 * SCALAR_7;
    vault.deposit(&large_deposit, &user1);

    // Add tiny profit
    let tiny_profit = 1; // 1 stroop profit
    token_client.transfer(&user1, &vault.address, &tiny_profit);

    // Small deposit should still work
    let small_deposit = 100 * SCALAR_7;
    let shares = vault.deposit(&small_deposit, &user2);

    assert!(shares > 0);
    assert!(shares < small_deposit); // Should get slightly fewer shares due to appreciation

    println!("✅ Share price precision maintained with tiny profit");
    println!("  Large deposit: {}", large_deposit);
    println!("  Tiny profit: {}", tiny_profit);
    println!("  Small deposit: {} -> {} shares", small_deposit, shares);
}

#[test]
fn test_vault_state_consistency() {
    let (env, user1, user2, strategy1, token, vault) = setup_vault();

    let token_client = TokenClient::new(&env, &token);

    // Perform various operations
    vault.deposit(&(2000 * SCALAR_7), &user1);
    vault.deposit(&(1000 * SCALAR_7), &user2);

    vault.transfer_to(&strategy1, &(500 * SCALAR_7));
    vault.transfer_from(&strategy1, &(550 * SCALAR_7)); // 50 token profit

    // Check consistency
    let vault_balance = token_client.balance(&vault.address);
    let total_shares = vault.total_shares();
    let strategy_impact = vault.net_impact(&strategy1);

    // Vault should have 3000 + 50 = 3050 tokens
    assert_eq!(vault_balance, 3050 * SCALAR_7);

    // Total shares should be 3000 (no change from strategy operations)
    assert_eq!(total_shares, 3000 * SCALAR_7);

    // Strategy impact should be +50
    assert_eq!(strategy_impact, 50 * SCALAR_7);

    println!("✅ Vault state consistency maintained");
    println!("  Vault balance: {}", vault_balance);
    println!("  Total shares: {}", total_shares);
    println!("  Strategy impact: {}", strategy_impact);
    println!("  Share price: {}", vault_balance * SCALAR_7 / total_shares);
}