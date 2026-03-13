use soroban_sdk::{BytesN, Bytes, Env, Vec};

use crate::error::OracleError;
use crate::PriceData;

const SOLANA_FORMAT_MAGIC: u32 = 0x821A01B9;
const PAYLOAD_FORMAT_MAGIC: u32 = 0x93C7D375;
const PROP_PRICE: u8 = 0;
const PROP_EXPONENT: u8 = 4;
const PROP_CONFIDENCE: u8 = 5;
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

/// Verify a Pyth Lazer update blob and extract all price feeds.
///
/// Expected wire format:
///   Envelope: magic(4) | sig(64) | pubkey(32) | payload_len(u16) | payload
///   Payload:  magic(4) | timestamp(8) | channel(1) | num_feeds(1) | feeds...
///   Feed:     feed_id(4) | num_props(1) | [type(1) + value]...
///
/// Only price (type 0, i64) and exponent (type 4, i16) properties are accepted.
pub fn verify_and_extract(
    env: &Env,
    update_data: Bytes,
    trusted_signer: &BytesN<32>,
    max_confidence_bps: u32,
) -> Result<Vec<PriceData>, OracleError> {
    // Copy into native memory so all reads are free of host-call overhead.
    let len = update_data.len() as usize;
    if len > MAX_BUF { return Err(OracleError::BufferTooShort); }
    let mut buf = [0u8; MAX_BUF];
    for i in 0..len {
        buf[i] = update_data.get(i as u32).unwrap();
    }

    // ── Envelope ────────────────────────────────────────────────────────
    if len < 102 { return Err(OracleError::BufferTooShort); }

    if read_u32(&buf, 0) != SOLANA_FORMAT_MAGIC {
        return Err(OracleError::InvalidMagic);
    }

    let sig = BytesN::<64>::from_array(env, &core::array::from_fn(|i| buf[4 + i]));
    let pubkey = BytesN::<32>::from_array(env, &core::array::from_fn(|i| buf[68 + i]));

    if pubkey != *trusted_signer {
        return Err(OracleError::InvalidSigner);
    }

    let payload_len = read_u16(&buf, 100) as usize;
    let ps = 102; // payload start
    if len < ps + payload_len { return Err(OracleError::BufferTooShort); }

    let payload = Bytes::from_slice(env, &buf[ps..ps + payload_len]);
    env.crypto().ed25519_verify(&pubkey, &payload, &sig);

    // ── Payload header ──────────────────────────────────────────────────
    if payload_len < 14 { return Err(OracleError::BufferTooShort); }

    if read_u32(&buf, ps) != PAYLOAD_FORMAT_MAGIC {
        return Err(OracleError::InvalidPayloadMagic);
    }

    let publish_time = read_u64(&buf, ps + 4) / 1_000_000;
    let num_feeds = buf[ps + 13];

    // ── Feeds ───────────────────────────────────────────────────────────
    let mut off = ps + 14;
    let mut results: Vec<PriceData> = Vec::new(env);

    for _ in 0..num_feeds {
        if off + 5 > len { return Err(OracleError::BufferTooShort); }
        let feed_id = read_u32(&buf, off);
        let num_props = buf[off + 4];
        off += 5;

        let mut price: Option<i64> = None;
        let mut confidence: Option<i64> = None;
        let mut exponent: Option<i32> = None;

        for _ in 0..num_props {
            if off >= len { return Err(OracleError::BufferTooShort); }
            match buf[off] {
                PROP_PRICE | PROP_CONFIDENCE => {
                    let prop = buf[off];
                    off += 1;
                    if off + 8 > len { return Err(OracleError::BufferTooShort); }
                    let val = read_u64(&buf, off) as i64;
                    off += 8;
                    if prop == PROP_PRICE { price = Some(val); } else { confidence = Some(val); }
                }
                PROP_EXPONENT => {
                    off += 1;
                    if off + 2 > len { return Err(OracleError::BufferTooShort); }
                    exponent = Some(read_u16(&buf, off) as i16 as i32);
                    off += 2;
                }
                _ => return Err(OracleError::UnknownProperty),
            }
        }

        let exp = exponent.ok_or(OracleError::MissingExponent)?;
        let raw_price = price.ok_or(OracleError::MissingPrice)? as i128;
        if let Some(raw_conf) = confidence {
            let raw_conf = raw_conf as i128;
            if raw_conf * 10_000 > raw_price.abs() * max_confidence_bps as i128 {
                return Err(OracleError::ConfidenceTooHigh);
            }
        }
        results.push_back(PriceData {
            feed_id,
            price: raw_price,
            exponent: exp,
            publish_time,
        });
    }

    Ok(results)
}
