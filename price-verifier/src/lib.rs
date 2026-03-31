#![no_std]

mod error;
mod pyth;
mod storage;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, BytesN, Env, Vec};
use soroban_sdk::unwrap::UnwrapOptimized;
use stellar_access::ownable::{self as ownable, Ownable};
use stellar_macros::only_owner;

/// Verified price data returned by the oracle.
///
/// The trading contract uses this to determine entry/exit prices and compute PnL.
/// `price_scalar` is derived at the call site as `10^(-exponent)`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    /// Pyth feed identifier (u32 mapping to an asset pair, e.g. BTC/USD).
    pub feed_id: u32,
    /// Price in the feed's native precision (e.g. for exponent -8, 10_000_000_000_000 = $100k).
    pub price: i128,
    /// Negative exponent defining price precision (e.g. -8 means 8 decimal places).
    pub exponent: i32,
    /// Unix timestamp (seconds) when the price was published by the oracle.
    pub publish_time: u64,
}

/// Pyth Lazer price verification. Verifies Ed25519 signatures and enforces
/// staleness and confidence bounds before exposing price data.
#[contract]
pub struct PriceVerifier;

#[contractimpl]
impl PriceVerifier {
    /// Initialize the price verifier with oracle trust parameters.
    ///
    /// # Parameters
    /// - `owner` - Admin address for updating signer/staleness/confidence
    /// - `trusted_signer` - Ed25519 public key of the authorized Pyth Lazer relay
    /// - `max_confidence_bps` - Maximum allowed confidence interval in basis points
    ///   (e.g. 100 = 1%). Prices with wider confidence are rejected.
    /// - `max_staleness` - Maximum age of a price update in seconds. Uses `abs_diff`
    ///   with current ledger timestamp for clock-skew tolerance.
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

    /// Verify a Pyth Lazer price update and return a single price feed.
    ///
    /// Delegates to [`verify_and_extract`](pyth::verify_and_extract) for signature
    /// verification and parsing, then checks staleness on the first result.
    ///
    /// # Panics
    /// - `PriceVerifierError::InvalidData` if signature or format is invalid
    /// - `PriceVerifierError::InvalidPrice` if confidence exceeds bounds or required fields missing
    /// - `PriceVerifierError::PriceStale` if price is older than `max_staleness`
    pub fn verify_price(env: Env, update_data: Bytes) -> PriceData {
        let max_staleness = storage::get_max_staleness(&env);
        let prices = pyth::verify_and_extract(&env, update_data);
        // SAFETY: verify_and_extract guarantees non-empty Vec on success;
        // empty input panics with InvalidData before reaching here
        let price = prices.get(0).unwrap_optimized();
        pyth::check_staleness(&env, &price, max_staleness);
        price
    }

    /// Verify a Pyth Lazer price update and return all price feeds in the payload.
    ///
    /// Each feed is individually staleness-checked. Used by the trading contract's
    /// `update_status` which needs prices for all registered markets simultaneously.
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

    /// Returns the current max confidence interval in basis points.
    pub fn max_confidence_bps(env: Env) -> u32 {
        storage::get_max_confidence_bps(&env)
    }

    /// Returns the current max staleness threshold in seconds.
    pub fn max_staleness(env: Env) -> u64 {
        storage::get_max_staleness(&env)
    }
}

#[contractimpl(contracttrait)]
impl Ownable for PriceVerifier {}

#[cfg(test)]
mod test;
