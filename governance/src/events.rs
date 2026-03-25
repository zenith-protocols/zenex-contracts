use soroban_sdk::{contractevent, Address, Symbol};

#[contractevent]
#[derive(Clone)]
pub struct Queued {
    #[topic]
    pub nonce: u32,
    pub target: Address,
    pub fn_name: Symbol,
    pub unlock_time: u64,
}

#[contractevent]
#[derive(Clone)]
pub struct Executed {
    #[topic]
    pub nonce: u32,
    pub target: Address,
    pub fn_name: Symbol,
}

#[contractevent]
#[derive(Clone)]
pub struct Cancelled {
    #[topic]
    pub nonce: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct StatusSet {
    #[topic]
    pub target: Address,
    pub status: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct DelaySet {
    pub old_delay: u64,
    pub new_delay: u64,
}
