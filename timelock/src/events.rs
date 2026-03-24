use soroban_sdk::{contracttype, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Queued {
    pub nonce: u32,
    pub target: Address,
    pub fn_name: Symbol,
    pub unlock_time: u64,
}

impl Queued {
    #[allow(deprecated)]
    pub fn publish(&self, e: &Env) {
        e.events()
            .publish(("timelock", Symbol::new(e, "queued")), self.clone());
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Executed {
    pub nonce: u32,
    pub target: Address,
    pub fn_name: Symbol,
}

impl Executed {
    #[allow(deprecated)]
    pub fn publish(&self, e: &Env) {
        e.events()
            .publish(("timelock", Symbol::new(e, "executed")), self.clone());
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cancelled {
    pub nonce: u32,
}

impl Cancelled {
    #[allow(deprecated)]
    pub fn publish(&self, e: &Env) {
        e.events()
            .publish(("timelock", Symbol::new(e, "cancelled")), self.clone());
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusSet {
    pub target: Address,
    pub status: u32,
}

impl StatusSet {
    #[allow(deprecated)]
    pub fn publish(&self, e: &Env) {
        e.events()
            .publish(("timelock", Symbol::new(e, "status_set")), self.clone());
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelaySet {
    pub old_delay: u64,
    pub new_delay: u64,
}

impl DelaySet {
    #[allow(deprecated)]
    pub fn publish(&self, e: &Env) {
        e.events()
            .publish(("timelock", Symbol::new(e, "delay_set")), self.clone());
    }
}
