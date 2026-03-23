use soroban_sdk::{contractclient, Env};

#[contractclient(name = "Client")]
pub trait TreasuryInterface {
    fn get_rate(e: Env) -> i128;

    fn get_fee(e: Env, total_fee: i128) -> i128;
}
