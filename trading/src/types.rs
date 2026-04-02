use crate::errors::TradingError;
use soroban_sdk::{contracttype, panic_with_error, Address, Env};

#[contracttype]
#[derive(Clone, Debug)]
pub struct TradingConfig {
    pub caller_rate:  i128, // keeper's share of trading fees (SCALAR_7)
    pub min_notional: i128, // minimum notional per position (token_decimals)
    pub max_notional: i128, // maximum notional per position (token_decimals)
    pub fee_dom:      i128, // trading fee rate for dominant side (SCALAR_7)
    pub fee_non_dom:  i128, // trading fee rate for non-dominant side (SCALAR_7)
    pub max_util:     i128, // global utilization cap: total_notional / vault_balance (SCALAR_7)
    pub r_funding:    i128, // base hourly funding rate (SCALAR_18)
    pub r_base:       i128, // base hourly borrowing rate (SCALAR_18)
    pub r_var:        i128, // vault-level variable borrowing rate at full vault utilization (SCALAR_18)
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketConfig {
    pub enabled:  bool,  // true = active, false = disabled (positions refunded)
    pub max_util: i128, // per-market utilization cap (SCALAR_7)
    pub r_var_market: i128, // per-market variable borrowing rate at full market utilization (SCALAR_18)
    pub margin:   i128, // initial margin requirement, max leverage = 1/margin (SCALAR_7)
    pub liq_fee:  i128, // liquidation fee/threshold, must be < margin (SCALAR_7)
    pub impact:   i128, // price-impact fee divisor, fee = notional / impact (SCALAR_7)
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketData {
    pub l_notional:  i128, // total long open interest (token_decimals)
    pub s_notional:  i128, // total short open interest (token_decimals)
    pub l_fund_idx:  i128, // cumulative long funding index (SCALAR_18)
    pub s_fund_idx:  i128, // cumulative short funding index (SCALAR_18)
    pub l_borr_idx:  i128, // cumulative long borrowing index (SCALAR_18)
    pub s_borr_idx:  i128, // cumulative short borrowing index (SCALAR_18)
    pub l_entry_wt:  i128, // sum of notional/entry_price for longs, for O(1) ADL PnL
    pub s_entry_wt:  i128, // sum of notional/entry_price for shorts, for O(1) ADL PnL
    pub fund_rate:   i128, // current funding rate, positive = longs pay (SCALAR_18)
    pub last_update: u64,  // timestamp of last accrual (seconds)
    pub l_adl_idx:   i128, // long ADL reduction index, starts at SCALAR_18
    pub s_adl_idx:   i128, // short ADL reduction index, starts at SCALAR_18
}

#[contracttype]
#[derive(Clone)]
pub struct Position {
    pub user:        Address, // position owner
    pub filled:      bool,    // false = pending limit, true = filled
    pub feed:        u32,     // price feed ID
    pub long:        bool,    // true = long, false = short
    pub sl:          i128,    // stop-loss trigger price, 0 = not set (price_scalar)
    pub tp:          i128,    // take-profit trigger price, 0 = not set (price_scalar)
    pub entry_price: i128,    // entry price at fill (price_scalar)
    pub col:         i128,    // current collateral (token_decimals)
    pub notional:    i128,    // notional size, may be reduced by ADL (token_decimals)
    pub fund_idx:    i128,    // funding index snapshot at fill (SCALAR_18)
    pub borr_idx:    i128,    // borrowing index snapshot at fill (SCALAR_18)
    pub adl_idx:     i128,    // ADL index snapshot at fill (SCALAR_18)
    pub created_at:  u64,     // timestamp of creation or fill (seconds)
}

/// Contract operational state.
///
/// Active -> OnIce: permissionless via update_status (ADL threshold)
/// OnIce -> Active: permissionless via update_status (PnL < 90%)
/// Active/OnIce -> AdminOnIce/Frozen: admin via set_status
/// Admin cannot set OnIce (reserved for circuit breaker)
#[derive(Clone, PartialEq, Debug)]
#[repr(u32)]
pub enum ContractStatus {
    Active    = 0, // normal operation, all actions permitted
    OnIce     = 1, // circuit breaker, new opens blocked
    AdminOnIce = 2, // admin restriction, same as OnIce
    Frozen    = 3, // full freeze, all position operations blocked
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
