//! Pyth Lazer binary format parser and Ed25519 signature verifier.
//!
//! # Binary format (Solana-compatible Pyth Lazer envelope)
//! ```text
//! [0..4]    magic: 0x821A01B9 (Solana format identifier)
//! [4..68]   signature: Ed25519 signature (64 bytes)
//! [68..100] pubkey: Ed25519 public key (32 bytes)
//! [100..102] payload_len: u16 LE
//! [102..]   payload:
//!   [0..4]   magic: 0x93C7D375 (payload format identifier)
//!   [4..12]  publish_time: u64 LE (microseconds, divided by 1e6 -> seconds)
//!   [13]     num_feeds: u8
//!   For each feed:
//!     [0..4]  feed_id: u32 LE
//!     [4]     num_props: u8
//!     For each property:
//!       [0]     prop_type: u8 (0=price, 4=exponent, 5=confidence)
//!       [1..]   value: i64 LE (price/confidence) or i16 LE (exponent)
//! ```
//!
//! Signature is verified BEFORE parsing any feed data (defense in depth).
//! An attacker cannot craft a payload that passes parsing but has an invalid signature
//! because verification happens first.

use soroban_sdk::{panic_with_error, BytesN, Bytes, Env, Vec};

use crate::error::PriceVerifierError;
use crate::PriceData;

const SOLANA_FORMAT_MAGIC: [u8; 4] = 0x821A01B9_u32.to_le_bytes();
const PAYLOAD_FORMAT_MAGIC: [u8; 4] = 0x93C7D375_u32.to_le_bytes();
const PROP_PRICE: u8 = 0;      // i64 LE (8 bytes)
const PROP_EXPONENT: u8 = 4;   // i16 LE (2 bytes)
const PROP_CONFIDENCE: u8 = 5; // i64 LE (8 bytes)

// Envelope layout offsets
const OFF_SIG: usize = 4;       // [4..68]   Ed25519 signature
const OFF_PUBKEY: usize = 68;   // [68..100] Ed25519 public key
const OFF_PAYLOAD_LEN: usize = 100; // [100..102] payload length (u16 LE)
const ENVELOPE_HEADER: usize = 102;  // total envelope header size

// Payload layout offsets (relative to payload start)
const OFF_PUBLISH_TIME: usize = 4;  // [4..12] publish_time (u64 LE, microseconds)
const OFF_NUM_FEEDS: usize = 13;    // [13]    number of price feeds
const PAYLOAD_HEADER: usize = 14;   // minimum payload size (magic + time + padding + count)

// Feed layout sizes
const FEED_HEADER: usize = 5; // feed_id (4) + num_props (1)

/// Maximum buffer size for price update payloads.
/// Derived from: 102 (envelope) + 14 (payload header) + 50 feeds × 32 bytes/feed = 1716.
/// Validated against real Pyth Lazer API: 50 feeds × 3 props = 1416 bytes.
const MAX_BUF: usize = 2048;

fn read_u16(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(buf[off..off + 2].try_into().unwrap())
}

fn read_u64(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(buf[off..off + 8].try_into().unwrap())
}

/// Check that a price is not stale relative to the current ledger timestamp.
///
/// Rejects future publish times, the oracle always publishes before the transaction
/// is included in a block, so `publish_time > now` indicates a malformed payload.
///
/// # Panics
/// - `PriceVerifierError::PriceStale` if price is older than `max_staleness` or in the future
pub fn check_staleness(env: &Env, price: &PriceData, max_staleness: u64) {
    let now = env.ledger().timestamp();
    if price.publish_time > now || now - price.publish_time > max_staleness {
        panic_with_error!(env, PriceVerifierError::PriceStale);
    }
}

