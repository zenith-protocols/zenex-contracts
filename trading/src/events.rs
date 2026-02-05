use sep_40_oracle::Asset;
use soroban_sdk::{contractevent, Address};

use crate::TradingConfig;

// Configuration Events

#[contractevent]
#[derive(Clone)]
pub struct SetConfig {
    pub config: TradingConfig,
}

#[contractevent]
#[derive(Clone)]
pub struct QueueSetConfig {
    pub config: TradingConfig,
}

#[contractevent]
#[derive(Clone)]
pub struct CancelSetConfig {
    pub config: TradingConfig,
}

#[contractevent]
#[derive(Clone)]
pub struct QueueSetMarket {
    #[topic]
    pub asset: Asset,
}

#[contractevent]
#[derive(Clone)]
pub struct CancelSetMarket {
    #[topic]
    pub asset: Asset,
}

#[contractevent]
#[derive(Clone)]
pub struct SetMarket {
    #[topic]
    pub asset: Asset,
    pub asset_index: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct SetStatus {
    pub status: u32,
}

// Position Events

#[contractevent]
#[derive(Clone)]
pub struct OpenMarket {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub base_fee: i128,
    pub impact_fee: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct PlaceLimit {
    #[topic]
    pub asset_index: u32,
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
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub price: i128,
    pub pnl: i128,
    pub base_fee: i128,
    pub impact_fee: i128,
    pub interest: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct FillLimit {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub base_fee: i128,
    pub impact_fee: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct Liquidation {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub price: i128,
    pub pnl: i128,
    pub base_fee: i128,
    pub impact_fee: i128,
    pub interest: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct TakeProfit {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub price: i128,
    pub pnl: i128,
    pub base_fee: i128,
    pub impact_fee: i128,
    pub interest: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct StopLoss {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub price: i128,
    pub pnl: i128,
    pub base_fee: i128,
    pub impact_fee: i128,
    pub interest: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct CancelLimit {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub base_fee: i128,
    pub impact_fee: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct ModifyCollateral {
    #[topic]
    pub asset_index: u32,
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
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    #[topic]
    pub position_id: u32,
    pub take_profit: i128,
    pub stop_loss: i128,
}
