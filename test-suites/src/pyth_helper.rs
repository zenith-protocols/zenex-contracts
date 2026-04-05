//! Pyth Lazer payload construction and Ed25519 signing for integration tests.
//!
//! Builds binary payloads that match the format expected by price-verifier/src/pyth.rs.
//! Uses real Ed25519 signing via `ed25519-dalek` so the on-chain PriceVerifier
//! contract's signature verification passes without mocking.

use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::{Bytes, Env};

/// Outer envelope magic (Solana format header).
const SOLANA_FORMAT_MAGIC: u32 = 0x821A01B9;

/// Inner payload magic (Pyth Lazer format).
const PAYLOAD_FORMAT_MAGIC: u32 = 0x93C7D375;

/// Property type: price (8-byte LE u64).
const PROP_PRICE: u8 = 0;

/// Property type: exponent (2-byte LE i16).
const PROP_EXPONENT: u8 = 4;

/// Property type: confidence interval (8-byte LE u64).
const PROP_CONFIDENCE: u8 = 5;

/// Input for a single price feed within a Pyth Lazer update.
pub struct FeedInput {
    pub feed_id: u32,
    pub price: i64,
    pub exponent: i16,
    /// Confidence interval (max confidence = 0, i.e. no uncertainty).
    pub confidence: u64,
}

/// Generate a deterministic Ed25519 keypair for tests.
/// Returns `(signing_key, public_key_bytes)`.
pub fn test_keypair() -> (SigningKey, [u8; 32]) {
    let seed: [u8; 32] = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31, 32,
    ];
    let signing_key = SigningKey::from_bytes(&seed);
    let pubkey = signing_key.verifying_key().to_bytes();
    (signing_key, pubkey)
}

/// Build a signed Pyth Lazer update blob.
///
/// `timestamp_secs` should match or be close to the Soroban ledger timestamp
/// (within `max_staleness` seconds).
///
/// # Layout
/// ```text
/// [4B magic][64B signature][32B pubkey][2B payload_len][payload...]
/// ```
///
/// # Payload layout
/// ```text
/// [4B magic][8B publish_time_us][1B channel][1B num_feeds]
/// Per feed: [4B feed_id][1B num_props]
///   Per prop: [1B type][value bytes]
///     PROP_PRICE: 8B LE u64
///     PROP_EXPONENT: 2B LE i16
///     PROP_CONFIDENCE: 8B LE u64
/// ```
pub fn build_price_update(
    env: &Env,
    signing_key: &SigningKey,
    feeds: &[FeedInput],
    timestamp_secs: u64,
) -> Bytes {
    // 1. Build payload
    // Layout: [4B magic][8B publish_time_us][1B channel][1B num_feeds][feeds...]
    // Note: no reserved bytes between channel and num_feeds (matching pyth.rs parsing)
    let mut payload = std::vec::Vec::<u8>::new();
    payload.extend_from_slice(&PAYLOAD_FORMAT_MAGIC.to_le_bytes());
    // publish_time in microseconds
    payload.extend_from_slice(&(timestamp_secs * 1_000_000u64).to_le_bytes());
    payload.push(0u8); // channel
    payload.push(feeds.len() as u8); // num_feeds

    for feed in feeds {
        payload.extend_from_slice(&feed.feed_id.to_le_bytes());
        payload.push(3u8); // num_props: price, exponent, confidence

        // Price property
        payload.push(PROP_PRICE);
        payload.extend_from_slice(&(feed.price as u64).to_le_bytes());

        // Exponent property
        payload.push(PROP_EXPONENT);
        payload.extend_from_slice(&feed.exponent.to_le_bytes());

        // Confidence property
        payload.push(PROP_CONFIDENCE);
        payload.extend_from_slice(&feed.confidence.to_le_bytes());
    }

    // 2. Sign payload
    let signature = signing_key.sign(&payload);
    let pubkey = signing_key.verifying_key();

    // 3. Assemble full blob: [magic][sig][pubkey][payload_len][payload]
    let mut blob = std::vec::Vec::<u8>::new();
    blob.extend_from_slice(&SOLANA_FORMAT_MAGIC.to_le_bytes());
    blob.extend_from_slice(&signature.to_bytes());
    blob.extend_from_slice(pubkey.as_bytes());
    blob.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    blob.extend_from_slice(&payload);

    Bytes::from_slice(env, &blob)
}

/// Convenience: build a single-feed BTC price update at the given timestamp.
pub fn btc_price_update(
    env: &Env,
    signing_key: &SigningKey,
    price: i64,
    timestamp: u64,
) -> Bytes {
    build_price_update(
        env,
        signing_key,
        &[FeedInput {
            feed_id: 1,
            price,
            exponent: -8,
            confidence: 0,
        }],
        timestamp,
    )
}

/// Convenience: build a multi-feed update for BTC + ETH + XLM.
pub fn multi_price_update(
    env: &Env,
    signing_key: &SigningKey,
    btc_price: i64,
    eth_price: i64,
    xlm_price: i64,
    timestamp: u64,
) -> Bytes {
    build_price_update(
        env,
        signing_key,
        &[
            FeedInput {
                feed_id: 1,
                price: btc_price,
                exponent: -8,
                confidence: 0,
            },
            FeedInput {
                feed_id: 2,
                price: eth_price,
                exponent: -8,
                confidence: 0,
            },
            FeedInput {
                feed_id: 3,
                price: xlm_price,
                exponent: -8,
                confidence: 0,
            },
        ],
        timestamp,
    )
}

