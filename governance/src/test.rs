use crate::{GovernanceContract, GovernanceClient, GovernanceError};
use crate::storage::QueuedCall;
use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{contract, contractimpl, Address, Env, IntoVal, Symbol, Val, Vec};

#[contract]
pub struct MockTarget;

#[contractimpl]
impl MockTarget {
    pub fn __constructor(_e: Env) {}

    pub fn set_status(e: Env, status: u32) {
        e.storage().instance().set(&Symbol::new(&e, "status"), &status);
    }

    pub fn set_config(e: Env, value: u32) {
        e.storage().instance().set(&Symbol::new(&e, "config"), &value);
    }

    pub fn get_status(e: Env) -> u32 {
        e.storage().instance().get(&Symbol::new(&e, "status")).unwrap_or(0)
    }

    pub fn get_config(e: Env) -> u32 {
        e.storage().instance().get(&Symbol::new(&e, "config")).unwrap_or(0)
    }
}

const DELAY: u64 = 3600;

fn setup_env() -> (Env, Address, Address, Address) {
    let e = Env::default();
    e.mock_all_auths();
    let owner = Address::generate(&e);
    let target_id = e.register(MockTarget, ());
    let gov_id = e.register(GovernanceContract, (&owner, DELAY));
    (e, owner, gov_id, target_id)
}

fn set_ledger_timestamp(e: &Env, timestamp: u64) {
    e.ledger().set(LedgerInfo {
        timestamp,
        protocol_version: 25,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 100_000,
        min_persistent_entry_ttl: 100_000,
        max_entry_ttl: 10_000_000,
    });
}

#[test]
fn test_queue_and_execute() {
    let (e, _owner, gov_id, target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);
    let target_client = MockTargetClient::new(&e, &target_id);

    set_ledger_timestamp(&e, 1000);
    let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
    let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);
    assert_eq!(nonce, 0);

    let queued = client.get_queued(&nonce);
    assert_eq!(queued.unlock_time, 1000 + DELAY);

    set_ledger_timestamp(&e, 1000 + DELAY + 1);
    client.execute(&nonce);
    assert_eq!(target_client.get_config(), 42);
}

#[test]
#[should_panic(expected = "Error(Contract, #771)")]
fn test_execute_before_delay_fails() {
    let (e, _owner, gov_id, target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);

    set_ledger_timestamp(&e, 1000);
    let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
    let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);
    client.execute(&nonce);
}

#[test]
fn test_cancel_removes_queued() {
    let (e, _owner, gov_id, target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);

    set_ledger_timestamp(&e, 1000);
    let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
    let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);
    client.cancel(&nonce);

    set_ledger_timestamp(&e, 1000 + DELAY + 1);
    let result = client.try_execute(&nonce);
    assert!(result.is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #770)")]
fn test_cancel_nonexistent_fails() {
    let (e, _owner, gov_id, _target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);
    client.cancel(&999);
}

#[test]
fn test_set_status_bypasses_delay() {
    let (e, _owner, gov_id, target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);
    let target_client = MockTargetClient::new(&e, &target_id);

    set_ledger_timestamp(&e, 1000);
    client.set_status(&target_id, &2);
    assert_eq!(target_client.get_status(), 2);
}

#[test]
fn test_queue_requires_owner() {
    let e = Env::default();
    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let target_id = e.register(MockTarget, ());
    let gov_id = e.register(GovernanceContract, (&owner, DELAY));
    let client = GovernanceClient::new(&e, &gov_id);

    e.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_owner,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &gov_id,
            fn_name: "queue",
            args: (
                target_id.clone(),
                Symbol::new(&e, "set_config"),
                Vec::<Val>::new(&e),
            ).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_queue(
        &target_id,
        &Symbol::new(&e, "set_config"),
        &Vec::<Val>::new(&e),
    );
    assert!(result.is_err());
}

#[test]
fn test_cancel_requires_owner() {
    let e = Env::default();
    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let target_id = e.register(MockTarget, ());
    let gov_id = e.register(GovernanceContract, (&owner, DELAY));
    let client = GovernanceClient::new(&e, &gov_id);

    e.mock_all_auths();
    set_ledger_timestamp(&e, 1000);
    let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
    let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);

    e.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_owner,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &gov_id,
            fn_name: "cancel",
            args: (nonce,).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_cancel(&nonce);
    assert!(result.is_err());
}

#[test]
fn test_nonce_increments() {
    let (e, _owner, gov_id, target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);

    set_ledger_timestamp(&e, 1000);
    let args: Vec<Val> = Vec::from_array(&e, [1u32.into_val(&e)]);
    let nonce0 = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);
    let nonce1 = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);
    assert_eq!(nonce0, 0);
    assert_eq!(nonce1, 1);
}

#[test]
fn test_execute_cei_ordering() {
    let (e, _owner, gov_id, target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);

    set_ledger_timestamp(&e, 1000);
    let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
    let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);

    set_ledger_timestamp(&e, 1000 + DELAY + 1);
    client.execute(&nonce);

    let result = client.try_execute(&nonce);
    assert!(result.is_err());
}

#[test]
fn test_set_delay_through_timelock() {
    let (e, _owner, gov_id, _target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);

    set_ledger_timestamp(&e, 1000);
    assert_eq!(client.get_delay(), DELAY);

    client.set_delay(&7200);
    assert_eq!(client.get_delay(), DELAY);

    let result = client.try_apply_delay();
    assert!(result.is_err());

    set_ledger_timestamp(&e, 1000 + DELAY + 1);
    client.apply_delay();
    assert_eq!(client.get_delay(), 7200);
}

#[test]
#[should_panic(expected = "Error(Contract, #770)")]
fn test_apply_delay_without_pending_fails() {
    let (e, _owner, gov_id, _target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);

    set_ledger_timestamp(&e, 1000);
    client.apply_delay();
}

#[test]
fn test_event_emission() {
    let (e, _owner, gov_id, target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);
    let target_client = MockTargetClient::new(&e, &target_id);

    set_ledger_timestamp(&e, 1000);
    let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
    let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);

    set_ledger_timestamp(&e, 1000 + DELAY + 1);
    client.execute(&nonce);

    assert_eq!(target_client.get_config(), 42);
    let result = client.try_execute(&nonce);
    assert!(result.is_err());
}

#[test]
fn test_set_status_event() {
    let (e, _owner, gov_id, target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);
    let target_client = MockTargetClient::new(&e, &target_id);

    set_ledger_timestamp(&e, 1000);
    client.set_status(&target_id, &1);
    assert_eq!(target_client.get_status(), 1);
}

#[test]
fn test_multiple_queue_execute() {
    let (e, _owner, gov_id, target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);
    let target_client = MockTargetClient::new(&e, &target_id);

    set_ledger_timestamp(&e, 1000);
    let args1: Vec<Val> = Vec::from_array(&e, [100u32.into_val(&e)]);
    let args2: Vec<Val> = Vec::from_array(&e, [200u32.into_val(&e)]);
    let nonce0 = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args1);
    let nonce1 = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args2);

    set_ledger_timestamp(&e, 1000 + DELAY + 1);
    client.execute(&nonce1);
    assert_eq!(target_client.get_config(), 200);

    client.execute(&nonce0);
    assert_eq!(target_client.get_config(), 100);
}

#[test]
#[should_panic(expected = "Error(Contract, #770)")]
fn test_get_queued_nonexistent_panics() {
    let (e, _owner, gov_id, _target_id) = setup_env();
    let client = GovernanceClient::new(&e, &gov_id);
    client.get_queued(&999);
}
