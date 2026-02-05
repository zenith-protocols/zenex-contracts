#![cfg(test)]
extern crate std;

use soroban_sdk::{
    contract, contractimpl, map, testutils::Address as _, vec, Address, Bytes, Env, Map, String,
    Val, Vec,
};
use stellar_accounts::{
    policies::Policy,
    smart_account::{ContextRule, ContextRuleType, Signer},
};

use crate::contract::{ZenexAccount, ZenexAccountClient};

// ==========================================
// Mock Policy Contract
// ==========================================

#[contract]
struct MockPolicyContract;

#[contractimpl]
impl Policy for MockPolicyContract {
    type AccountParams = Val;

    fn can_enforce(
        _e: &Env,
        _context: soroban_sdk::auth::Context,
        _authenticated_signers: Vec<Signer>,
        _rule: ContextRule,
        _smart_account: Address,
    ) -> bool {
        true
    }

    fn enforce(
        _e: &Env,
        _context: soroban_sdk::auth::Context,
        _authenticated_signers: Vec<Signer>,
        _rule: ContextRule,
        _smart_account: Address,
    ) {
    }

    fn install(
        _e: &Env,
        _install_params: Self::AccountParams,
        _rule: ContextRule,
        _smart_account: Address,
    ) {
    }

    fn uninstall(_e: &Env, _rule: ContextRule, _smart_account: Address) {}
}

// ==========================================
// Helper Functions
// ==========================================

fn create_delegated_signer(e: &Env) -> Signer {
    Signer::Delegated(Address::generate(e))
}

fn create_external_signer(e: &Env, verifier: &Address, key_data: &[u8; 64]) -> Signer {
    Signer::External(verifier.clone(), Bytes::from_array(e, key_data))
}

fn create_client<'a>(e: &Env, primary_signer: Signer, policies: Map<Address, Val>) -> ZenexAccountClient<'a> {
    let address = e.register(ZenexAccount, (primary_signer, policies));
    ZenexAccountClient::new(e, &address)
}

// ==========================================
// Constructor Tests
// ==========================================

#[test]
fn test_create_account_with_delegated_signer() {
    let e = Env::default();
    let signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);

    let client = create_client(&e, signer, policies);

    // Verify the account was created with one context rule
    assert_eq!(client.get_context_rules_count(), 1);
}

#[test]
fn test_create_account_with_external_signer() {
    let e = Env::default();
    let verifier = Address::generate(&e);
    let key_data: [u8; 64] = *b"4cb5abf6ad79fbf5abbccafcc269d85cd2651ed4b885b5869f241aedf0a5ba29";
    let signer = create_external_signer(&e, &verifier, &key_data);
    let policies: Map<Address, Val> = Map::new(&e);

    let client = create_client(&e, signer, policies);

    assert_eq!(client.get_context_rules_count(), 1);
}

#[test]
fn test_create_account_with_policy() {
    let e = Env::default();
    let signer = create_delegated_signer(&e);
    let policy = e.register(MockPolicyContract, ());
    let policies = map![&e, (policy.clone(), Val::from_void().into())];

    let client = create_client(&e, signer, policies);

    assert_eq!(client.get_context_rules_count(), 1);

    // Get the default rule and verify policy is attached
    let rules = client.get_context_rules(&ContextRuleType::Default);
    assert_eq!(rules.len(), 1);
    let rule = rules.get(0).unwrap();
    assert!(rule.policies.contains(policy));
}

// ==========================================
// Context Rule Tests
// ==========================================

#[test]
fn test_get_context_rule() {
    let e = Env::default();
    e.mock_all_auths();

    let signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, signer, policies);

    // Get the default rule (id 0)
    let rule = client.get_context_rule(&0);
    assert_eq!(rule.id, 0);
    assert_eq!(rule.name, String::from_str(&e, "primary"));
}

#[test]
fn test_get_context_rules_by_type() {
    let e = Env::default();
    e.mock_all_auths();

    let signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, signer, policies);

    let default_rules = client.get_context_rules(&ContextRuleType::Default);
    assert_eq!(default_rules.len(), 1);

    // No CallContract rules initially
    let call_rules = client.get_context_rules(&ContextRuleType::CallContract(Address::generate(&e)));
    assert_eq!(call_rules.len(), 0);
}

#[test]
fn test_add_context_rule() {
    let e = Env::default();
    e.mock_all_auths();

    let signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, signer.clone(), policies.clone());

    // Add a new context rule for a specific contract
    let target_contract = Address::generate(&e);
    let new_rule = client.add_context_rule(
        &ContextRuleType::CallContract(target_contract.clone()),
        &String::from_str(&e, "trading_rule"),
        &None,
        &vec![&e, signer],
        &policies,
    );

    assert_eq!(new_rule.id, 1);
    assert_eq!(new_rule.name, String::from_str(&e, "trading_rule"));
    assert_eq!(client.get_context_rules_count(), 2);
}

#[test]
fn test_update_context_rule_name() {
    let e = Env::default();
    e.mock_all_auths();

    let signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, signer, policies);

    let updated_rule = client.update_context_rule_name(&0, &String::from_str(&e, "main_signer"));
    assert_eq!(updated_rule.name, String::from_str(&e, "main_signer"));
}

