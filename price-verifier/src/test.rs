
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

// All blobs share the same publish_time (fetched concurrently from Pyth Lazer REST API)
const PUBLISH_TIME: u64 = 1_775_140_467;
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

fn load_1_feed(env: &Env) -> Bytes {
    hex_to_bytes(env, include_str!("testdata/1_feed.hex").trim())
}

fn load_2_feeds(env: &Env) -> Bytes {
    hex_to_bytes(env, include_str!("testdata/2_feeds.hex").trim())
}

fn load_50_feeds(env: &Env) -> Bytes {
    hex_to_bytes(env, include_str!("testdata/50_feeds.hex").trim())
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
fn test_verify_single_feed() {
    let (env, client) = setup_env();
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME);

    let feed = client.verify_price(&load_1_feed(&env));
    assert_eq!(feed.feed_id, 1);
    assert_eq!(feed.price, 6_651_333_675_616_i128);
    assert_eq!(feed.exponent, -8);
    assert_eq!(feed.publish_time, PUBLISH_TIME);
}

#[test]
fn test_verify_btc_usd() {
    let (env, client) = setup_env();
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME);

    let feeds = client.verify_prices(&load_2_feeds(&env));
    let feed = feeds.get(0).unwrap();

    assert_eq!(feed.feed_id, 1);
    assert_eq!(feed.price, 6_651_333_675_616_i128);
    assert_eq!(feed.exponent, -8);
    assert_eq!(feed.publish_time, PUBLISH_TIME);
}

#[test]
fn test_verify_eth_usd() {
    let (env, client) = setup_env();
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME);

    let feeds = client.verify_prices(&load_2_feeds(&env));
    let feed = feeds.get(1).unwrap();

    assert_eq!(feed.feed_id, 2);
    assert_eq!(feed.price, 205_033_408_168_i128);
    assert_eq!(feed.exponent, -8);
}

#[test]
fn test_verify_multi() {
    let (env, client) = setup_env();
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME);

    let feeds = client.verify_prices(&load_2_feeds(&env));
    assert_eq!(feeds.len(), 2);
    assert_eq!(feeds.get(0).unwrap().feed_id, 1);
    assert_eq!(feeds.get(1).unwrap().feed_id, 2);
}

#[test]
fn test_verify_50_feeds() {
    let (env, client) = setup_env();
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME);

    let feeds = client.verify_prices(&load_50_feeds(&env));
    assert_eq!(feeds.len(), 50);
    assert_eq!(feeds.get(0).unwrap().feed_id, 1);
    assert_eq!(feeds.get(49).unwrap().feed_id, 51);
}

#[test]
fn test_verify_within_staleness() {
    let (env, client) = setup_env();
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME + MAX_STALENESS);

    let feeds = client.verify_prices(&load_2_feeds(&env));
    assert_eq!(feeds.len(), 2);
}

#[test]
#[should_panic(expected = "Error(Contract, #782)")]
fn test_rejects_stale_price() {
    let (env, client) = setup_env();
    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME + MAX_STALENESS + 1);

    client.verify_prices(&load_2_feeds(&env));
}

#[test]
#[should_panic]
fn test_rejects_wrong_signer() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = soroban_sdk::Address::generate(&env);
    let wrong = BytesN::from_array(&env, &[0xAA; 32]);
    let id = env.register(PriceVerifier, (&admin, &wrong, &200u32, &MAX_STALENESS));
    let client = PriceVerifierClient::new(&env, &id);

    env.ledger().with_mut(|li| li.timestamp = PUBLISH_TIME);
    client.verify_prices(&load_2_feeds(&env));
}
