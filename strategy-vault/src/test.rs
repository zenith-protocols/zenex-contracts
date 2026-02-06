#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, String, Vec,
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
    let strategies = Vec::from_array(&env, [strategy.clone()]);
    let vault_address = env.register(
        StrategyVaultContract,
        (
            String::from_str(&env, "Vault Shares"),
            String::from_str(&env, "vTKN"),
            token.address(),
            0u32,
            strategies,
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

    assert!(vault.lock_duration(&user) > 0);
    // Lock only blocks transfers, not withdrawals
    assert!(vault.max_redeem(&user) > 0);
}

#[test]
fn test_mint_sets_lock() {
    let (_env, vault, _, user, _) = setup_test();

    vault.mint(&(1000 * SCALAR_7), &user, &user, &user);

    assert!(vault.lock_duration(&user) > 0);
}

#[test]
fn test_max_withdraw_returns_value_when_locked() {
    let (_env, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    // Lock only blocks transfers, not withdrawals
    assert!(vault.max_withdraw(&user) > 0);
}

#[test]
fn test_lock_time_returns_configured_value() {
    let (_env, vault, _, _, _) = setup_test();

    assert_eq!(vault.lock_time(), LOCK_TIME);
}

#[test]
fn test_unlock_after_lock_time() {
    let (env, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    // Advance past lock time
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);

    assert_eq!(vault.lock_duration(&user), 0);
    assert!(vault.max_redeem(&user) > 0);
}

#[test]
fn test_new_deposit_resets_lock() {
    let (env, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    // Advance halfway
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME / 2);
    assert!(vault.lock_duration(&user) > 0);

    // New deposit resets lock
    vault.deposit(&(500 * SCALAR_7), &user, &user, &user);

    // Advance another half - still locked due to reset
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME / 2);
    assert!(vault.lock_duration(&user) > 0);

    // Advance past new lock
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME / 2 + 1);
    assert_eq!(vault.lock_duration(&user), 0);
}

#[test]
fn test_redeem_while_locked_succeeds() {
    let (_, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    assert!(vault.lock_duration(&user) > 0);

    // Redeem succeeds - lock only blocks transfers, not withdrawals
    let assets = vault.redeem(&(500 * SCALAR_7), &user, &user, &user);
    assert!(assets > 0);
}

#[test]
fn test_withdraw_while_locked_succeeds() {
    let (_, vault, _, user, _) = setup_test();

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    assert!(vault.lock_duration(&user) > 0);

    // Withdraw succeeds - lock only blocks transfers, not withdrawals
    let shares = vault.withdraw(&(500 * SCALAR_7), &user, &user, &user);
    assert!(shares > 0);
}

// ==================== Transfer Lock Tests ====================

#[test]
fn test_user_without_deposit_history_is_not_locked() {
    let (env, vault, _, _user, _) = setup_test();
    let recipient = Address::generate(&env);

    // User who never deposited is not locked (e.g. received shares via transfer)
    assert_eq!(vault.lock_duration(&recipient), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #421)")] // SharesLocked
fn test_transfer_while_locked_fails() {
    let (env, vault, _, user, _) = setup_test();
    let recipient = Address::generate(&env);

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    assert!(vault.lock_duration(&user) > 0);

    // Transfer should fail while locked
    vault.transfer(&user, &recipient, &(500 * SCALAR_7));
}

#[test]
fn test_transfer_after_unlock_succeeds() {
    let (env, vault, _, user, _) = setup_test();
    let recipient = Address::generate(&env);

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);

    // Wait for lock to expire
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);
    assert_eq!(vault.lock_duration(&user), 0);

    // Transfer should succeed
    vault.transfer(&user, &recipient, &(500 * SCALAR_7));

    // Recipient can immediately redeem (no deposit history)
    assert_eq!(vault.lock_duration(&recipient), 0);
    assert!(vault.max_redeem(&recipient) > 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #421)")] // SharesLocked
fn test_transfer_from_while_locked_fails() {
    let (env, vault, _, user, _) = setup_test();
    let spender = Address::generate(&env);
    let recipient = Address::generate(&env);

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    vault.approve(&user, &spender, &(500 * SCALAR_7), &1000);

    // transfer_from should fail while owner is locked
    vault.transfer_from(&spender, &user, &recipient, &(500 * SCALAR_7));
}

#[test]
fn test_transfer_from_after_unlock_succeeds() {
    let (env, vault, _, user, _) = setup_test();
    let spender = Address::generate(&env);
    let recipient = Address::generate(&env);

    vault.deposit(&(1000 * SCALAR_7), &user, &user, &user);
    vault.approve(&user, &spender, &(500 * SCALAR_7), &1000);

    // Wait for lock to expire
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + LOCK_TIME + 1);

    // transfer_from should succeed
    vault.transfer_from(&spender, &user, &recipient, &(500 * SCALAR_7));

    // Recipient can immediately redeem (no deposit history)
    assert_eq!(vault.lock_duration(&recipient), 0);
    assert!(vault.max_redeem(&recipient) > 0);
}

// ==================== Strategy Tests ====================

#[test]
fn test_strategy_withdraw_decreases_assets() {
    let (_env, vault, _token, user, strategy) = setup_test();

    vault.deposit(&(10_000 * SCALAR_7), &user, &user, &user);
    let initial_assets = vault.total_assets();

    // Strategy withdraws
    vault.strategy_withdraw(&strategy, &(2000 * SCALAR_7));

    assert_eq!(vault.total_assets(), initial_assets - 2000 * SCALAR_7);
}

#[test]
#[should_panic(expected = "Error(Contract, #422)")] // UnauthorizedStrategy
fn test_unauthorized_strategy_fails() {
    let (env, vault, _, user, _) = setup_test();
    let fake_strategy = Address::generate(&env);

    vault.deposit(&(10_000 * SCALAR_7), &user, &user, &user);
    vault.strategy_withdraw(&fake_strategy, &(1000 * SCALAR_7));
}

#[test]
#[should_panic(expected = "Error(Contract, #420)")] // InvalidAmount
fn test_zero_strategy_withdraw_fails() {
    let (_, vault, _, user, strategy) = setup_test();

    vault.deposit(&(10_000 * SCALAR_7), &user, &user, &user);
    vault.strategy_withdraw(&strategy, &0);
}
