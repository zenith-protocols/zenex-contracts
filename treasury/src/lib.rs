#![no_std]

mod storage;

#[cfg(test)]
mod test;

use soroban_sdk::{contract, contractclient, contractimpl, token::TokenClient, Address, Env};
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_contract_utils::upgradeable::UpgradeableInternal;
use stellar_macros::{only_owner, Upgradeable};

#[derive(Upgradeable)]
#[contract]
pub struct TreasuryContract;

#[contractclient(name = "TreasuryClient")]
pub trait Treasury {
    /// Get the current protocol fee rate (SCALAR_7)
    fn get_rate(e: Env) -> i128;

    /// (Owner only) Set the protocol fee rate (SCALAR_7)
    fn set_rate(e: Env, rate: i128);

    /// (Owner only) Withdraw accumulated fees
    fn withdraw(e: Env, token: Address, to: Address, amount: i128);
}

#[contractimpl]
impl TreasuryContract {
    pub fn __constructor(e: Env, owner: Address, rate: i128) {
        ownable::set_owner(&e, &owner);
        storage::set_rate(&e, rate);
    }
}

#[contractimpl]
impl Treasury for TreasuryContract {
    fn get_rate(e: Env) -> i128 {
        storage::extend_instance(&e);
        storage::get_rate(&e)
    }

    #[only_owner]
    fn set_rate(e: Env, rate: i128) {
        storage::extend_instance(&e);
        storage::set_rate(&e, rate);
    }

    #[only_owner]
    fn withdraw(e: Env, token: Address, to: Address, amount: i128) {
        storage::extend_instance(&e);
        let token_client = TokenClient::new(&e, &token);
        token_client.transfer(&e.current_contract_address(), &to, &amount);
    }
}

#[contractimpl(contracttrait)]
impl Ownable for TreasuryContract {}

impl UpgradeableInternal for TreasuryContract {
    fn _require_auth(e: &Env, operator: &Address) {
        operator.require_auth();
        let owner = ownable::get_owner(e).expect("owner not set");
        if *operator != owner {
            panic!("unauthorized");
        }
    }
}
