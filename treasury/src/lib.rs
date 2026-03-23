#![no_std]

mod storage;

#[cfg(test)]
mod test;

use soroban_sdk::{contract, contracterror, contractclient, contractimpl, panic_with_error, token::TokenClient, Address, Env};
use soroban_fixed_point_math::SorobanFixedPoint;
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_macros::only_owner;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TreasuryError {
    InvalidRate = 900,
}

#[contract]
pub struct TreasuryContract;

const SCALAR_7: i128 = 10_000_000;

#[contractclient(name = "TreasuryClient")]
pub trait Treasury {
    /// Get the current protocol fee rate (SCALAR_7)
    fn get_rate(e: Env) -> i128;

    /// Calculate the protocol fee for a given total fee amount
    fn get_fee(e: Env, total_fee: i128) -> i128;

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

    fn get_fee(e: Env, total_fee: i128) -> i128 {
        storage::extend_instance(&e);
        let rate = storage::get_rate(&e);
        if rate > 0 && total_fee > 0 {
            total_fee.fixed_mul_floor(&e, &rate, &SCALAR_7)
        } else {
            0
        }
    }

    #[only_owner]
    fn set_rate(e: Env, rate: i128) {
        if rate < 0 || rate > SCALAR_7 / 2 {
            panic_with_error!(&e, TreasuryError::InvalidRate);
        }
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
