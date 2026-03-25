use soroban_sdk::{contractevent, Address, Symbol};

/// Emitted when a new call is queued via `queue` or when `set_delay` creates a pending change.
#[contractevent]
#[derive(Clone)]
pub struct Queued {
    #[topic]
    pub nonce: u32,
    pub target: Address,
    pub fn_name: Symbol,
    pub unlock_time: u64,
}

/// Emitted when a queued call is executed after the delay period.
#[contractevent]
#[derive(Clone)]
pub struct Executed {
    #[topic]
    pub nonce: u32,
    pub target: Address,
    pub fn_name: Symbol,
}

/// Emitted when a queued call is cancelled by the owner.
#[contractevent]
#[derive(Clone)]
pub struct Cancelled {
    #[topic]
    pub nonce: u32,
}

/// Emitted when `set_status` is called (immediate, no delay).
#[contractevent]
#[derive(Clone)]
pub struct StatusSet {
    #[topic]
    pub target: Address,
    pub status: u32,
}

/// Emitted when a pending delay change is applied via `apply_delay`.
#[contractevent]
#[derive(Clone)]
pub struct DelaySet {
    pub old_delay: u64,
    pub new_delay: u64,
}
