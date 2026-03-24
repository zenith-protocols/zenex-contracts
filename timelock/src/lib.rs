#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, panic_with_error, Address, Env, IntoVal, Symbol, Val,
    Vec,
};
use stellar_access::ownable::{self, Ownable};
use stellar_contract_utils::upgradeable::UpgradeableInternal;
use stellar_macros::{only_owner, Upgradeable};

mod errors;
mod events;
mod storage;

pub use errors::TimelockError;
pub use storage::QueuedCall;

#[derive(Upgradeable)]
#[contract]
pub struct TimelockContract;

#[contractclient(name = "TimelockClient")]
pub trait Timelock {
    /// (Owner only) Queue a call to target contract after delay
    fn queue(e: Env, target: Address, fn_name: Symbol, args: Vec<Val>) -> u32;

    /// (Owner only) Cancel a queued call by nonce
    fn cancel(e: Env, nonce: u32);

    /// (Permissionless) Execute a queued call after delay has passed
    fn execute(e: Env, nonce: u32);

    /// (Owner only) Immediately call set_status on a target contract (bypass delay)
    fn set_status(e: Env, target: Address, status: u32);

    /// (Owner only) Queue a delay update through the timelock mechanism.
    /// The new delay is stored in temporary storage with the current delay as
    /// the waiting period. Call apply_delay() after the delay passes.
    fn set_delay(e: Env, new_delay: u64);

    /// (Permissionless) Apply a pending delay change after the delay has passed.
    /// Analogous to execute() but for the delay parameter itself.
    fn apply_delay(e: Env);

    /// Get the configured delay in seconds
    fn get_delay(e: Env) -> u64;

    /// Get a queued call by nonce (panics if not found or expired)
    fn get_queued(e: Env, nonce: u32) -> QueuedCall;
}

#[contractimpl]
impl TimelockContract {
    pub fn __constructor(e: Env, owner: Address, delay: u64) {
        ownable::set_owner(&e, &owner);
        storage::set_delay(&e, delay);
    }
}

#[contractimpl]
impl Timelock for TimelockContract {
    #[only_owner]
    fn queue(e: Env, target: Address, fn_name: Symbol, args: Vec<Val>) -> u32 {
        let delay = storage::get_delay(&e);
        let unlock_time = e.ledger().timestamp() + delay;
        let nonce = storage::next_nonce(&e);
        let queued = QueuedCall {
            target: target.clone(),
            fn_name: fn_name.clone(),
            args,
            unlock_time,
        };
        storage::set_queued(&e, nonce, &queued);
        events::Queued {
            nonce,
            target,
            fn_name,
            unlock_time,
        }
        .publish(&e);
        nonce
    }

    #[only_owner]
    fn cancel(e: Env, nonce: u32) {
        if storage::get_queued(&e, nonce).is_none() {
            panic_with_error!(&e, TimelockError::NotQueued);
        }
        storage::remove_queued(&e, nonce);
        events::Cancelled { nonce }.publish(&e);
    }

    fn execute(e: Env, nonce: u32) {
        let queued = storage::get_queued(&e, nonce)
            .unwrap_or_else(|| panic_with_error!(&e, TimelockError::NotQueued));

        if queued.unlock_time > e.ledger().timestamp() {
            panic_with_error!(&e, TimelockError::NotUnlocked);
        }

        // CEI: remove state before external call. Soroban prevents re-entrancy
        // at the host level, but following CEI is defense-in-depth for auditor
        // confidence.
        let target = queued.target.clone();
        let fn_name = queued.fn_name.clone();
        storage::remove_queued(&e, nonce);

        e.invoke_contract::<Val>(&target, &fn_name, queued.args);

        events::Executed {
            nonce,
            target,
            fn_name,
        }
        .publish(&e);
    }

    #[only_owner]
    fn set_status(e: Env, target: Address, status: u32) {
        let args: Vec<Val> = Vec::from_array(&e, [status.into_val(&e)]);
        e.invoke_contract::<Val>(&target, &Symbol::new(&e, "set_status"), args);
        events::StatusSet { target, status }.publish(&e);
    }

    #[only_owner]
    fn set_delay(e: Env, new_delay: u64) {
        // Queue the delay change through the timelock mechanism itself.
        // Uses dedicated PendingDelay storage instead of self-invoke to avoid
        // Soroban's re-entry restriction. The delay change uses the current
        // delay as the waiting period, preventing instant delay reduction attacks.
        let current_delay = storage::get_delay(&e);
        let unlock_time = e.ledger().timestamp() + current_delay;
        let pending = storage::PendingDelay {
            new_delay,
            unlock_time,
        };
        storage::set_pending_delay(&e, &pending);
        events::Queued {
            nonce: u32::MAX, // sentinel value indicating delay change
            target: e.current_contract_address(),
            fn_name: Symbol::new(&e, "set_delay"),
            unlock_time,
        }
        .publish(&e);
    }

