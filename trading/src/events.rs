use soroban_sdk::{contractevent, Address};

// Configuration Events

#[contractevent]
#[derive(Clone)]
pub struct SetConfig {}

#[contractevent]
#[derive(Clone)]
pub struct SetMarket {
    #[topic]
    pub feed_id: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct SetStatus {
    pub status: u32,
}

// Position Events

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

#[contractevent]
#[derive(Clone)]
pub struct ModifyCollateral {
    #[topic]
    pub feed_id: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub amount: i128, // Positive = deposit, negative = withdraw
}

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

#[contractevent]
#[derive(Clone)]
pub struct DelMarket {
    #[topic]
    pub feed_id: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct ApplyFunding {}

#[contractevent]
#[derive(Clone)]
pub struct ADLMarket {
    #[topic]
    pub feed_id: u32,
    pub factor: i128,            // Reduction factor applied (SCALAR_18, e.g. 0.7e18 = 30% cut)
    pub long: bool,              // Which side was reduced
}

#[contractevent]
#[derive(Clone)]
pub struct ADLTriggered {
    pub reduction_pct: i128,     // Reduction percentage (SCALAR_18)
    pub deficit: i128,           // Deficit amount (token_decimals)
}
