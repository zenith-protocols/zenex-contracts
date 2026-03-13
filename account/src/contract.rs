//! # ZenexAccount - Smart Account for Zenex
//!
//! A smart account that supports multiple signers with context-based authorization.
//! Uses OpenZeppelin's stellar-accounts implementation. This contract is upgradeable.
//!
//! Context rules are the core authorization primitive: each rule binds a set of signers
//! and policies to a specific context type (`Default`, `CallContract`, or `CreateContract`).
//! During `__check_auth`, matching rules are evaluated to authorize the transaction.
use soroban_sdk::{
    auth::{Context, CustomAccountInterface},
    contract, contractimpl,
    crypto::Hash,
    Address, Env, Map, String, Symbol, Val, Vec,
};
use stellar_accounts::smart_account::{
    add_context_rule, add_policy, add_signer, do_check_auth, get_context_rule, get_context_rules,
    get_context_rules_count, remove_context_rule, remove_policy, remove_signer,
    update_context_rule_name, update_context_rule_valid_until, ContextRule, ContextRuleType,
    ExecutionEntryPoint, Signatures, Signer, SmartAccount, SmartAccountError,
};
use stellar_contract_utils::upgradeable::UpgradeableInternal;
use stellar_macros::Upgradeable;

#[derive(Upgradeable)]
#[contract]
pub struct ZenexAccount;

#[contractimpl]
impl ZenexAccount {
    /// Initializes the account by creating a `Default` context rule named "primary"
    /// with the given signers and policies. This rule has no expiration.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `signers` - Initial signers for the account. Each signer is either
    ///   `Signer::Delegated(Address)` for built-in ed25519 verification or
    ///   `Signer::External(Address, Bytes)` for custom verifier contracts.
    /// * `policies` - Map of policy contract addresses to their installation parameters.
    ///   Pass an empty map if no policies are needed.
    ///
    /// # Panics
    ///
    /// * `NoSignersAndPolicies` (3004) - If both `signers` and `policies` are empty.
    /// * `TooManySigners` (3010) - If `signers` length exceeds 15.
    /// * `TooManyPolicies` (3011) - If `policies` length exceeds 5.
    /// * `DuplicateSigner` (3007) - If `signers` contains duplicates.
    pub fn __constructor(e: Env, signers: Vec<Signer>, policies: Map<Address, Val>) {
        add_context_rule(
            &e,
            &ContextRuleType::Default,
            &String::from_str(&e, "primary"),
            None,
            &signers,
            &policies,
        );
    }
}

#[contractimpl]
impl CustomAccountInterface for ZenexAccount {
    type Signature = Signatures;
    type Error = SmartAccountError;

    /// Validates transaction signatures against the account's context rules.
    /// Called automatically by the Soroban runtime during authorization.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `signature_payload` - The 32-byte hash of the transaction payload to verify.
    /// * `signatures` - A `Signatures` map of `Signer` to `Bytes` containing the
    ///   signature data for each signer.
    /// * `auth_contexts` - The authorization contexts being requested, each matched
    ///   against stored context rules.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If all auth contexts are satisfied by matching context rules.
    /// * `Err(SmartAccountError)` - If verification fails.
    ///
    /// # Errors
    ///
    /// * `UnvalidatedContext` (3002) - If an auth context cannot be matched to any rule.
    /// * `ExternalVerificationFailed` (3003) - If an external signer's verifier
    ///   contract rejects the signature.
    fn __check_auth(
        e: Env,
        signature_payload: Hash<32>,
        signatures: Signatures,
        auth_contexts: Vec<Context>,
    ) -> Result<(), Self::Error> {
        do_check_auth(&e, &signature_payload, &signatures, &auth_contexts)
    }
}

#[contractimpl]
impl SmartAccount for ZenexAccount {
    /// Retrieves a context rule by its unique ID.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `context_rule_id` - The unique numeric identifier of the context rule.
    ///
    /// # Returns
    ///
    /// A `ContextRule` containing the rule's `id`, `context_type`, `name`,
    /// `signers`, `policies`, and `valid_until`.
    ///
    /// # Panics
    ///
    /// * `ContextRuleNotFound` (3000) - If no rule exists with the given ID.
    fn get_context_rule(e: &Env, context_rule_id: u32) -> ContextRule {
        get_context_rule(e, context_rule_id)
    }

