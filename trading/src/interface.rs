#![allow(clippy::too_many_arguments)]

use crate::trading::ExecuteRequest;
use crate::types::{MarketConfig, MarketData, Position, TradingConfig};
use soroban_sdk::{contractclient, Address, Bytes, Env, Vec};

/// ### Trading
///
/// A perpetual futures trading contract supporting leveraged long/short positions
/// with limit orders, market orders, funding rates, and auto-deleveraging.
#[contractclient(name = "TradingClient")]
pub trait Trading {
    /********** Admin **********/

    /// (Owner only) Update the trading configuration
    fn set_config(e: Env, config: TradingConfig);

    /// (Owner only) Add a new market or update an existing market's configuration
    fn set_market(e: Env, feed_id: u32, config: MarketConfig);

    /// (Owner only) Remove a market. Fails if any positions remain (notional != 0).
    fn del_market(e: Env, feed_id: u32);

    /// (Owner only) Set the contract status to an admin-level state.
    fn set_status(e: Env, status: u32);

    /// Permissionless status update based on price data.
    fn update_status(e: Env, price: Bytes);

    /********** User Actions **********/

    /// Place a pending limit order.
    fn place_limit(
        e: Env,
        user: Address,
        feed_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
        take_profit: i128,
        stop_loss: i128,
    ) -> u32;

    /// Open a market order filled immediately at the current oracle price.
    fn open_market(
        e: Env,
        user: Address,
        feed_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        take_profit: i128,
        stop_loss: i128,
        price: Bytes,
    ) -> u32;

    /// Cancel a pending (unfilled) limit order.
    fn cancel_limit(e: Env, position_id: u32) -> i128;

    /// Close a filled position at the current oracle price.
    fn close_position(e: Env, position_id: u32, price: Bytes) -> i128;

    /// Add or withdraw collateral on an open position.
    fn modify_collateral(e: Env, position_id: u32, new_collateral: i128, price: Bytes);

    /// Update take profit and stop loss trigger prices.
    fn set_triggers(e: Env, position_id: u32, take_profit: i128, stop_loss: i128);

    /********** Keeper Actions **********/

    /// Execute a batch of keeper actions (fill, SL, TP, liquidate).
    fn execute(e: Env, caller: Address, requests: Vec<ExecuteRequest>, price: Bytes);

    /// Apply funding across all markets (hourly). No price data needed —
    /// funding uses OI imbalance, borrowing uses notional/vault utilization.
    fn apply_funding(e: Env);

    /********** Getters **********/

    fn get_position(e: Env, position_id: u32) -> Position;
    fn get_user_positions(e: Env, user: Address) -> Vec<u32>;
    fn get_market_config(e: Env, feed_id: u32) -> MarketConfig;
    fn get_market_data(e: Env, feed_id: u32) -> MarketData;
    fn get_markets(e: Env) -> Vec<u32>;
    fn get_config(e: Env) -> TradingConfig;
    fn get_status(e: Env) -> u32;
    fn get_vault(e: Env) -> Address;
    fn get_price_verifier(e: Env) -> Address;
    fn get_treasury(e: Env) -> Address;
    fn get_token(e: Env) -> Address;
}