#[test]
fn test_update_context_rule_valid_until() {
    let e = Env::default();
    e.mock_all_auths();

    let signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, signer, policies);

    // Set expiration
    let updated_rule = client.update_context_rule_valid_until(&0, &Some(1000000));
    assert_eq!(updated_rule.valid_until, Some(1000000));

    // Clear expiration
    let cleared_rule = client.update_context_rule_valid_until(&0, &None);
    assert_eq!(cleared_rule.valid_until, None);
}

#[test]
fn test_remove_context_rule() {
    let e = Env::default();
    e.mock_all_auths();

    let signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, signer.clone(), policies.clone());

    // Add a rule first
    let target = Address::generate(&e);
    client.add_context_rule(
        &ContextRuleType::CallContract(target),
        &String::from_str(&e, "temp_rule"),
        &None,
        &vec![&e, signer],
        &policies,
    );
    assert_eq!(client.get_context_rules_count(), 2);

    // Remove the added rule
    client.remove_context_rule(&1);
    // Note: count doesn't decrease, but the rule is marked as removed
}

// ==========================================
// Signer Tests
// ==========================================

#[test]
fn test_add_signer() {
    let e = Env::default();
    e.mock_all_auths();

    let primary_signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, primary_signer, policies);

    // Add a new signer to the default rule
    let new_signer = create_delegated_signer(&e);
    client.add_signer(&0, &new_signer);

    let rule = client.get_context_rule(&0);
    assert_eq!(rule.signers.len(), 2);
}

#[test]
fn test_remove_signer() {
    let e = Env::default();
    e.mock_all_auths();

    let primary_signer = create_delegated_signer(&e);
    let secondary_signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, primary_signer, policies);

    // Add second signer
    client.add_signer(&0, &secondary_signer.clone());
    assert_eq!(client.get_context_rule(&0).signers.len(), 2);

    // Remove the secondary signer
    client.remove_signer(&0, &secondary_signer);
    assert_eq!(client.get_context_rule(&0).signers.len(), 1);
}

// ==========================================
// Policy Tests
// ==========================================

#[test]
fn test_add_policy() {
    let e = Env::default();
    e.mock_all_auths();

    let signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, signer, policies);

    // Add a policy to the default rule
    let policy = e.register(MockPolicyContract, ());
    client.add_policy(&0, &policy, &Val::from_void().into());

    let rule = client.get_context_rule(&0);
    assert_eq!(rule.policies.len(), 1);
    assert!(rule.policies.contains(policy));
}

#[test]
fn test_remove_policy() {
    let e = Env::default();
    e.mock_all_auths();

    let signer = create_delegated_signer(&e);
    let policy = e.register(MockPolicyContract, ());
    let policies = map![&e, (policy.clone(), Val::from_void().into())];
    let client = create_client(&e, signer, policies);

    // Verify policy exists
    assert_eq!(client.get_context_rule(&0).policies.len(), 1);

    // Add another signer first (can't have empty signers AND empty policies)
    let backup_signer = create_delegated_signer(&e);
    client.add_signer(&0, &backup_signer);

    // Remove the policy
    client.remove_policy(&0, &policy);
    assert_eq!(client.get_context_rule(&0).policies.len(), 0);
}

// ==========================================
// Execute Tests
// ==========================================

#[contract]
struct MockTargetContract;

#[contractimpl]
impl MockTargetContract {
    pub fn test_fn(_e: Env) -> u32 {
        42
    }
}

#[test]
fn test_execute() {
    let e = Env::default();
    e.mock_all_auths();

    let signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, signer, policies);

    // Register a target contract
    let target = e.register(MockTargetContract, ());

    // Execute a call through the smart account
    client.execute(
        &target,
        &soroban_sdk::Symbol::new(&e, "test_fn"),
        &vec![&e],
    );
}

// ==========================================
// Multi-signer Account Test (like multisig example)
// ==========================================

#[test]
fn test_create_multisig_account() {
    let e = Env::default();
    let verifier = Address::generate(&e);
    let policy = e.register(MockPolicyContract, ());

    // Create account with external signers (like the multisig example)
    let primary_signer = Signer::External(
        verifier.clone(),
        Bytes::from_array(
            &e,
            b"4cb5abf6ad79fbf5abbccafcc269d85cd2651ed4b885b5869f241aedf0a5ba29",
        ),
    );

    let policies = map![&e, (policy, Val::from_void().into())];
    let client = create_client(&e, primary_signer, policies);

    // Add second signer
    e.mock_all_auths();
    let second_signer = Signer::External(
        verifier.clone(),
        Bytes::from_array(
            &e,
            b"3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29",
        ),
    );
    client.add_signer(&0, &second_signer);

    let rule = client.get_context_rule(&0);
    assert_eq!(rule.signers.len(), 2);
    assert_eq!(rule.policies.len(), 1);
}

// ==========================================
// Upgrade Test
// ==========================================

#[test]
fn test_upgrade_contract() {
    let e = Env::default();
    e.mock_all_auths();

    let signer = create_delegated_signer(&e);
    let policies: Map<Address, Val> = Map::new(&e);
    let client = create_client(&e, signer, policies);

    // Include the WASM bytes directly
    const WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zenex_account.wasm");

    // Upload the WASM to get its hash
    let wasm_hash = e.deployer().upload_contract_wasm(WASM);

    // The account contract uses its own address for auth during upgrade
    let contract_address = client.address.clone();

    // Call upgrade function directly via invoke_contract
    e.invoke_contract::<()>(
        &client.address,
        &soroban_sdk::Symbol::new(&e, "upgrade"),
        vec![&e, wasm_hash.to_val(), contract_address.to_val()],
    );
}
