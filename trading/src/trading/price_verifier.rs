use crate::errors::TradingError;
use soroban_sdk::{contractclient, contracttype, panic_with_error, Bytes, Env, Vec};

/// Raw price data returned by the price-verifier.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    pub feed_id: u32,
    pub price: i128,
    pub exponent: i32,
    pub publish_time: u64,
}

/// Price-verifier contract interface.
#[contractclient(name = "PriceVerifierClient")]
pub trait PriceVerifier {
    fn verify_prices(env: Env, price: Bytes) -> Vec<PriceData>;
}

/// Panic if the price is too stale.
pub fn check_staleness(e: &Env, publish_time: u64, max_staleness: u64) {
    let now = e.ledger().timestamp();
    let age = if now >= publish_time { now - publish_time } else { publish_time - now };
    if age > max_staleness {
        panic_with_error!(e, TradingError::PriceStale);
    }
}

/// Derive price_scalar from the Pyth exponent: 10^(-exponent)
pub fn scalar_from_exponent(exponent: i32) -> i128 {
    10i128.pow((-exponent) as u32)
}
