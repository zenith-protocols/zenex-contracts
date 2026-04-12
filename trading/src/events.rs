use soroban_sdk::{contractevent, Address};

// ─────────────────────────────────────────────────────────────────
// Event design
//
// Events carry only the fields that CHANGE at the emitting moment.
// Off-chain indexers combine each event with the position row they
// already hold. Fields established by an earlier event (e.g. long,
// col, notional set at PlaceLimit) are not repeated on later events.
//
// Market-level events (ApplyFunding, ADLTriggered) are lifecycle
// signals; consumers that need post-state re-read Market data. The
// indexer runs one Market.loadMultiple per funding tick, not per
// trade, so this is not hot-path.
//
// Scaling (i128 fields):
//   col, notional, fees, pnl, amount   → token_decimals (10^7 on mainnet)
//   entry_price, sl, tp, price         → 10^price_decimals per market
//   fund_idx, borr_idx, adl_idx,
//   reduction_pct                      → SCALAR_18
// ─────────────────────────────────────────────────────────────────

// ── Admin ────────────────────────────────────────────────────────

/// Global trading configuration updated via `set_config`. Indexer
/// audits via (contract_id, tx_hash, event_type); full config read
/// from storage on demand.
#[contractevent]
#[derive(Clone)]
pub struct SetConfig {}

/// Market added or updated via `set_market`.
#[contractevent]
#[derive(Clone)]
pub struct SetMarket {
    #[topic]
    pub market_id: u32,
}

/// Market removed via `del_market`.
#[contractevent]
#[derive(Clone)]
pub struct DelMarket {
    #[topic]
    pub market_id: u32,
}

/// Contract status changed (admin action or circuit breaker).
#[contractevent]
#[derive(Clone)]
pub struct SetStatus {
    pub status: u32,
}

// ── Position lifecycle ───────────────────────────────────────────

/// Pending limit order created via `place_limit`. Establishes the
/// row; `fund_idx` / `borr_idx` / `adl_idx` are not snapshotted
/// until the order fills (indexer writes them as 0).
#[contractevent]
#[derive(Clone)]
pub struct PlaceLimit {
    #[topic]
    pub market_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub long: bool,
    pub col: i128,
    pub notional: i128,
    pub entry_price: i128, // limit trigger price
    pub sl: i128,
    pub tp: i128,
    pub created_at: u64,
}

/// Market order opened and immediately filled via `open_market`.
/// No prior row — event carries full post-fill state.
#[contractevent]
#[derive(Clone)]
pub struct OpenMarket {
    #[topic]
    pub market_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub long: bool,
    pub col: i128,
    pub notional: i128,
    pub entry_price: i128,
    pub sl: i128,
    pub tp: i128,
    pub fund_idx: i128,
    pub borr_idx: i128,
    pub adl_idx: i128,
    pub created_at: u64,
    pub base_fee: i128,
    pub impact_fee: i128,
}

/// Pending limit order filled via `execute`. Emits only fill-time
/// state; long / col / notional / sl / tp are inherited from the
/// prior `PlaceLimit` row.
#[contractevent]
#[derive(Clone)]
pub struct FillLimit {
    #[topic]
    pub market_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub entry_price: i128, // actual fill price (supersedes limit)
    pub fund_idx: i128,
    pub borr_idx: i128,
    pub adl_idx: i128,
    pub created_at: u64, // fill time supersedes placement time
    pub base_fee: i128,
    pub impact_fee: i128,
}

/// Collateral added or withdrawn via `modify_collateral`. Funding
/// accrues → indices refresh. Delta is `col - prior_col`.
#[contractevent]
#[derive(Clone)]
pub struct ModifyCollateral {
    #[topic]
    pub market_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub col: i128, // new collateral
    pub fund_idx: i128,
    pub borr_idx: i128,
    pub adl_idx: i128,
}

/// Take-profit / stop-loss triggers updated via `set_triggers`.
/// Nothing else changes.
#[contractevent]
#[derive(Clone)]
pub struct SetTriggers {
    #[topic]
    pub market_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub sl: i128,
    pub tp: i128,
}

// ── Position close (on-chain Position is deleted) ────────────────
//
// Indexer holds long / col / notional / entry_price / created_at
// in its live row. Events carry only the close-specific payload.

/// Closed by the user via `close_position`.
#[contractevent]
#[derive(Clone)]
pub struct ClosePosition {
    #[topic]
    pub market_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub price: i128, // close price
    pub pnl: i128,
    pub base_fee: i128,
    pub impact_fee: i128,
    pub funding: i128,
    pub borrowing_fee: i128,
}

/// Liquidated by a keeper.
#[contractevent]
#[derive(Clone)]
pub struct Liquidation {
    #[topic]
    pub market_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub price: i128,
    pub base_fee: i128,
    pub impact_fee: i128,
    pub funding: i128,
    pub borrowing_fee: i128,
    pub liq_fee: i128,
}

/// Take-profit trigger executed by a keeper.
#[contractevent]
#[derive(Clone)]
pub struct TakeProfit {
    #[topic]
    pub market_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub price: i128,
    pub pnl: i128,
    pub base_fee: i128,
    pub impact_fee: i128,
    pub funding: i128,
    pub borrowing_fee: i128,
}

/// Stop-loss trigger executed by a keeper.
#[contractevent]
#[derive(Clone)]
pub struct StopLoss {
    #[topic]
    pub market_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub price: i128,
    pub pnl: i128,
    pub base_fee: i128,
    pub impact_fee: i128,
    pub funding: i128,
    pub borrowing_fee: i128,
}

/// Position refunded (market disabled or deleted). The refund amount
/// equals the stored collateral, which the indexer already has.
#[contractevent]
#[derive(Clone)]
pub struct RefundPosition {
    #[topic]
    pub market_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
}

// ── Market-level (indexer refetches affected markets) ────────────

/// Funding rates recalculated via `apply_funding`. Emitted once per
/// tick. Indexer reads post-recalc market state separately.
#[contractevent]
#[derive(Clone)]
pub struct ApplyFunding {}

/// Auto-deleverage triggered — winning-side notionals and adl_idx
/// scaled down. Single event per ADL action; indexer refetches the
/// affected markets.
#[contractevent]
#[derive(Clone)]
pub struct ADLTriggered {
    /// Reduction percentage applied to winning sides (SCALAR_18).
    pub reduction_pct: i128,
    /// Deficit amount: net_pnl - vault_balance (token_decimals).
    pub deficit: i128,
}
