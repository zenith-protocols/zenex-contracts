use soroban_sdk::{contractevent, Address};

#[contractevent]
#[derive(Clone)]
pub struct Deploy {
    #[topic]
    pub trading: Address,
    #[topic]
    pub vault: Address,
}
