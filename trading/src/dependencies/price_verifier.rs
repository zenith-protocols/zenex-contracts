use soroban_sdk::{contractclient, contracttype, Bytes, Env, Vec};

/// Raw price data returned by the price-verifier.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    pub feed_id: u32,
    pub price: i128,
    pub exponent: i32,
    pub publish_time: u64,
}

/// Price-verifier contract interface (used in tests; trading calls via factory-injected address).
#[allow(dead_code)]
#[contractclient(name = "PriceVerifierClient")]
pub trait PriceVerifier {
    fn verify_prices(env: Env, price: Bytes) -> Vec<PriceData>;
}

/// Derive price_scalar from the Pyth exponent: 10^(-exponent)
pub fn scalar_from_exponent(exponent: i32) -> i128 {
    10i128.pow((-exponent) as u32)
}
