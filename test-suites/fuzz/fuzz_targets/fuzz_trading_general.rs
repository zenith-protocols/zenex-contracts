#![no_main]

//! Stateful fuzz target for the Zenex trading contract.
//!
//! Exercises the full trading lifecycle across 2 users, 3 markets (BTC/ETH/XLM),
//! and 30 sequential random commands per run.
//!
//! Commands: OpenMarket, PlaceLimit, FillLimit, ClosePosition, CancelLimit,
//! ModifyCollateral, SetTriggers, ApplyFunding, UpdateStatus, PassTime, UpdatePrice.
//!
//! Invariants checked after every operation:
//! 1. **Zero residual** — contract holds 0 tokens when no positions are open
//! 2. **Known errors** — contract errors must be valid TradingError codes
//! 3. **Borrowing index monotonicity** — borrowing indices never decrease
//!    (funding indices are bidirectional and not checked for monotonicity)
//! 4. **ADL index monotonicity** — ADL indices never increase (only shrink)

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ScErrorType;
use soroban_sdk::{vec as svec, Address};
use test_suites::constants::{SCALAR_7, SECONDS_IN_HOUR};
use test_suites::pyth_helper;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::TestFixture;
use trading::testutils::{FEED_BTC, FEED_ETH, FEED_XLM, PRICE_SCALAR};

// ── Constants ───────────────────────────────────────────────────────────────

const FEEDS: [u32; 3] = [FEED_BTC, FEED_ETH, FEED_XLM];

/// Initial oracle prices in Pyth raw format (exponent -8).
const INITIAL_PRICES: [i64; 3] = [
    100_000 * PRICE_SCALAR as i64, // BTC $100k
    2_000 * PRICE_SCALAR as i64,   // ETH $2k
    PRICE_SCALAR as i64 / 10,      // XLM $0.10
];

// ── Fuzz Input ──────────────────────────────────────────────────────────────

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    commands: [FuzzCommand; 30],
}

#[derive(Arbitrary, Debug, Clone)]
enum FuzzCommand {
    /// Open a market order (filled immediately at current price).
    OpenMarket {
        user_idx: bool,
        asset_idx: u8,
        collateral_raw: u16,
        leverage_raw: u8,
        is_long: bool,
    },
    /// Place a limit order (pending, needs fill via execute).
    PlaceLimit {
        user_idx: bool,
        asset_idx: u8,
        collateral_raw: u16,
        leverage_raw: u8,
        is_long: bool,
        /// Entry price offset in bps from current price (-2000 to +2000).
        price_offset_bps: i16,
    },
    /// Attempt to fill pending limit orders via execute().
    FillLimit {
        position_idx: u8,
    },
    /// Close a filled position.
    ClosePosition {
        position_idx: u8,
    },
    /// Cancel a pending limit order.
    CancelLimit {
        position_idx: u8,
    },
    /// Add or remove collateral from a filled position.
    ModifyCollateral {
        position_idx: u8,
        amount_raw: u16,
        is_increase: bool,
    },
    /// Set or update take-profit / stop-loss on a filled position.
    SetTriggers {
        position_idx: u8,
        /// TP offset from entry in bps (positive for longs, negative for shorts).
        tp_offset_bps: u16,
        /// SL offset from entry in bps (negative for longs, positive for shorts).
        sl_offset_bps: u16,
    },
    /// Trigger hourly funding rate recalculation.
    ApplyFunding,
    /// Permissionless circuit breaker / ADL check.
    UpdateStatus,
    /// Advance the ledger clock.
    PassTime {
        seconds: u16,
    },
    /// Change an asset's oracle price.
    UpdatePrice {
        asset_idx: u8,
        change_bps: i16,
    },
}

// ── Position Tracking ───────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct TrackedPosition {
    id: u32,
    user: Address,
    market_id: u32,
    is_filled: bool,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Per-operation expected error codes. Any other contract error indicates a bug.
const OPEN_ERRORS: &[u32] = &[
    724, // NotionalBelowMinimum
    725, // NotionalAboveMaximum
    726, // LeverageAboveMaximum
    741, // ContractOnIce
    751, // UtilizationExceeded
];
const LIMIT_ERRORS: &[u32] = &[
    724, // NotionalBelowMinimum
    725, // NotionalAboveMaximum
    726, // LeverageAboveMaximum
    741, // ContractOnIce
];
const EXECUTE_ERRORS: &[u32] = &[
    720, // PositionNotFound
    731, // NotActionable
];
const CLOSE_ERRORS: &[u32] = &[
    720, // PositionNotFound
    732, // PositionTooNew
];
const CANCEL_ERRORS: &[u32] = &[
    720, // PositionNotFound
    721, // PositionNotPending (if somehow filled between place and cancel)
];
const MODIFY_ERRORS: &[u32] = &[
    720, // PositionNotFound (position closed/liquidated between filter and modify)
    727, // CollateralUnchanged
    728, // WithdrawalBreaksMargin
];
const TRIGGER_ERRORS: &[u32] = &[];
const FUNDING_ERRORS: &[u32] = &[
    752, // FundingTooEarly
];
const STATUS_ERRORS: &[u32] = &[
    740, // InvalidStatus (frozen)
    750, // ThresholdNotMet
];

