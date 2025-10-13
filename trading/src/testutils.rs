#![cfg(feature = "testutils")]

use crate::{MarketConfig, TradingContract};
use soroban_sdk::{testutils::Address as _, Address, Env};
use crate::constants::SCALAR_7;

pub(crate) fn create_trading(e: &Env) -> Address {
    e.register(
        TradingContract {},
        (
            Address::generate(e),
            Address::generate(e),
            Address::generate(e),
            0_1000000u32,
            4u32,
        )
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

pub fn default_market() -> MarketConfig {
    MarketConfig {
        enabled: true,
        max_payout: 10 * SCALAR_7,          // 10.0 (1000% max payout)
        min_collateral: SCALAR_7,      // 10 tokens minimum
        max_collateral: 1000000 * SCALAR_7, // 1M tokens maximum

        init_margin: 0_0100000,         // 1% = 1_00_000 (in SCALAR_7)
        maintenance_margin: 0_0050000,   // 0.5% = 50_000 (in SCALAR_7)

        base_fee: 0_0005000,              // 0.05% = 50_000 (in SCALAR_7)
        price_impact_scalar: 8_000_000_000 * SCALAR_7,   // BTC: 8_000_000_000, XLM: 700_000_000
        base_hourly_rate: 10000000000000,       // 0.001% = 10000000000000 (in SCALAR_18)
    }
}