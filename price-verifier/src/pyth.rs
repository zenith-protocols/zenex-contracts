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
//! WHY: Signature is verified BEFORE parsing any feed data (defense in depth).
//! An attacker cannot craft a payload that passes parsing but has an invalid signature
//! because verification happens first.

use soroban_sdk::{panic_with_error, BytesN, Bytes, Env, Vec};
use soroban_sdk::unwrap::UnwrapOptimized;

use crate::error::PriceVerifierError;
use crate::PriceData;

const SOLANA_FORMAT_MAGIC: u32 = 0x821A01B9;
const PAYLOAD_FORMAT_MAGIC: u32 = 0x93C7D375;
const PROP_PRICE: u8 = 0;
const PROP_EXPONENT: u8 = 4;
const PROP_CONFIDENCE: u8 = 5;
/// Maximum buffer size for price update payloads (prevents excessive memory usage in WASM).
const MAX_BUF: usize = 1024;

fn read_u16(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn read_u64(buf: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&buf[off..off + 8]);
    u64::from_le_bytes(a)
}

/// Check that a price is not stale relative to the current ledger timestamp.
///
/// WHY: Uses `abs_diff` instead of `now - publish_time` for clock-skew tolerance.
/// In some network conditions, the oracle publish_time may be slightly ahead of the
/// ledger timestamp. `abs_diff` handles both directions without underflow.
///
/// # Panics
/// - `PriceVerifierError::PriceStale` if `|now - publish_time| > max_staleness`
pub fn check_staleness(env: &Env, price: &PriceData, max_staleness: u64) {
    let now = env.ledger().timestamp();
    let age = now.abs_diff(price.publish_time);
    if age > max_staleness {
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
    let len = update_data.len() as usize;
    if len > MAX_BUF { panic_with_error!(env, PriceVerifierError::InvalidData); }
    let mut buf = [0u8; MAX_BUF];
    #[allow(clippy::needless_range_loop)]
    for i in 0..len {
        // SAFETY: i < len where len = update_data.len(); index always valid
        // Note: Soroban Vec::get() requires u32 index; iter_mut().enumerate() would
        // lose the direct buf[i] = val assignment pattern needed here.
        buf[i] = update_data.get(i as u32).unwrap_optimized();
    }

    if len < 102 { panic_with_error!(env, PriceVerifierError::InvalidData); }

    if read_u32(&buf, 0) != SOLANA_FORMAT_MAGIC {
        panic_with_error!(env, PriceVerifierError::InvalidData);
    }

    let sig = BytesN::<64>::from_array(env, &core::array::from_fn(|i| buf[4 + i]));
    let pubkey = BytesN::<32>::from_array(env, &core::array::from_fn(|i| buf[68 + i]));

    if pubkey != trusted_signer {
        panic_with_error!(env, PriceVerifierError::InvalidData);
    }

    let payload_len = read_u16(&buf, 100) as usize;
    let ps = 102; // payload start
    if len < ps + payload_len { panic_with_error!(env, PriceVerifierError::InvalidData); }

    let payload = Bytes::from_slice(env, &buf[ps..ps + payload_len]);
    env.crypto().ed25519_verify(&pubkey, &payload, &sig);

    if payload_len < 14 { panic_with_error!(env, PriceVerifierError::InvalidData); }

    if read_u32(&buf, ps) != PAYLOAD_FORMAT_MAGIC {
        panic_with_error!(env, PriceVerifierError::InvalidData);
    }

    let publish_time = read_u64(&buf, ps + 4) / 1_000_000;
    let num_feeds = buf[ps + 13];

    let mut off = ps + 14;
    let mut results: Vec<PriceData> = Vec::new(env);

    for _ in 0..num_feeds {
        if off + 5 > len { panic_with_error!(env, PriceVerifierError::InvalidData); }
        let feed_id = read_u32(&buf, off);
        let num_props = buf[off + 4];
        off += 5;

        let mut price: Option<i64> = None;
        let mut confidence: Option<i64> = None;
        let mut exponent: Option<i32> = None;

        for _ in 0..num_props {
            if off >= len { panic_with_error!(env, PriceVerifierError::InvalidData); }
            match buf[off] {
                PROP_PRICE | PROP_CONFIDENCE => {
                    let prop = buf[off];
                    off += 1;
                    if off + 8 > len { panic_with_error!(env, PriceVerifierError::InvalidData); }
                    let val = read_u64(&buf, off) as i64;
                    off += 8;
                    if prop == PROP_PRICE { price = Some(val); } else { confidence = Some(val); }
                }
                PROP_EXPONENT => {
                    off += 1;
                    if off + 2 > len { panic_with_error!(env, PriceVerifierError::InvalidData); }
                    exponent = Some(read_u16(&buf, off) as i16 as i32);
                    off += 2;
                }
                _ => panic_with_error!(env, PriceVerifierError::InvalidData),
            }
        }

        let exp = match exponent {
            Some(e) => e,
            None => panic_with_error!(env, PriceVerifierError::InvalidPrice),
        };
        let raw_price = match price {
            Some(p) => p as i128,
            None => panic_with_error!(env, PriceVerifierError::InvalidPrice),
        };
        if let Some(raw_conf) = confidence {
            let raw_conf = raw_conf as i128;
            if raw_conf * 10_000 > raw_price.abs() * max_confidence_bps as i128 {
                panic_with_error!(env, PriceVerifierError::InvalidPrice);
            }
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
