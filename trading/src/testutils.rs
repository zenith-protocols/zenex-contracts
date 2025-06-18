#![cfg(feature = "testutils")]

use crate::{MarketConfig, TradingContract};
use soroban_sdk::{testutils::Address as _, Address, Env};

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
        max_leverage: 1000,              // 10x leverage (keep as integer)
        max_payout: 10_0000000,          // 10.0 (1000% max payout)
        min_collateral: 10_0000000,      // 10 tokens minimum
        max_collateral: 100_000_0000000, // 100,000 tokens maximum
        liquidation_threshold: 0_0500000, // 5% (0.05 in SCALAR_7)
        total_available: 1_0000000, // 10M tokens

        base_fee: 0,              // 0.05% = 5_000 (in SCALAR_7)
        price_impact_scalar: 0,   // BTC: 8_000_000_000, XLM: 700_000_000
        min_hourly_rate: 0,       // 0.0003% = 30
        max_hourly_rate: 0,       // BTC: 0.009% = 900, XLM: 0.016% = 1_600
        target_hourly_rate: 0,    // BTC: 0.001% = 100, XLM: 0.002% = 200
        target_utilization: 0_8000000,    // 80% = 8_000_000
    }
}