/// Verify the Ed25519 signature on a Pyth Lazer binary payload, then parse and
/// return all price feeds contained within.
///
/// # Verification steps
/// 1. Check minimum length and Solana format magic bytes
/// 2. Extract signature (64 bytes) and public key (32 bytes)
/// 3. Verify public key matches `trusted_signer` from storage
/// 4. Verify Ed25519 signature over the payload bytes
/// 5. Check payload format magic bytes
/// 6. Parse each feed's price, exponent, and optional confidence
/// 7. Reject any feed where `confidence > price * max_confidence_bps / 10000`
///
/// # Panics
/// - `PriceVerifierError::InvalidData` on any format or signature error
/// - `PriceVerifierError::InvalidPrice` if required fields missing or confidence too wide
pub fn verify_and_extract(
    env: &Env,
    update_data: Bytes,
) -> Vec<PriceData> {
    let trusted_signer = crate::storage::get_signer(env);
    let max_confidence_bps = crate::storage::get_max_confidence_bps(env);
    // Copy update_data to native buffer for parsing
    let len = update_data.len() as usize;
    if len > MAX_BUF { panic_with_error!(env, PriceVerifierError::InvalidData); }
    let mut buf = [0u8; MAX_BUF];
    update_data.copy_into_slice(&mut buf[..len]);

    // --- Envelope validation ---
    if len < ENVELOPE_HEADER { panic_with_error!(env, PriceVerifierError::InvalidData); }
    if buf[0..4] != SOLANA_FORMAT_MAGIC {
        panic_with_error!(env, PriceVerifierError::InvalidData);
    }

    // Extract signature and public key from fixed envelope positions
    let sig = BytesN::<64>::from_array(env, &core::array::from_fn(|i| buf[OFF_SIG + i]));
    let pubkey = BytesN::<32>::from_array(env, &core::array::from_fn(|i| buf[OFF_PUBKEY + i]));

    if pubkey != trusted_signer {
        panic_with_error!(env, PriceVerifierError::InvalidData);
    }

    // --- Signature verification (before any payload parsing) ---
    let payload_len = read_u16(&buf, OFF_PAYLOAD_LEN) as usize;
    if len < ENVELOPE_HEADER + payload_len {
        panic_with_error!(env, PriceVerifierError::InvalidData);
    }

    // Slice the original host Bytes (avoids round-trip through native buf)
    let payload = update_data.slice(ENVELOPE_HEADER as u32..(ENVELOPE_HEADER + payload_len) as u32);
    env.crypto().ed25519_verify(&pubkey, &payload, &sig);

    // --- Payload parsing (signature verified, data is trusted) ---
    if payload_len < PAYLOAD_HEADER {
        panic_with_error!(env, PriceVerifierError::InvalidData);
    }
    let ps = ENVELOPE_HEADER; // payload start offset in buf
    if buf[ps..ps + 4] != PAYLOAD_FORMAT_MAGIC {
        panic_with_error!(env, PriceVerifierError::InvalidData);
    }

    // publish_time is microseconds in the wire format, convert to seconds
    let publish_time = read_u64(&buf, ps + OFF_PUBLISH_TIME) / 1_000_000;
    let num_feeds = buf[ps + OFF_NUM_FEEDS];

    let mut off = ps + PAYLOAD_HEADER;
    let mut results: Vec<PriceData> = Vec::new(env);

    // --- Parse each feed ---
    for _ in 0..num_feeds {
        if off + FEED_HEADER > len { panic_with_error!(env, PriceVerifierError::InvalidData); }
        let feed_id = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        let num_props = buf[off + 4];
        off += FEED_HEADER;

        let mut price: Option<i64> = None;
        let mut confidence: Option<i64> = None;
        let mut exponent: Option<i32> = None;

        // Parse variable-length properties (type tag + value)
        for _ in 0..num_props {
            if off >= len { panic_with_error!(env, PriceVerifierError::InvalidData); }
            match buf[off] {
                PROP_PRICE | PROP_CONFIDENCE => {
                    let prop = buf[off];
                    off += 1; // skip type tag
                    if off + 8 > len { panic_with_error!(env, PriceVerifierError::InvalidData); }
                    let val = read_u64(&buf, off) as i64;
                    off += 8; // i64 value
                    if prop == PROP_PRICE { price = Some(val); } else { confidence = Some(val); }
                }
                PROP_EXPONENT => {
                    off += 1; // skip type tag
                    if off + 2 > len { panic_with_error!(env, PriceVerifierError::InvalidData); }
                    exponent = Some(read_u16(&buf, off) as i16 as i32);
                    off += 2; // i16 value
                }
                _ => panic_with_error!(env, PriceVerifierError::InvalidData),
            }
        }

        // All three properties are required
        let exp = match exponent {
            Some(e) => e,
            None => panic_with_error!(env, PriceVerifierError::InvalidPrice),
        };
        let raw_price = match price {
            Some(p) => p as i128,
            None => panic_with_error!(env, PriceVerifierError::InvalidPrice),
        };
        let raw_conf = match confidence {
            Some(c) => c as i128,
            None => panic_with_error!(env, PriceVerifierError::InvalidPrice),
        };

        // Reject if confidence interval is too wide relative to price
        // e.g. max_confidence_bps=200 means confidence must be < 2% of price
        if raw_conf * 10_000 > raw_price.abs() * max_confidence_bps as i128 {
            panic_with_error!(env, PriceVerifierError::InvalidPrice);
        }
        results.push_back(PriceData {
            feed_id,
            price: raw_price,
            exponent: exp,
            publish_time,
        });
    }

    results
}
