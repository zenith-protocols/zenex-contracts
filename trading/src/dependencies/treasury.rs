use soroban_sdk::{contractclient, Env};

/// Treasury contract interface (used via TreasuryClient in market.rs).
#[allow(dead_code)]
#[contractclient(name = "Client")]
pub trait TreasuryInterface {
    fn get_rate(e: Env) -> i128;
}
