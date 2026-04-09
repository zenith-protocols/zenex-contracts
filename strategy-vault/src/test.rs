
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, String,
};

use crate::{StrategyVaultContract, StrategyVaultContractClient};

const SCALAR_7: i128 = 10_000_000;
const LOCK_TIME: u64 = 300;

fn setup_test<'a>() -> (
    Env,
    StrategyVaultContractClient<'a>,
    Address,
    Address,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(admin.clone());
    let user = Address::generate(&env);
    let strategy = Address::generate(&env);

    // Fund user
    StellarAssetClient::new(&env, &token.address()).mint(&user, &(100_000 * SCALAR_7));

    // Deploy vault
    let vault_address = env.register(
        StrategyVaultContract,
        (
            String::from_str(&env, "Vault Shares"),
            String::from_str(&env, "vTKN"),
            token.address(),
            0u32,
            strategy.clone(),
            LOCK_TIME,
        ),
    );

    let vault = StrategyVaultContractClient::new(&env, &vault_address);
    (env, vault, token.address(), user, strategy)
}

// ==================== Lock Mechanism Tests ====================

#[test]
fn test_deposit_sets_lock() {
    let (_env, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    assert!(vault.available_shares(&user) == 0);
}

#[test]
fn test_mint_sets_lock() {
    let (_env, vault, _, user, _) = setup_test();

    vault.mint(&(1000 * SCALAR_7), &user, &user, &user);

    assert!(vault.available_shares(&user) == 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #791)")] // SharesLocked
fn test_withdraw_while_locked_fails_via_withdraw() {
    let (_, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    assert!(vault.available_shares(&user) == 0);

    // All shares are locked (just deposited), withdraw fails
    vault.withdraw(&(500 * SCALAR_7), &user, &user, &user);
}

#[test]
fn test_unlock_after_lock_time() {
    let (env, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    // Advance past lock time
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);

    assert_eq!(vault.available_shares(&user), vault.balance(&user));
    assert!(vault.max_redeem(&user) > 0);
}

#[test]
fn test_new_deposit_resets_lock() {
    let (env, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    // Advance halfway
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME / 2);
    assert!(vault.available_shares(&user) == 0);

    // New deposit resets lock and accumulates locked shares
    vault.deposit(&(500 * SCALAR_7), &user, &user, &user);

    // Advance another half - still locked due to reset
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME / 2);
    assert!(vault.available_shares(&user) == 0);

    // Advance past new lock
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME / 2 + 1);
    assert_eq!(vault.available_shares(&user), vault.balance(&user));
}

#[test]
#[should_panic(expected = "Error(Contract, #791)")] // SharesLocked
fn test_redeem_while_locked_fails() {
    let (_, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    assert!(vault.available_shares(&user) == 0);

    // All shares are locked, redeem fails
    vault.redeem(&(500 * SCALAR_7), &user, &user, &user);
}

#[test]
#[should_panic(expected = "Error(Contract, #791)")] // SharesLocked
fn test_withdraw_while_locked_fails() {
    let (_, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    assert!(vault.available_shares(&user) == 0);

    // All shares are locked, withdraw fails
    vault.withdraw(&(500 * SCALAR_7), &user, &user, &user);
}

#[test]
fn test_redeem_after_unlock_succeeds() {
    let (env, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);
    assert_eq!(vault.available_shares(&user), vault.balance(&user));

    let assets = vault.redeem(&(500 * SCALAR_7), &user, &user, &user);
    assert!(assets > 0);
}

#[test]
fn test_withdraw_after_unlock_succeeds() {
    let (env, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);
    assert_eq!(vault.available_shares(&user), vault.balance(&user));

    let shares = vault.withdraw(&(500 * SCALAR_7), &user, &user, &user);
    assert!(shares > 0);
}

// ==================== Share-Aware Lock Tests ====================

#[test]
fn test_old_shares_available_while_new_locked() {
    let (env, vault, _, user, _) = setup_test();

    // First deposit: 1000 shares
    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    // Wait for lock to expire
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);
    assert_eq!(vault.available_shares(&user), vault.balance(&user));

    // Second deposit: 200 shares (only these are locked)
    vault.deposit(&(200 * SCALAR_7), &user, &user, &user);
    assert!(vault.available_shares(&user) < vault.balance(&user));

    // 1000 old shares are available, 200 new shares are locked
    assert_eq!(vault.available_shares(&user), 1000 * SCALAR_7);

    // Can withdraw up to 1000 shares
    let shares = vault.withdraw(&(500 * SCALAR_7), &user, &user, &user);
    assert!(shares > 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #791)")] // SharesLocked
fn test_cannot_withdraw_locked_shares() {
    let (env, vault, _, user, _) = setup_test();

    // First deposit: 1000
    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    // Wait for lock to expire
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);

    // Second deposit: 200 (locked)
    vault.deposit(&(200 * SCALAR_7), &user, &user, &user);

    // Available = 1000, trying to withdraw 1100 should fail
    vault.withdraw(&(1100 * SCALAR_7), &user, &user, &user);
}

#[test]
fn test_all_shares_available_after_lock_expires() {
    let (env, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);
    vault.deposit(&(200 * SCALAR_7), &user, &user, &user);

    // Wait for second lock to expire
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);

    // All 1200 shares available
    assert_eq!(vault.available_shares(&user), 1200 * SCALAR_7);
}

#[test]
fn test_multiple_deposits_within_lock_accumulate() {
    let (env, vault, _, user, _) = setup_test();

    // Two deposits within same lock window
    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    env.ledger()
        .set_timestamp(env.ledger().timestamp() + 10); // 10s later, still locked

    vault.deposit(&(500 * SCALAR_7), &user, &user, &user);

    // Both deposits are locked (1500 total)
    assert_eq!(vault.available_shares(&user), 0);

    // Wait for lock to expire
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);

    // All 1500 available
    assert_eq!(vault.available_shares(&user), 1500 * SCALAR_7);
}

#[test]
fn test_expired_lock_resets_on_new_deposit() {
    let (env, vault, _, user, _) = setup_test();

    // Deposit 1000, wait for lock
    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);

    // Deposit 100 more (only 100 locked, not 1100)
    vault.deposit(&(100 * SCALAR_7), &user, &user, &user);

    // 1000 available (old), 100 locked (new)
    assert_eq!(vault.available_shares(&user), 1000 * SCALAR_7);
}

#[test]
fn test_redeem_old_shares_while_new_locked() {
    let (env, vault, _, user, _) = setup_test();

    vault.deposit(&(2000 * SCALAR_7), &user, &user, &user);
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);

    vault.deposit(&(500 * SCALAR_7), &user, &user, &user);

    // Redeem 1000 of the 2000 old shares (available = 2000)
    let assets = vault.redeem(&(1000 * SCALAR_7), &user, &user, &user);
    assert!(assets > 0);

    // Remaining: 1000 old + 500 locked = 1500 total, 1000 available
    assert_eq!(vault.available_shares(&user), 1000 * SCALAR_7);
}

