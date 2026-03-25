#![no_std]

mod storage;

#[cfg(test)]
mod test;

use soroban_sdk::{contract, contracterror, contractclient, contractimpl, panic_with_error, token::TokenClient, Address, Env};
use soroban_fixed_point_math::SorobanFixedPoint;
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_macros::only_owner;

/// Rate out of valid bounds (must be 0..=50%).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TreasuryError {
    /// Rate must be in range [0, SCALAR_7/2] (0% to 50%).
    InvalidRate = 900,
}

/// Protocol fee collection and distribution contract.
///
/// The trading contract calls `get_fee(revenue)` to compute the treasury's cut,
/// then transfers that amount to this contract's address. The owner can withdraw
/// accumulated fees at any time.
///
/// See: Protocol Spec -- `docs/audit/PROTOCOL-SPEC.md`
#[contract]
pub struct TreasuryContract;

const SCALAR_7: i128 = 10_000_000;

#[contractclient(name = "TreasuryClient")]
pub trait Treasury {
    /// Returns the current protocol fee rate (SCALAR_7 fraction, e.g. 1e6 = 10%).
    fn get_rate(e: Env) -> i128;

    /// Calculate the protocol fee for a given revenue amount.
    ///
    /// `fee = total_fee * rate / SCALAR_7` using floor rounding.
    /// WHY: Floor rounding is conservative for the protocol -- the treasury never
    /// over-claims from trading fees, ensuring the vault/user split is fair.
    ///
    /// Returns 0 if rate or total_fee is <= 0.
    fn get_fee(e: Env, total_fee: i128) -> i128;

    /// (Owner only) Set the protocol fee rate (SCALAR_7 fraction).
    ///
    /// Bounded to [0, SCALAR_7/2] = [0%, 50%].
    ///
    /// # Panics
    /// - `TreasuryError::InvalidRate` (900) if rate is outside bounds
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
    /// - `rate` - Protocol fee rate (SCALAR_7 fraction, e.g. 1e6 = 10%)
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
            // WHY: floor rounding -- treasury never over-claims
            total_fee.fixed_mul_floor(&e, &rate, &SCALAR_7)
        } else {
            0
        }
    }

    #[only_owner]
    fn set_rate(e: Env, rate: i128) {
        // WHY: upper bound at 50% prevents admin from extracting more than half
        // of trading revenue, protecting the vault/user fee share.
        if !(0..=SCALAR_7 / 2).contains(&rate) {
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