fn verify_expected_error<T, E: core::fmt::Debug>(
    result: &Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
    context: &str,
    allowed: &[u32],
) {
    match result {
        Ok(Ok(_)) | Ok(Err(_)) => {}
        Err(Ok(e)) if e.is_type(ScErrorType::Contract) => {
            let code = e.get_code();
            assert!(
                allowed.contains(&code),
                "[{}] Unexpected contract error {}: not in {:?}",
                context, code, allowed
            );
        }
        Err(Ok(e)) => panic!("[{}] Host error: {:?}", context, e),
        Err(Err(e)) => panic!("[{}] InvokeError: {:?}", context, e),
    }
}

fn is_ok<T, E>(
    result: &Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> bool {
    matches!(result, Ok(Ok(_)))
}

/// Build a signed multi-feed price update at the current ledger timestamp.
fn build_prices(fixture: &TestFixture, prices: &[i64; 3]) -> soroban_sdk::Bytes {
    let ts = fixture.env.ledger().timestamp();
    pyth_helper::build_price_update(
        &fixture.env,
        &fixture.signing_key,
        &[
            pyth_helper::FeedInput { feed_id: FEED_BTC, price: prices[0], exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: FEED_ETH, price: prices[1], exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: FEED_XLM, price: prices[2], exponent: -8, confidence: 0 },
        ],
        ts,
    )
}

/// Build a single-feed signed price update at the current ledger timestamp.
fn build_price(fixture: &TestFixture, feed_id: u32, price: i64) -> soroban_sdk::Bytes {
    fixture.price_for_feed(feed_id, price)
}

/// Look up the price array index for a given feed_id.
fn feed_idx(feed_id: u32) -> usize {
    FEEDS.iter().position(|&f| f == feed_id)
        .expect("tracked position has unknown feed_id")
}

/// Index snapshot for monotonicity checks.
#[derive(Clone, Default)]
struct IndexSnapshot {
    l_borr_idx: [i128; 3],
    s_borr_idx: [i128; 3],
    l_fund_idx: [i128; 3],
    s_fund_idx: [i128; 3],
    l_adl_idx: [i128; 3],
    s_adl_idx: [i128; 3],
}

fn take_snapshot(fixture: &TestFixture) -> IndexSnapshot {
    let mut snap = IndexSnapshot::default();
    for (i, &feed) in FEEDS.iter().enumerate() {
        let md = fixture.trading.get_market_data(&feed);
        snap.l_borr_idx[i] = md.l_borr_idx;
        snap.s_borr_idx[i] = md.s_borr_idx;
        snap.l_fund_idx[i] = md.l_fund_idx;
        snap.s_fund_idx[i] = md.s_fund_idx;
        snap.l_adl_idx[i] = md.l_adl_idx;
        snap.s_adl_idx[i] = md.s_adl_idx;
    }
    snap
}

fn check_invariants(
    fixture: &TestFixture,
    positions: &[TrackedPosition],
    prev: &IndexSnapshot,
) -> IndexSnapshot {
    // 1. Zero residual when no positions open
    if positions.is_empty() {
        let bal = fixture.token.balance(&fixture.trading.address);
        assert_eq!(bal, 0, "Contract holds {} tokens with no open positions", bal);
    }

    // 2. Index monotonicity
    let curr = take_snapshot(fixture);
    for i in 0..3 {
        // Borrowing indices never decrease
        assert!(
            curr.l_borr_idx[i] >= prev.l_borr_idx[i],
            "l_borr_idx[{}] decreased: {} -> {}", i, prev.l_borr_idx[i], curr.l_borr_idx[i]
        );
        assert!(
            curr.s_borr_idx[i] >= prev.s_borr_idx[i],
            "s_borr_idx[{}] decreased: {} -> {}", i, prev.s_borr_idx[i], curr.s_borr_idx[i]
        );

        // ADL indices never increase (reduction only)
        assert!(
            curr.l_adl_idx[i] <= prev.l_adl_idx[i],
            "l_adl_idx[{}] increased: {} -> {}", i, prev.l_adl_idx[i], curr.l_adl_idx[i]
        );
        assert!(
            curr.s_adl_idx[i] <= prev.s_adl_idx[i],
            "s_adl_idx[{}] increased: {} -> {}", i, prev.s_adl_idx[i], curr.s_adl_idx[i]
        );
    }
    curr
}

// ── Fuzz Target ─────────────────────────────────────────────────────────────

fuzz_target!(|input: FuzzInput| {
    let fixture = create_fixture_with_data();

    let user0 = Address::generate(&fixture.env);
    let user1 = Address::generate(&fixture.env);
    let users = [&user0, &user1];

    fixture.token.mint(&user0, &(10_000_000 * SCALAR_7));
    fixture.token.mint(&user1, &(10_000_000 * SCALAR_7));

    let mut positions: Vec<TrackedPosition> = Vec::new();
    let mut prices = INITIAL_PRICES;
    let mut last_funding_time: u64 = 0;
    let mut snapshot = take_snapshot(&fixture);

    for cmd in &input.commands {
        match cmd {
            FuzzCommand::OpenMarket {
                user_idx, asset_idx, collateral_raw, leverage_raw, is_long,
            } => {
                let user = users[*user_idx as usize];
                let feed = FEEDS[(*asset_idx % 3) as usize];
                let collateral = ((*collateral_raw as i128).max(10).min(10_000)) * SCALAR_7;
                let leverage = (*leverage_raw as i128).max(2).min(100);
                let notional = collateral * leverage;
                let price = prices[(*asset_idx % 3) as usize];
                let price_bytes = build_price(&fixture, feed, price);

                let result = fixture.trading.try_open_market(
                    user, &feed, &collateral, &notional, is_long, &0i128, &0i128, &price_bytes,
                );
                verify_expected_error(&result, "OpenMarket", OPEN_ERRORS);

                if let Ok(Ok(pos_id)) = result {
                    positions.push(TrackedPosition { id: pos_id, user: user.clone(), market_id: feed, is_filled: true });
                }
            }

            FuzzCommand::PlaceLimit {
                user_idx, asset_idx, collateral_raw, leverage_raw, is_long, price_offset_bps,
            } => {
                let user = users[*user_idx as usize];
                let feed = FEEDS[(*asset_idx % 3) as usize];
                let collateral = ((*collateral_raw as i128).max(10).min(10_000)) * SCALAR_7;
                let leverage = (*leverage_raw as i128).max(2).min(100);
                let notional = collateral * leverage;
                let base_price = prices[(*asset_idx % 3) as usize] as i128;
                let offset = (*price_offset_bps as i128).max(-2000).min(2000);
                let entry_price = base_price + base_price * offset / 10_000;
                if entry_price <= 0 { continue; }

                let result = fixture.trading.try_place_limit(
                    user, &feed, &collateral, &notional, is_long,
                    &entry_price, &0i128, &0i128,
                );
                verify_expected_error(&result, "PlaceLimit", LIMIT_ERRORS);

                if let Ok(Ok(pos_id)) = result {
                    positions.push(TrackedPosition { id: pos_id, user: user.clone(), market_id: feed, is_filled: false });
                }
            }

            FuzzCommand::FillLimit { position_idx } => {
                let pending: Vec<usize> = positions.iter().enumerate()
                    .filter(|(_, p)| !p.is_filled)
                    .map(|(i, _)| i)
                    .collect();
                if pending.is_empty() { continue; }
                let idx = pending[(*position_idx as usize) % pending.len()];
                let pos = &positions[idx];
                let price_bytes = build_price(&fixture, pos.market_id, prices[feed_idx(pos.market_id)]);
                let keeper = Address::generate(&fixture.env);

                let result = fixture.trading.try_execute(
                    &keeper, &pos.market_id, &svec![&fixture.env, pos.user.clone()], &svec![&fixture.env, pos.id], &price_bytes,
                );
                verify_expected_error(&result, "FillLimit", EXECUTE_ERRORS);

                if is_ok(&result) {
                    positions[idx].is_filled = true;
                }
            }

            FuzzCommand::ClosePosition { position_idx } => {
                let filled: Vec<usize> = positions.iter().enumerate()
                    .filter(|(_, p)| p.is_filled)
                    .map(|(i, _)| i)
                    .collect();
                if filled.is_empty() { continue; }
                let idx = filled[(*position_idx as usize) % filled.len()];
                let pos = &positions[idx];
                let price_bytes = build_price(&fixture, pos.market_id, prices[feed_idx(pos.market_id)]);

                let result = fixture.trading.try_close_position(&pos.user, &pos.id, &price_bytes);
                verify_expected_error(&result, "ClosePosition", CLOSE_ERRORS);

                if is_ok(&result) {
                    positions.remove(idx);
                }
            }

            FuzzCommand::CancelLimit { position_idx } => {
                let pending: Vec<usize> = positions.iter().enumerate()
                    .filter(|(_, p)| !p.is_filled)
                    .map(|(i, _)| i)
                    .collect();
                if pending.is_empty() { continue; }
                let idx = pending[(*position_idx as usize) % pending.len()];
                let pos = &positions[idx];

                let result = fixture.trading.try_cancel_position(&pos.user, &pos.id);
                verify_expected_error(&result, "CancelLimit", CANCEL_ERRORS);

                if is_ok(&result) {
                    positions.remove(idx);
                }
            }

            FuzzCommand::ModifyCollateral {
                position_idx, amount_raw, is_increase,
            } => {
                let filled: Vec<usize> = positions.iter().enumerate()
                    .filter(|(_, p)| p.is_filled)
                    .map(|(i, _)| i)
                    .collect();
                if filled.is_empty() { continue; }
                let idx = filled[(*position_idx as usize) % filled.len()];
                let pos = &positions[idx];

                let pos_data = fixture.trading.get_position(&pos.user, &pos.id);
                let delta = ((*amount_raw as i128).max(1).min(5_000)) * SCALAR_7;
                let new_collateral = if *is_increase {
                    pos_data.col + delta
                } else {
                    (pos_data.col - delta).max(SCALAR_7)
                };

                let price_bytes = build_price(&fixture, pos.market_id, prices[feed_idx(pos.market_id)]);
                let result = fixture.trading.try_modify_collateral(&pos.user, &pos.id, &new_collateral, &price_bytes);
                verify_expected_error(&result, "ModifyCollateral", MODIFY_ERRORS);
            }

            FuzzCommand::SetTriggers {
                position_idx, tp_offset_bps, sl_offset_bps,
            } => {
                let filled: Vec<usize> = positions.iter().enumerate()
                    .filter(|(_, p)| p.is_filled)
                    .map(|(i, _)| i)
                    .collect();
                if filled.is_empty() { continue; }
                let idx = filled[(*position_idx as usize) % filled.len()];
                let pos = &positions[idx];
                let pos_data = fixture.trading.get_position(&pos.user, &pos.id);

                // Compute TP/SL based on entry price and offsets
                let tp_bps = (*tp_offset_bps as i128).min(5000).max(100);
                let sl_bps = (*sl_offset_bps as i128).min(5000).max(100);

                let (tp, sl) = if pos_data.long {
                    (
                        pos_data.entry_price + pos_data.entry_price * tp_bps / 10_000,
                        pos_data.entry_price - pos_data.entry_price * sl_bps / 10_000,
                    )
                } else {
                    (
                        pos_data.entry_price - pos_data.entry_price * tp_bps / 10_000,
                        pos_data.entry_price + pos_data.entry_price * sl_bps / 10_000,
                    )
                };

                let result = fixture.trading.try_set_triggers(&pos.user, &pos.id, &tp, &sl);
                verify_expected_error(&result, "SetTriggers", TRIGGER_ERRORS);
            }

            FuzzCommand::ApplyFunding => {
                let now = fixture.env.ledger().timestamp();
                // Only attempt if at least 1 hour has passed (avoid FundingTooEarly)
                if now.saturating_sub(last_funding_time) >= SECONDS_IN_HOUR {
                    let result = fixture.trading.try_apply_funding();
                    verify_expected_error(&result, "ApplyFunding", FUNDING_ERRORS);
                    if is_ok(&result) {
                        last_funding_time = now;
                    }
                }
            }

            FuzzCommand::UpdateStatus => {
                let price_bytes = build_prices(&fixture, &prices);
                let result = fixture.trading.try_update_status(&price_bytes);
                verify_expected_error(&result, "UpdateStatus", STATUS_ERRORS);
            }

            FuzzCommand::PassTime { seconds } => {
                let secs = (*seconds as u64).max(1).min(86_400);
                fixture.jump(secs);
            }

            FuzzCommand::UpdatePrice { asset_idx, change_bps } => {
                let idx = (*asset_idx % 3) as usize;
                let bps = (*change_bps as i64).max(-5000).min(5000);
                let base = prices[idx];
                let new_price = base + base * bps / 10_000;

                if new_price > 0 {
                    prices[idx] = new_price;
                }
            }
        }

        snapshot = check_invariants(&fixture, &positions, &snapshot);
    }
});
