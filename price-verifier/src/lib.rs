#![no_std]

mod error;
mod pyth;
mod storage;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, BytesN, Env, Vec};
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_macros::only_owner;

use error::OracleError;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    pub feed_id: u32,
    pub price: i128,
    pub exponent: i32,
    pub publish_time: u64,
}

#[contract]
pub struct PriceVerifier;

#[contractimpl]
impl PriceVerifier {
    pub fn __constructor(env: Env, owner: Address, trusted_signer: BytesN<32>, max_confidence_bps: u32) {
        ownable::set_owner(&env, &owner);
        storage::set_signer(&env, &trusted_signer);
        storage::set_max_confidence_bps(&env, max_confidence_bps);
    }

    /// Verify a Pyth Lazer price update and return raw price data with exponent.
    pub fn verify_prices(
        env: Env,
        update_data: Bytes,
    ) -> Result<Vec<PriceData>, OracleError> {
        let signer = storage::get_signer(&env);
        let max_confidence_bps = storage::get_max_confidence_bps(&env);
        pyth::verify_and_extract(&env, update_data, &signer, max_confidence_bps)
    }

    // -- Owner only -------------------------------------------------------------

    /// Update the trusted signer public key. Owner only.
    #[only_owner]
    pub fn update_trusted_signer(env: Env, new_signer: BytesN<32>) {
        storage::set_signer(&env, &new_signer);
    }

    /// Update the max confidence basis points. Owner only.
    #[only_owner]
    pub fn update_max_confidence_bps(env: Env, max_confidence_bps: u32) {
        storage::set_max_confidence_bps(&env, max_confidence_bps);
    }

    // -- Getters ----------------------------------------------------------------

    pub fn max_confidence_bps(env: Env) -> u32 {
        storage::get_max_confidence_bps(&env)
    }
}

#[contractimpl(contracttrait)]
impl Ownable for PriceVerifier {}

#[cfg(test)]
mod test;
