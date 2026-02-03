#![cfg(feature = "testutils")]

use crate::constants::{SCALAR_7, SCALAR_18};
use crate::{MarketConfig, TradingContract};
use sep_40_oracle::Asset;
use soroban_sdk::{testutils::Address as _, Address, Env, Symbol};

#[allow(dead_code)]
pub(crate) fn create_trading(e: &Env) -> Address {
    e.register(
        TradingContract {},
        (
            Address::generate(e),
            Address::generate(e),
            Address::generate(e),
            0u32,
            4u32,
        ),
    )
}

//***** Oracle ******

// pub(crate) fn create_mock_oracle(e: &Env) -> (Address, MockPriceOracleClient) {
//     let contract_address = e.register(MockPriceOracleWASM, ());
//     (
//         contract_address.clone(),
//         MockPriceOracleClient::new(e, &contract_address),
//     )
// }

/// Create a default market config for testing
/// The asset field is a placeholder - the actual asset is set by execute_set_market
pub fn default_market(e: &Env) -> MarketConfig {
    MarketConfig {
        asset: Asset::Other(Symbol::new(e, "PLACEHOLDER")), // Placeholder - set during set_market
        enabled: true,
        max_payout: 10 * SCALAR_7,          // 10.0 (1000% max payout)
        min_collateral: SCALAR_7,           // 10 tokens minimum
        max_collateral: 1000000 * SCALAR_7, // 1M tokens maximum

        init_margin: 0_0100000,        // 1% = 1_00_000 (in SCALAR_7)
        maintenance_margin: 0_0050000, // 0.5% = 50_000 (in SCALAR_7)

        base_fee: 0_0005000, // 0.05% = 5_000 (in SCALAR_7)
        price_impact_scalar: 8_000_000_000 * SCALAR_7, // BTC: 8_000_000_000, XLM: 700_000_000
        base_hourly_rate: 10000000000000, // 0.001% = 10000000000000 (in SCALAR_18)
        ratio_cap: 5 * SCALAR_18, // 5x ratio cap (in SCALAR_18)
    }
}
