#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, panic_with_error, Address, Env, IntoVal, Symbol, Val,
    Vec,
};
use soroban_sdk::unwrap::UnwrapOptimized;
use stellar_access::ownable::{self, Ownable};
use stellar_contract_utils::upgradeable::UpgradeableInternal;
use stellar_macros::{only_owner, Upgradeable};

mod errors;
mod events;
mod storage;

pub use errors::GovernanceError;
pub use storage::QueuedCall;

/// Governance timelock for deferred admin operations. Config changes are queued
/// with a mandatory delay. set_status bypasses delay for emergency halts.
#[derive(Upgradeable)]
#[contract]
pub struct GovernanceContract;

#[contractclient(name = "GovernanceClient")]
pub trait Governance {
    /// (Owner only) Queue an arbitrary contract call to execute after the delay.
    ///
    /// # Parameters
    /// - `target` - Contract address to call
    /// - `fn_name` - Function name to invoke on the target
    /// - `args` - Arguments to pass (serialized as `Vec<Val>`)
    ///
    /// # Returns
    /// Nonce (monotonically increasing u32) identifying this queued call.
    ///
    /// # Panics
    /// - `GovernanceError::Unauthorized` (1) if caller is not the owner
    fn queue(e: Env, target: Address, fn_name: Symbol, args: Vec<Val>) -> u32;

    /// (Owner only) Cancel a queued call before it is executed.
    ///
    /// # Panics
    /// - `GovernanceError::Unauthorized` (1) if caller is not the owner
    /// - `GovernanceError::NotQueued` (601) if nonce not found
    fn cancel(e: Env, nonce: u32);

    /// (Permissionless) Execute a queued call after the delay has passed.
    ///
    /// # Panics
    /// - `GovernanceError::NotQueued` (601) if nonce not found
    /// - `GovernanceError::NotUnlocked` (602) if delay has not yet passed
    fn execute(e: Env, nonce: u32);

    /// (Owner only) Immediately call `set_status` on a target contract, bypassing the delay.
    ///
    /// # Parameters
    /// - `target` - Trading contract address
    /// - `status` - New status value (0=Active, 2=AdminOnIce, 3=Frozen)
    fn set_status(e: Env, target: Address, status: u32);

    /// (Owner only) Queue a delay update. The new delay takes effect after the
    /// CURRENT delay passes, preventing instant delay reduction attacks.
    fn set_delay(e: Env, new_delay: u64);

    /// (Permissionless) Apply a pending delay change after the current delay has passed.
    ///
    /// # Panics
    /// - `GovernanceError::NotQueued` (601) if no pending delay change
    /// - `GovernanceError::NotUnlocked` (602) if current delay has not yet passed
    fn apply_delay(e: Env);

    /// Returns the current delay in seconds.
    fn get_delay(e: Env) -> u64;

    /// Returns a queued call by nonce.
    ///
    /// # Panics
    /// - `GovernanceError::NotQueued` (601) if nonce not found or expired
    fn get_queued(e: Env, nonce: u32) -> QueuedCall;
}

#[contractimpl]
impl GovernanceContract {
    /// Initialize the governance contract with an owner and delay period.
    ///
    /// # Parameters
    /// - `owner` - Admin address authorized to queue/cancel calls and set status
    /// - `delay` - Mandatory waiting period in seconds before queued calls can execute
    pub fn __constructor(e: Env, owner: Address, delay: u64) {
        if delay == 0 {
            panic_with_error!(&e, GovernanceError::InvalidDelay);
        }
        ownable::set_owner(&e, &owner);
        storage::set_delay(&e, delay);
    }
}

#[contractimpl]
impl Governance for GovernanceContract {
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
            panic_with_error!(&e, GovernanceError::NotQueued);
        }
        storage::remove_queued(&e, nonce);
        events::Cancelled { nonce }.publish(&e);
    }

    fn execute(e: Env, nonce: u32) {
        let queued = storage::get_queued(&e, nonce)
            .unwrap_or_else(|| panic_with_error!(&e, GovernanceError::NotQueued));

        if queued.unlock_time > e.ledger().timestamp() {
            panic_with_error!(&e, GovernanceError::NotUnlocked);
        }

        // CEI: remove state before external call
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
        if new_delay == 0 {
            panic_with_error!(&e, GovernanceError::InvalidDelay);
        }
        let current_delay = storage::get_delay(&e);
        let unlock_time = e.ledger().timestamp() + current_delay;
        let pending = storage::PendingDelay {
            new_delay,
            unlock_time,
        };
        storage::set_pending_delay(&e, &pending);
        events::Queued {
            nonce: u32::MAX,
            target: e.current_contract_address(),
            fn_name: Symbol::new(&e, "set_delay"),
            unlock_time,
        }
        .publish(&e);
    }

    fn apply_delay(e: Env) {
        let pending = storage::get_pending_delay(&e)
            .unwrap_or_else(|| panic_with_error!(&e, GovernanceError::NotQueued));

        if pending.unlock_time > e.ledger().timestamp() {
            panic_with_error!(&e, GovernanceError::NotUnlocked);
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
            .unwrap_or_else(|| panic_with_error!(&e, GovernanceError::NotQueued))
    }
}

#[contractimpl(contracttrait)]
impl Ownable for GovernanceContract {}

impl UpgradeableInternal for GovernanceContract {
    fn _require_auth(e: &Env, operator: &Address) {
        operator.require_auth();
        let owner = ownable::get_owner(e).unwrap_optimized();
        if *operator != owner {
            panic_with_error!(e, GovernanceError::Unauthorized)
        }
    }
}

#[cfg(test)]
mod test;
