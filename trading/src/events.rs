use sep_40_oracle::Asset;
use soroban_sdk::{contractevent, Address};

use crate::MarketConfig;

// Configuration Events

#[contractevent]
#[derive(Clone)]
pub struct SetConfig {
    pub oracle: Address,
    pub caller_take_rate: i128,
    pub max_positions: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct QueueSetConfig {
    pub oracle: Address,
    pub caller_take_rate: i128,
    pub max_positions: u32,
    pub unlock_time: u64,
}

#[contractevent]
#[derive(Clone)]
pub struct CancelSetConfig {}

// Market setup events still use Asset since index doesn't exist yet
#[contractevent]
#[derive(Clone)]
pub struct QueueSetMarket {
    #[topic]
    pub asset: Asset,
    pub config: MarketConfig,
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
    pub asset_index: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct SetStatus {
    pub status: u32,
}

// Position Events - all use asset_index

#[contractevent]
#[derive(Clone)]
pub struct OpenPosition {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct ClosePosition {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub price: i128,
    pub fee: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct FillPosition {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct Liquidation {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub price: i128,
    pub fee: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct TakeProfit {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub price: i128,
    pub fee: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct StopLoss {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub price: i128,
    pub fee: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct CancelPosition {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct WithdrawCollateral {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct DepositCollateral {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct SetTakeProfit {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub price: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct SetStopLoss {
    #[topic]
    pub asset_index: u32,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub price: i128,
}