    /// Retrieves all context rules matching a specific type.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `context_rule_type` - The type to filter by: `Default`,
    ///   `CallContract(Address)`, or `CreateContract(BytesN<32>)`.
    ///
    /// # Returns
    ///
    /// A `Vec<ContextRule>` of all matching rules. Returns an empty vector if
    /// no rules of the given type exist.
    fn get_context_rules(e: &Env, context_rule_type: ContextRuleType) -> Vec<ContextRule> {
        get_context_rules(e, &context_rule_type)
    }

    /// Returns the total number of context rules, including expired ones.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    ///
    /// # Returns
    ///
    /// The count as `u32`. Defaults to 0 if no rules have been created.
    fn get_context_rules_count(e: &Env) -> u32 {
        get_context_rules_count(e)
    }

    /// Creates a new context rule with the specified configuration.
    /// Requires account authorization.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `context_type` - The type of context this rule authorizes: `Default`,
    ///   `CallContract(Address)`, or `CreateContract(BytesN<32>)`.
    /// * `name` - A human-readable name for the rule.
    /// * `valid_until` - Optional ledger sequence number after which the rule
    ///   expires. `None` means no expiration.
    /// * `signers` - Signers authorized by this rule.
    /// * `policies` - Map of policy contract addresses to their install parameters.
    ///
    /// # Returns
    ///
    /// The newly created `ContextRule` with a unique auto-incremented `id`.
    ///
    /// # Panics
    ///
    /// * `TooManyContextRules` (3012) - If total rules would exceed 15.
    /// * `NoSignersAndPolicies` (3004) - If both `signers` and `policies` are empty.
    /// * `TooManySigners` (3010) - If `signers` length exceeds 15.
    /// * `TooManyPolicies` (3011) - If `policies` length exceeds 5.
    /// * `DuplicateSigner` (3007) - If `signers` contains duplicates.
    /// * `PastValidUntil` (3005) - If `valid_until` is in the past.
    fn add_context_rule(
        e: &Env,
        context_type: ContextRuleType,
        name: String,
        valid_until: Option<u32>,
        signers: Vec<Signer>,
        policies: Map<Address, Val>,
    ) -> ContextRule {
        e.current_contract_address().require_auth();
        add_context_rule(e, &context_type, &name, valid_until, &signers, &policies)
    }

    /// Updates the name of an existing context rule.
    /// Requires account authorization.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `context_rule_id` - The ID of the context rule to update.
    /// * `name` - The new human-readable name.
    ///
    /// # Returns
    ///
    /// The updated `ContextRule` with the new name.
    ///
    /// # Panics
    ///
    /// * `ContextRuleNotFound` (3000) - If no rule exists with the given ID.
    fn update_context_rule_name(e: &Env, context_rule_id: u32, name: String) -> ContextRule {
        e.current_contract_address().require_auth();
        update_context_rule_name(e, context_rule_id, &name)
    }

    /// Updates the expiration ledger sequence of an existing context rule.
    /// Requires account authorization.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `context_rule_id` - The ID of the context rule to update.
    /// * `valid_until` - New expiration ledger sequence, or `None` to remove
    ///   expiration.
    ///
    /// # Returns
    ///
    /// The updated `ContextRule` with the new expiration.
    ///
    /// # Panics
    ///
    /// * `ContextRuleNotFound` (3000) - If no rule exists with the given ID.
    /// * `PastValidUntil` (3005) - If `valid_until` is in the past.
    fn update_context_rule_valid_until(
        e: &Env,
        context_rule_id: u32,
        valid_until: Option<u32>,
    ) -> ContextRule {
        e.current_contract_address().require_auth();
        update_context_rule_valid_until(e, context_rule_id, valid_until)
    }

