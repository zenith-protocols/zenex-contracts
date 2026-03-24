#![no_std]

mod error;
mod pyth;
mod storage;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, BytesN, Env, Vec};
use soroban_sdk::unwrap::UnwrapOptimized;
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_macros::only_owner;

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
    pub fn __constructor(
        env: Env,
        owner: Address,
        trusted_signer: BytesN<32>,
        max_confidence_bps: u32,
        max_staleness: u64,
    ) {
        ownable::set_owner(&env, &owner);
        storage::set_signer(&env, &trusted_signer);
        storage::set_max_confidence_bps(&env, max_confidence_bps);
        storage::set_max_staleness(&env, max_staleness);
    }

    /// Verify a Pyth Lazer price update and return a single price.
    pub fn verify_price(env: Env, update_data: Bytes) -> PriceData {
        let max_staleness = storage::get_max_staleness(&env);
        let prices = pyth::verify_and_extract(&env, update_data);
        // SAFETY: verify_and_extract guarantees non-empty Vec on success;
        // empty input panics with InvalidData before reaching here
        let price = prices.get(0).unwrap_optimized();
        pyth::check_staleness(&env, &price, max_staleness);
        price
    }

    /// Verify a Pyth Lazer price update and return all price feeds.
    pub fn verify_prices(env: Env, update_data: Bytes) -> Vec<PriceData> {
        let max_staleness = storage::get_max_staleness(&env);
        let prices = pyth::verify_and_extract(&env, update_data);
        for price in prices.iter() {
            pyth::check_staleness(&env, &price, max_staleness);
        }
        prices
    }


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

    /// Update the max staleness threshold in seconds. Owner only.
    #[only_owner]
    pub fn update_max_staleness(env: Env, max_staleness: u64) {
        storage::set_max_staleness(&env, max_staleness);
    }

    pub fn max_confidence_bps(env: Env) -> u32 {
        storage::get_max_confidence_bps(&env)
    }

    pub fn max_staleness(env: Env) -> u64 {
        storage::get_max_staleness(&env)
    }
}

#[contractimpl(contracttrait)]
impl Ownable for PriceVerifier {}

#[cfg(test)]
mod test;
