use sep_40_oracle::Asset;
use soroban_sdk::{contractevent, Address, Env};

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
    pub asset: Asset,
}

#[contractevent]
#[derive(Clone)]
pub struct SetStatus {
    pub status: u32,
}

// Position Events

#[contractevent]
#[derive(Clone)]
pub struct OpenPosition {
    #[topic]
    pub asset: Asset,
    #[topic]
    pub user: Address,
    pub position_id: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct ClosePosition {
    #[topic]
    pub asset: Asset,
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
    pub asset: Asset,
    #[topic]
    pub user: Address,
    pub position_id: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct Liquidation {
    #[topic]
    pub asset: Asset,
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
    pub asset: Asset,
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
    pub asset: Asset,
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
    pub asset: Asset,
    #[topic]
    pub user: Address,
    pub position_id: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct WithdrawCollateral {
    #[topic]
    pub asset: Asset,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct DepositCollateral {
    #[topic]
    pub asset: Asset,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct SetTakeProfit {
    #[topic]
    pub asset: Asset,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub price: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct SetStopLoss {
    #[topic]
    pub asset: Asset,
    #[topic]
    pub user: Address,
    pub position_id: u32,
    pub price: i128,
}

// Helper functions to emit events

pub fn emit_set_config(e: &Env, oracle: Address, caller_take_rate: i128, max_positions: u32) {
    SetConfig {
        oracle,
        caller_take_rate,
        max_positions,
    }
    .publish(e);
}

pub fn emit_queue_set_config(
    e: &Env,
    oracle: Address,
    caller_take_rate: i128,
    max_positions: u32,
    unlock_time: u64,
) {
    QueueSetConfig {
        oracle,
        caller_take_rate,
        max_positions,
        unlock_time,
    }
    .publish(e);
}

pub fn emit_cancel_set_config(e: &Env) {
    CancelSetConfig {}.publish(e);
}

pub fn emit_queue_set_market(e: &Env, asset: Asset, config: MarketConfig) {
    QueueSetMarket { asset, config }.publish(e);
}

pub fn emit_cancel_set_market(e: &Env, asset: Asset) {
    CancelSetMarket { asset }.publish(e);
}

pub fn emit_set_market(e: &Env, asset: Asset) {
    SetMarket { asset }.publish(e);
}

pub fn emit_set_status(e: &Env, status: u32) {
    SetStatus { status }.publish(e);
}

pub fn emit_open_position(e: &Env, user: Address, asset: Asset, position_id: u32) {
    OpenPosition {
        asset,
        user,
        position_id,
    }
    .publish(e);
}

pub fn emit_close_position(
    e: &Env,
    user: Address,
    asset: Asset,
    position_id: u32,
    price: i128,
    fee: i128,
) {
    ClosePosition {
        asset,
        user,
        position_id,
        price,
        fee,
    }
    .publish(e);
}

pub fn emit_fill_position(e: &Env, user: Address, asset: Asset, position_id: u32) {
    FillPosition {
        asset,
        user,
        position_id,
    }
    .publish(e);
}

pub fn emit_liquidation(
    e: &Env,
    user: Address,
    asset: Asset,
    position_id: u32,
    price: i128,
    fee: i128,
) {
    Liquidation {
        asset,
        user,
        position_id,
        price,
        fee,
    }
    .publish(e);
}

pub fn emit_take_profit(
    e: &Env,
    user: Address,
    asset: Asset,
    position_id: u32,
    price: i128,
    fee: i128,
) {
    TakeProfit {
        asset,
        user,
        position_id,
        price,
        fee,
    }
    .publish(e);
}

pub fn emit_stop_loss(
    e: &Env,
    user: Address,
    asset: Asset,
    position_id: u32,
    price: i128,
    fee: i128,
) {
    StopLoss {
        asset,
        user,
        position_id,
        price,
        fee,
    }
    .publish(e);
}

pub fn emit_cancel_position(e: &Env, user: Address, asset: Asset, position_id: u32) {
    CancelPosition {
        asset,
        user,
        position_id,
    }
    .publish(e);
}

pub fn emit_withdraw_collateral(
    e: &Env,
    user: Address,
    asset: Asset,
    position_id: u32,
    amount: i128,
) {
    WithdrawCollateral {
        asset,
        user,
        position_id,
        amount,
    }
    .publish(e);
}

pub fn emit_deposit_collateral(
    e: &Env,
    user: Address,
    asset: Asset,
    position_id: u32,
    amount: i128,
) {
    DepositCollateral {
        asset,
        user,
        position_id,
        amount,
    }
    .publish(e);
}

pub fn emit_set_take_profit(e: &Env, user: Address, asset: Asset, position_id: u32, price: i128) {
    SetTakeProfit {
        asset,
        user,
        position_id,
        price,
    }
    .publish(e);
}

pub fn emit_set_stop_loss(e: &Env, user: Address, asset: Asset, position_id: u32, price: i128) {
    SetStopLoss {
        asset,
        user,
        position_id,
        price,
    }
    .publish(e);
}