    /// Removes a context rule and uninstalls all its associated policies.
    /// Requires account authorization.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `context_rule_id` - The ID of the context rule to remove.
    ///
    /// # Panics
    ///
    /// * `ContextRuleNotFound` (3000) - If no rule exists with the given ID.
    fn remove_context_rule(e: &Env, context_rule_id: u32) {
        e.current_contract_address().require_auth();
        remove_context_rule(e, context_rule_id);
    }

    /// Adds a signer to an existing context rule.
    /// Requires account authorization.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `context_rule_id` - The ID of the context rule to add the signer to.
    /// * `signer` - The signer to add: `Signer::Delegated(Address)` or
    ///   `Signer::External(Address, Bytes)`.
    ///
    /// # Panics
    ///
    /// * `ContextRuleNotFound` (3000) - If no rule exists with the given ID.
    /// * `DuplicateSigner` (3007) - If the signer already exists in the rule.
    /// * `TooManySigners` (3010) - If adding would exceed 15 signers.
    fn add_signer(e: &Env, context_rule_id: u32, signer: Signer) {
        e.current_contract_address().require_auth();
        add_signer(e, context_rule_id, &signer);
    }

    /// Removes a signer from an existing context rule. The last signer can only
    /// be removed if the rule still has at least one policy.
    /// Requires account authorization.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `context_rule_id` - The ID of the context rule to remove the signer from.
    /// * `signer` - The signer to remove.
    ///
    /// # Panics
    ///
    /// * `ContextRuleNotFound` (3000) - If no rule exists with the given ID.
    /// * `SignerNotFound` (3006) - If the signer doesn't exist in the rule.
    fn remove_signer(e: &Env, context_rule_id: u32, signer: Signer) {
        e.current_contract_address().require_auth();
        remove_signer(e, context_rule_id, &signer);
    }

    /// Adds a policy contract to an existing context rule and calls the policy's
    /// `install` method. Requires account authorization.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `context_rule_id` - The ID of the context rule to add the policy to.
    /// * `policy` - The address of the policy contract to add.
    /// * `install_param` - Parameter passed to the policy's `install` method
    ///   during installation.
    ///
    /// # Panics
    ///
    /// * `ContextRuleNotFound` (3000) - If no rule exists with the given ID.
    /// * `DuplicatePolicy` (3009) - If the policy already exists in the rule.
    /// * `TooManyPolicies` (3011) - If adding would exceed 5 policies.
    fn add_policy(e: &Env, context_rule_id: u32, policy: Address, install_param: Val) {
        e.current_contract_address().require_auth();
        add_policy(e, context_rule_id, &policy, install_param);
    }

    /// Removes a policy contract from an existing context rule and calls the
    /// policy's `uninstall` method. The last policy can only be removed if the
    /// rule still has at least one signer. Requires account authorization.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `context_rule_id` - The ID of the context rule to remove the policy from.
    /// * `policy` - The address of the policy contract to remove.
    ///
    /// # Panics
    ///
    /// * `ContextRuleNotFound` (3000) - If no rule exists with the given ID.
    /// * `PolicyNotFound` (3008) - If the policy doesn't exist in the rule.
    fn remove_policy(e: &Env, context_rule_id: u32, policy: Address) {
        e.current_contract_address().require_auth();
        remove_policy(e, context_rule_id, &policy);
    }
}

#[contractimpl]
impl ExecutionEntryPoint for ZenexAccount {
    /// Executes an arbitrary contract call from within this smart account.
    /// Requires account authorization.
    ///
    /// This enables the account to interact with external contracts (e.g. DeFi
    /// protocols, policy contracts) while maintaining proper authorization flow.
    ///
    /// # Arguments
    ///
    /// * `e` - The Soroban environment.
    /// * `target` - The address of the contract to call.
    /// * `target_fn` - The function name to invoke on the target contract.
    /// * `target_args` - Arguments to pass to the target function.
    fn execute(e: &Env, target: Address, target_fn: Symbol, target_args: Vec<Val>) {
        e.current_contract_address().require_auth();
        e.invoke_contract::<Val>(&target, &target_fn, target_args);
    }
}

impl UpgradeableInternal for ZenexAccount {
    /// Requires the contract's own authorization for upgrades.
    fn _require_auth(e: &Env, _operator: &Address) {
        e.current_contract_address().require_auth();
    }
}
