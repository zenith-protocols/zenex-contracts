#![no_std]

mod storage;

#[cfg(test)]
mod test;

use soroban_sdk::{contract, contracterror, contractclient, contractimpl, panic_with_error, token::TokenClient, Address, Env};
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_macros::only_owner;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TreasuryError {
    /// Rate must be in range [0, SCALAR_7/2] (0% to 50%).
    InvalidRate = 900,
}

/// Protocol fee rate storage and withdrawal. Trading computes fees inline using get_rate().
#[contract]
pub struct TreasuryContract;

const SCALAR_7: i128 = 10_000_000;

#[contractclient(name = "TreasuryClient")]
pub trait Treasury {
    /// Returns the current protocol fee rate (SCALAR_7 fraction, e.g. 1e6 = 10%).
    fn get_rate(e: Env) -> i128;

    /// (Owner only) Set the protocol fee rate.
    ///
    /// # Parameters
    /// - `rate` - New fee rate (SCALAR_7 fraction, e.g. 1e6 = 10%). Bounded to [0, SCALAR_7/2].
    ///
    /// # Panics
    /// - `TreasuryError::InvalidRate` (900) if rate is outside [0, SCALAR_7/2]
    fn set_rate(e: Env, rate: i128);

    /// (Owner only) Withdraw accumulated protocol fees.
    ///
    /// # Parameters
    /// - `token` - Token contract address to withdraw
    /// - `to` - Recipient address
    /// - `amount` - Amount to withdraw (token_decimals)
    fn withdraw(e: Env, token: Address, to: Address, amount: i128);
}

#[contractimpl]
impl TreasuryContract {
    /// Initialize the treasury with an owner and fee rate.
    ///
    /// # Parameters
    /// - `owner` - Admin address for rate changes and withdrawals
    /// - `rate` - Protocol fee rate (SCALAR_7 fraction)
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
        if rate < 0 || rate > SCALAR_7 / 2 {
            panic_with_error!(&e, TreasuryError::InvalidRate);
        }
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
