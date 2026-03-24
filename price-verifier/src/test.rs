
use soroban_sdk::{Bytes, BytesN, Env};
use soroban_sdk::testutils::{Address as _, Ledger};

use crate::{PriceVerifier, PriceVerifierClient};

// Pyth Lazer trusted signer Ed25519 public key.
const TRUSTED_SIGNER: [u8; 32] = [
    0x80, 0xef, 0xc1, 0xf4, 0x80, 0xc5, 0x61, 0x5a,
    0xf3, 0xfb, 0x67, 0x3d, 0x42, 0x28, 0x7e, 0x99,
    0x3d, 0xa9, 0xfb, 0xc3, 0x50, 0x6b, 0x6e, 0x41,
    0xdf, 0xa3, 0x29, 0x50, 0x82, 0x0c, 0x2e, 0x6c,
];

// Real Lazer update: BTC/USD (feed 1) + ETH/USD (feed 2).
// publish_time = 1772647347
// BTC: raw price=7324803585261, exponent=-8
// ETH: raw price=214644436210,  exponent=-8
const UPDATE_HEX: &str = "b9011a82f6d7f6e0b98555f2b0785ec8ad2162c5e19d09de0742f4f1129e3aeaf3899124e2187bf7bf1d31b381079260ce1aae766e395d9476f9ea1628699ee9f830eb0080efc1f480c5615af3fb673d42287e993da9fbc3506b6e41dfa32950820c2e6c420075d3c793402d749f364c06000302010000000300edd45070a906000004f8ff055af7284b00000000020000000300f22ccef93100000004f8ff05789fb60100000000";

// publish_time from the test vector
const PUBLISH_TIME: u64 = 1772647347;

// Default max staleness for tests (10 seconds)
const MAX_STALENESS: u64 = 10;

fn hex_to_bytes(env: &Env, hex: &str) -> Bytes {
    let mut bytes = Bytes::new(env);
    let hex_bytes = hex.as_bytes();
    let mut i = 0;
    while i < hex_bytes.len() {
        let hi = match hex_bytes[i] {
            b'0'..=b'9' => hex_bytes[i] - b'0',
            b'a'..=b'f' => hex_bytes[i] - b'a' + 10,
            b'A'..=b'F' => hex_bytes[i] - b'A' + 10,
            _ => panic!("invalid hex"),
        };
        let lo = match hex_bytes[i + 1] {
            b'0'..=b'9' => hex_bytes[i + 1] - b'0',
            b'a'..=b'f' => hex_bytes[i + 1] - b'a' + 10,
            b'A'..=b'F' => hex_bytes[i + 1] - b'A' + 10,
            _ => panic!("invalid hex"),
        };
        bytes.push_back((hi << 4) | lo);
        i += 2;
    }
    bytes
}

fn setup_env() -> (Env, PriceVerifierClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let owner = soroban_sdk::Address::generate(&env);
    let signer = BytesN::from_array(&env, &TRUSTED_SIGNER);
    let contract_id = env.register(PriceVerifier, (&owner, &signer, &200u32, &MAX_STALENESS));
    let client = PriceVerifierClient::new(&env, &contract_id);
    (env, client)
}

#[test]
fn test_constructor() {
    let (_env, client) = setup_env();
    assert_eq!(client.max_confidence_bps(), 200);
    assert_eq!(client.max_staleness(), MAX_STALENESS);
}

#[test]
fn test_verify_btc_usd() {
    let (env, client) = setup_env();
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME);

    let data = hex_to_bytes(&env, UPDATE_HEX);
    let feeds = client.verify_prices(&data);
    let feed = feeds.get(0).unwrap();

    assert_eq!(feed.feed_id, 1);
    assert_eq!(feed.price, 7_324_803_585_261_i128);
    assert_eq!(feed.exponent, -8);
    assert_eq!(feed.publish_time, PUBLISH_TIME);
}

#[test]
fn test_verify_eth_usd() {
    let (env, client) = setup_env();
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME);

    let data = hex_to_bytes(&env, UPDATE_HEX);
    let feeds = client.verify_prices(&data);
    let feed = feeds.get(1).unwrap(); // ETH is second feed in blob

    assert_eq!(feed.feed_id, 2);
    assert_eq!(feed.price, 214_644_436_210_i128);
    assert_eq!(feed.exponent, -8);
}

#[test]
fn test_verify_multi() {
    let (env, client) = setup_env();
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME);

    let data = hex_to_bytes(&env, UPDATE_HEX);
    let feeds = client.verify_prices(&data);
    assert_eq!(feeds.len(), 2);
    assert_eq!(feeds.get(0).unwrap().price, 7_324_803_585_261_i128);
    assert_eq!(feeds.get(1).unwrap().price, 214_644_436_210_i128);
}

#[test]
fn test_verify_within_staleness() {
    let (env, client) = setup_env();
    // Ledger time is MAX_STALENESS seconds after publish — still valid
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME + MAX_STALENESS);

    let data = hex_to_bytes(&env, UPDATE_HEX);
    let feeds = client.verify_prices(&data);
    assert_eq!(feeds.len(), 2);
}

#[test]
#[should_panic(expected = "Error(Contract, #820)")]
fn test_rejects_stale_price() {
    let (env, client) = setup_env();
    // Ledger time is MAX_STALENESS + 1 seconds after publish — stale
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME + MAX_STALENESS + 1);

    let data = hex_to_bytes(&env, UPDATE_HEX);
    client.verify_prices(&data);
}

#[test]
#[should_panic]
fn test_rejects_wrong_signer() {
    let env = Env::default();
    let admin = soroban_sdk::Address::generate(&env);
    let wrong = BytesN::from_array(&env, &[0xAA; 32]);
    let id = env.register(PriceVerifier, (&admin, &wrong, &200u32, &MAX_STALENESS));
    let client = PriceVerifierClient::new(&env, &id);

    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME);
    let data = hex_to_bytes(&env, UPDATE_HEX);
    client.verify_prices(&data);
}
