use crate::errors::TradingError;
use soroban_sdk::{contracttype, panic_with_error, Address, Env};

/// Global trading parameters set by the admin.
#[contracttype]
#[derive(Clone, Debug)]
pub struct TradingConfig {
    pub caller_rate: i128,  // keeper's share of fees (SCALAR_7)
    pub min_notional: i128, // minimum notional per position (token_decimals)
    pub max_notional: i128, // maximum notional per position (token_decimals)
    pub fee_dom: i128,      // fee rate for dominant side (SCALAR_7)
    pub fee_non_dom: i128,  // fee rate for non-dominant side (SCALAR_7)
    pub max_util: i128,     // global notional / vault cap (SCALAR_7)
    pub r_funding: i128,    // base hourly funding rate (SCALAR_18)
    pub r_base: i128,       // base hourly borrowing rate (SCALAR_18)
    pub r_var: i128,        // borrowing multiplier at full util (SCALAR_7)
}

/// Per-market parameters set by the admin via `set_market`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketConfig {
    pub enabled: bool,   // whether market accepts new positions
    pub max_util: i128,  // per-market notional / vault cap (SCALAR_7)
    pub r_borrow: i128,  // per-market borrowing weight (SCALAR_7, 1e7 = 1x)
    pub margin: i128,    // initial margin; max leverage = 1/margin (SCALAR_7)
    pub liq_fee: i128,   // liquidation fee + threshold (SCALAR_7)
    pub impact: i128,    // price impact divisor (SCALAR_7)
}

/// Per-market mutable state, updated on every position action.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketData {
    pub l_notional: i128, // total long notional (token_decimals)
    pub s_notional: i128, // total short notional (token_decimals)
    pub l_fund_idx: i128, // cumulative long funding index (SCALAR_18)
    pub s_fund_idx: i128, // cumulative short funding index (SCALAR_18)
    pub l_borr_idx: i128, // cumulative long borrowing index (SCALAR_18)
    pub s_borr_idx: i128, // cumulative short borrowing index (SCALAR_18)
    pub l_entry_wt: i128, // Σ(notional/entry_price) for longs
    pub s_entry_wt: i128, // Σ(notional/entry_price) for shorts
    pub fund_rate: i128,  // current funding rate, +longs pay/-shorts pay (SCALAR_18)
    pub last_update: u64, // last accrual timestamp
    pub l_adl_idx: i128,  // long ADL reduction index (SCALAR_18)
    pub s_adl_idx: i128,  // short ADL reduction index (SCALAR_18)
}

/// A leveraged perpetual position (pending limit order or filled).
#[contracttype]
#[derive(Clone)]
pub struct Position {
    pub user: Address,     // position owner
    pub filled: bool,      // false = pending limit order
    pub feed: u32,         // market feed ID
    pub long: bool,        // long or short
    pub sl: i128,          // stop loss price, 0 = not set
    pub tp: i128,          // take profit price, 0 = not set
    pub entry_price: i128, // entry price
    pub col: i128,         // collateral (token_decimals)
    pub notional: i128,    // notional size (token_decimals)
    pub fund_idx: i128,    // funding index snapshot at fill (SCALAR_18)
    pub borr_idx: i128,    // borrowing index snapshot at fill (SCALAR_18)
    pub adl_idx: i128,     // ADL index snapshot at fill (SCALAR_18)
    pub created_at: u64,   // creation timestamp
}

/// Batch execution request for keeper triggers.
#[contracttype]
#[derive(Clone)]
pub struct ExecuteRequest {
    pub request_type: u32,
    pub position_id: u32,
}

#[derive(Clone, PartialEq, Debug)]
#[repr(u32)]
pub enum ExecuteRequestType {
    Fill = 0,
    StopLoss = 1,
    TakeProfit = 2,
    Liquidate = 3,
}

impl ExecuteRequestType {
    pub fn from_u32(e: &Env, value: u32) -> Self {
        match value {
            0 => ExecuteRequestType::Fill,
            1 => ExecuteRequestType::StopLoss,
            2 => ExecuteRequestType::TakeProfit,
            3 => ExecuteRequestType::Liquidate,
            _ => panic_with_error!(e, TradingError::InvalidRequestType),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
#[repr(u32)]
pub enum ContractStatus {
    Active = 0,
    OnIce = 1,       // Permissionless circuit breaker (PnL threshold)
    AdminOnIce = 2,  // Admin-set on ice (only admin can lift)
    Frozen = 3,      // Admin-set full freeze
}

impl ContractStatus {
    pub fn from_u32(e: &Env, value: u32) -> Self {
        match value {
            0 => ContractStatus::Active,
            1 => ContractStatus::OnIce,
            2 => ContractStatus::AdminOnIce,
            3 => ContractStatus::Frozen,
            _ => panic_with_error!(e, TradingError::InvalidStatus),
        }
    }
}
