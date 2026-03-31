use soroban_sdk::{contractevent, Address};

/// Emitted when the global trading configuration is updated via `set_config`.
#[contractevent]
#[derive(Clone)]
pub struct SetConfig {}

/// Emitted when a market is added or updated via `set_market`.
#[contractevent]
#[derive(Clone)]
pub struct SetMarket {
    #[topic]
    pub feed_id: u32,
}

/// Emitted when the contract status changes (admin or circuit breaker).
#[contractevent]
#[derive(Clone)]
pub struct SetStatus {
    pub status: u32,
}

/// Emitted when a pending limit order is created via `place_limit`.
#[contractevent]
#[derive(Clone)]
pub struct PlaceLimit {
    #[topic]
    pub feed_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
}

/// Emitted when a market order is opened and filled immediately via `open_market`.
#[contractevent]
#[derive(Clone)]
pub struct OpenMarket {
    #[topic]
    pub feed_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub base_fee: i128,
    pub impact_fee: i128,
}

/// Emitted when a pending limit order is filled by a keeper via `execute`.
#[contractevent]
#[derive(Clone)]
pub struct FillLimit {
    #[topic]
    pub feed_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub base_fee: i128,
    pub impact_fee: i128,
}

/// Emitted when a position is closed by the user via `close_position`.
#[contractevent]
#[derive(Clone)]
pub struct ClosePosition {
    #[topic]
    pub feed_id: u32,
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

/// Emitted when a position is liquidated by a keeper.
#[contractevent]
#[derive(Clone)]
pub struct Liquidation {
    #[topic]
    pub feed_id: u32,
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

/// Emitted when a take-profit trigger is executed by a keeper.
#[contractevent]
#[derive(Clone)]
pub struct TakeProfit {
    #[topic]
    pub feed_id: u32,
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

/// Emitted when a stop-loss trigger is executed by a keeper.
#[contractevent]
#[derive(Clone)]
pub struct StopLoss {
    #[topic]
    pub feed_id: u32,
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

/// Emitted when a pending limit order is cancelled via `cancel_limit`.
#[contractevent]
#[derive(Clone)]
pub struct CancelLimit {
    #[topic]
    pub feed_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
}

/// Emitted when collateral is added or withdrawn via `modify_collateral`.
#[contractevent]
#[derive(Clone)]
pub struct ModifyCollateral {
    #[topic]
    pub feed_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    /// Positive = deposit, negative = withdrawal (token_decimals).
    pub amount: i128,
}

/// Emitted when take-profit or stop-loss triggers are updated via `set_triggers`.
#[contractevent]
#[derive(Clone)]
pub struct SetTriggers {
    #[topic]
    pub feed_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub take_profit: i128,
    pub stop_loss: i128,
}

/// Emitted when a market is removed via `del_market`.
#[contractevent]
#[derive(Clone)]
pub struct DelMarket {
    #[topic]
    pub feed_id: u32,
}

/// Emitted when funding rates are recalculated via `apply_funding`.
#[contractevent]
#[derive(Clone)]
pub struct ApplyFunding {}

/// Emitted per-market when ADL reduces a side's notional.
#[contractevent]
#[derive(Clone)]
pub struct ADLMarket {
    #[topic]
    pub feed_id: u32,
    /// Reduction factor applied (SCALAR_18, e.g. 0.7e18 = 30% reduction).
    pub factor: i128,
    /// Which side was reduced: `true` = longs, `false` = shorts.
    pub long: bool,
}

/// Emitted once when ADL is triggered, summarizing the overall reduction.
#[contractevent]
#[derive(Clone)]
pub struct ADLTriggered {
    /// Reduction percentage applied to winning sides (SCALAR_18).
    pub reduction_pct: i128,
    /// Deficit amount: net_pnl - vault_balance (token_decimals).
    pub deficit: i128,
}