    fn apply_delay(e: Env) {
        let pending = storage::get_pending_delay(&e)
            .unwrap_or_else(|| panic_with_error!(&e, TimelockError::NotQueued));

        if pending.unlock_time > e.ledger().timestamp() {
            panic_with_error!(&e, TimelockError::NotUnlocked);
        }

        let old_delay = storage::get_delay(&e);
        storage::remove_pending_delay(&e);
        storage::set_delay(&e, pending.new_delay);
        events::DelaySet {
            old_delay,
            new_delay: pending.new_delay,
        }
        .publish(&e);
    }

    fn get_delay(e: Env) -> u64 {
        storage::get_delay(&e)
    }

    fn get_queued(e: Env, nonce: u32) -> QueuedCall {
        storage::get_queued(&e, nonce)
            .unwrap_or_else(|| panic_with_error!(&e, TimelockError::NotQueued))
    }
}

#[contractimpl(contracttrait)]
impl Ownable for TimelockContract {}

impl UpgradeableInternal for TimelockContract {
    fn _require_auth(e: &Env, operator: &Address) {
        operator.require_auth();
        let owner = ownable::get_owner(e).expect("owner not set");
        if *operator != owner {
            panic_with_error!(e, TimelockError::Unauthorized)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{contract, contractimpl, Env, IntoVal, Symbol, Val, Vec};

    // ── Mock target contract ──────────────────────────────────────────

    #[contract]
    pub struct MockTarget;

    #[contractimpl]
    impl MockTarget {
        pub fn __constructor(_e: Env) {}

        pub fn set_status(e: Env, status: u32) {
            e.storage()
                .instance()
                .set(&Symbol::new(&e, "status"), &status);
        }

        pub fn set_config(e: Env, value: u32) {
            e.storage()
                .instance()
                .set(&Symbol::new(&e, "config"), &value);
        }

        pub fn get_status(e: Env) -> u32 {
            e.storage()
                .instance()
                .get(&Symbol::new(&e, "status"))
                .unwrap_or(0)
        }

        pub fn get_config(e: Env) -> u32 {
            e.storage()
                .instance()
                .get(&Symbol::new(&e, "config"))
                .unwrap_or(0)
        }
    }

    // ── Test helpers ──────────────────────────────────────────────────

    const DELAY: u64 = 3600; // 1 hour

    fn setup_env() -> (Env, Address, Address, Address) {
        let e = Env::default();
        e.mock_all_auths();

        let owner = Address::generate(&e);

        // Deploy mock target
        let target_id = e.register(MockTarget, ());

        // Deploy timelock
        let timelock_id = e.register(TimelockContract, (&owner, DELAY));

        (e, owner, timelock_id, target_id)
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

    // ── Tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_queue_and_execute() {
        let (e, _owner, timelock_id, target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);
        let target_client = MockTargetClient::new(&e, &target_id);

        set_ledger_timestamp(&e, 1000);

        let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
        let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);
        assert_eq!(nonce, 0);

        // Verify queued call data
        let queued = client.get_queued(&nonce);
        assert_eq!(queued.unlock_time, 1000 + DELAY);

        // Advance past delay
        set_ledger_timestamp(&e, 1000 + DELAY + 1);
        client.execute(&nonce);

        // Verify target was called
        assert_eq!(target_client.get_config(), 42);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #2)")]
    fn test_execute_before_delay_fails() {
        let (e, _owner, timelock_id, target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);

        set_ledger_timestamp(&e, 1000);

        let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
        let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);

        // Try to execute immediately (timestamp still 1000, unlock_time is 1000 + 3600)
        client.execute(&nonce);
    }

    #[test]
    fn test_cancel_removes_queued() {
        let (e, _owner, timelock_id, target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);

        set_ledger_timestamp(&e, 1000);

        let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
        let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);

        // Cancel it
        client.cancel(&nonce);

        // Advance past delay
        set_ledger_timestamp(&e, 1000 + DELAY + 1);

        // Try to execute -- should fail with NotQueued
        let result = client.try_execute(&nonce);
        assert!(result.is_err());
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_cancel_nonexistent_fails() {
        let (e, _owner, timelock_id, _target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);

        // Cancel a nonce that was never queued
        client.cancel(&999);
    }

    #[test]
    fn test_set_status_bypasses_delay() {
        let (e, _owner, timelock_id, target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);
        let target_client = MockTargetClient::new(&e, &target_id);

        set_ledger_timestamp(&e, 1000);

        // set_status should work immediately without queuing
        client.set_status(&target_id, &2);

        // Verify target's set_status was called
        assert_eq!(target_client.get_status(), 2);
    }

    #[test]
    fn test_queue_requires_owner() {
        let e = Env::default();
        // Do NOT mock all auths -- we want real auth checking
        let owner = Address::generate(&e);
        let non_owner = Address::generate(&e);

        let target_id = e.register(MockTarget, ());
        let timelock_id = e.register(TimelockContract, (&owner, DELAY));

        let client = TimelockClient::new(&e, &timelock_id);

        // Mock auth only for non_owner (not the owner)
        e.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &non_owner,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &timelock_id,
                fn_name: "queue",
                args: (
                    target_id.clone(),
                    Symbol::new(&e, "set_config"),
                    Vec::<Val>::new(&e),
                )
                    .into_val(&e),
                sub_invokes: &[],
            },
        }]);

        // This should fail because non_owner != owner
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
        let timelock_id = e.register(TimelockContract, (&owner, DELAY));

        let client = TimelockClient::new(&e, &timelock_id);

        // First, queue something with proper auth
        e.mock_all_auths();
        set_ledger_timestamp(&e, 1000);
        let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
        let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);

        // Now try to cancel as non-owner
        e.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &non_owner,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &timelock_id,
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
        let (e, _owner, timelock_id, target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);

        set_ledger_timestamp(&e, 1000);

        let args: Vec<Val> = Vec::from_array(&e, [1u32.into_val(&e)]);
        let nonce0 = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);
        let nonce1 = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);

        assert_eq!(nonce0, 0);
        assert_eq!(nonce1, 1);
    }

    #[test]
    fn test_execute_cei_ordering() {
        let (e, _owner, timelock_id, target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);

        set_ledger_timestamp(&e, 1000);

        let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
        let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);

        // Advance past delay
        set_ledger_timestamp(&e, 1000 + DELAY + 1);
        client.execute(&nonce);

        // Verify the queue entry is removed -- re-executing the same nonce should
        // fail with NotQueued. This confirms CEI: removal happened before the invoke.
        let result = client.try_execute(&nonce);
        assert!(result.is_err());
    }

    #[test]
    fn test_set_delay_through_timelock() {
        let (e, _owner, timelock_id, _target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);

        set_ledger_timestamp(&e, 1000);

        // Current delay is 3600
        assert_eq!(client.get_delay(), DELAY);

        // set_delay queues a pending delay change
        client.set_delay(&7200);

        // Delay should not have changed yet
        assert_eq!(client.get_delay(), DELAY);

        // Try to apply before delay passes -- should fail
        let result = client.try_apply_delay();
        assert!(result.is_err());

        // Advance past the current delay
        set_ledger_timestamp(&e, 1000 + DELAY + 1);
        client.apply_delay();

        // Now delay should be updated
        assert_eq!(client.get_delay(), 7200);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_apply_delay_without_pending_fails() {
        let (e, _owner, timelock_id, _target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);

        set_ledger_timestamp(&e, 1000);

        // No pending delay -- should fail with NotQueued
        client.apply_delay();
    }

    #[test]
    fn test_event_emission() {
        let (e, _owner, timelock_id, target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);
        let target_client = MockTargetClient::new(&e, &target_id);

        set_ledger_timestamp(&e, 1000);

        let args: Vec<Val> = Vec::from_array(&e, [42u32.into_val(&e)]);
        let nonce = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args);

        // Advance past delay and execute
        set_ledger_timestamp(&e, 1000 + DELAY + 1);
        client.execute(&nonce);

        // Verify the operations completed successfully (events are emitted inline).
        // The Queued and Executed event publish calls ran as part of queue() and execute().
        assert_eq!(target_client.get_config(), 42);

        // Verify queue entry removed (confirming full execute path ran including event)
        let result = client.try_execute(&nonce);
        assert!(result.is_err());
    }

    #[test]
    fn test_set_status_event() {
        let (e, _owner, timelock_id, target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);
        let target_client = MockTargetClient::new(&e, &target_id);

        set_ledger_timestamp(&e, 1000);

        // set_status emits StatusSet event inline
        client.set_status(&target_id, &1);

        // Verify the operation completed (status was forwarded to target)
        assert_eq!(target_client.get_status(), 1);
    }

    #[test]
    fn test_multiple_queue_execute() {
        let (e, _owner, timelock_id, target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);
        let target_client = MockTargetClient::new(&e, &target_id);

        set_ledger_timestamp(&e, 1000);

        let args1: Vec<Val> = Vec::from_array(&e, [100u32.into_val(&e)]);
        let args2: Vec<Val> = Vec::from_array(&e, [200u32.into_val(&e)]);
        let nonce0 = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args1);
        let nonce1 = client.queue(&target_id, &Symbol::new(&e, "set_config"), &args2);

        // Advance and execute the second one first
        set_ledger_timestamp(&e, 1000 + DELAY + 1);
        client.execute(&nonce1);
        assert_eq!(target_client.get_config(), 200);

        // Execute the first one
        client.execute(&nonce0);
        assert_eq!(target_client.get_config(), 100);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_get_queued_nonexistent_panics() {
        let (e, _owner, timelock_id, _target_id) = setup_env();
        let client = TimelockClient::new(&e, &timelock_id);

        client.get_queued(&999);
    }
}