// ==================== Transfer Lock Tests ====================

#[test]
fn test_user_without_deposit_history_is_not_locked() {
    let (env, vault, _, _user, _) = setup_test();
    let recipient = Address::generate(&env);

    assert_eq!(vault.available_shares(&recipient), vault.balance(&recipient));
}

#[test]
#[should_panic(expected = "Error(Contract, #791)")] // SharesLocked
fn test_transfer_while_locked_fails() {
    let (env, vault, _, user, _) = setup_test();
    let recipient = Address::generate(&env);

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    assert!(vault.available_shares(&user) == 0);

    // All shares locked, transfer fails
    vault.transfer(&user, &recipient, &(500 * SCALAR_7));
}

#[test]
fn test_transfer_after_unlock_succeeds() {
    let (env, vault, _, user, _) = setup_test();
    let recipient = Address::generate(&env);

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);
    assert_eq!(vault.available_shares(&user), vault.balance(&user));

    vault.transfer(&user, &recipient, &(500 * SCALAR_7));
    assert_eq!(vault.available_shares(&recipient), vault.balance(&recipient));
    assert!(vault.max_redeem(&recipient) > 0);
}

#[test]
fn test_transfer_old_shares_while_new_locked() {
    let (env, vault, _, user, _) = setup_test();
    let recipient = Address::generate(&env);

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);
    vault.deposit(&(200 * SCALAR_7), &user, &user, &user);

    // Can transfer up to 1000 old shares
    vault.transfer(&user, &recipient, &(800 * SCALAR_7));
    assert_eq!(vault.available_shares(&user), 200 * SCALAR_7);
}

#[test]
#[should_panic(expected = "Error(Contract, #791)")] // SharesLocked
fn test_transfer_from_while_locked_fails() {
    let (env, vault, _, user, _) = setup_test();
    let spender = Address::generate(&env);
    let recipient = Address::generate(&env);

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    vault.approve(&user, &spender, &(500 * SCALAR_7), &1000);

    vault.transfer_from(&spender, &user, &recipient, &(500 * SCALAR_7));
}

#[test]
fn test_transfer_from_after_unlock_succeeds() {
    let (env, vault, _, user, _) = setup_test();
    let spender = Address::generate(&env);
    let recipient = Address::generate(&env);

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    vault.approve(&user, &spender, &(500 * SCALAR_7), &1000);

    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);

    vault.transfer_from(&spender, &user, &recipient, &(500 * SCALAR_7));
    assert_eq!(vault.available_shares(&recipient), vault.balance(&recipient));
    assert!(vault.max_redeem(&recipient) > 0);
}

// ==================== Strategy Tests ====================

#[test]
fn test_strategy_withdraw_decreases_assets() {
    let (_env, vault, _token, user, strategy) = setup_test();

    vault.deposit(&(10_000 * SCALAR_7), &user, &user, &user);
    let initial_assets = vault.total_assets();

    vault.strategy_withdraw(&strategy, &(2000 * SCALAR_7));

    assert_eq!(vault.total_assets(), initial_assets - 2000 * SCALAR_7);
}

#[test]
#[should_panic(expected = "Error(Contract, #792)")] // UnauthorizedStrategy
fn test_unauthorized_strategy_fails() {
    let (env, vault, _, user, _) = setup_test();
    let fake_strategy = Address::generate(&env);

    vault.deposit(&(10_000 * SCALAR_7), &user, &user, &user);
    vault.strategy_withdraw(&fake_strategy, &(1000 * SCALAR_7));
}

#[test]
#[should_panic(expected = "Error(Contract, #790)")] // InvalidAmount
fn test_zero_strategy_withdraw_fails() {
    let (_, vault, _, user, strategy) = setup_test();

    vault.deposit(&(10_000 * SCALAR_7), &user, &user, &user);
    vault.strategy_withdraw(&strategy, &0);
}
