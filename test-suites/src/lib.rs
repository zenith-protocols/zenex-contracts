pub mod assertions;
pub mod constants;
pub mod test_fixture;
pub mod setup;
pub mod pyth_helper;
mod token;
pub mod dependencies;

pub use constants::SCALAR_7;

/// Convert trading::TradingConfig to factory::TradingConfig (same XDR, different Rust types).
pub fn to_factory_config(tc: &trading::TradingConfig) -> factory::TradingConfig {
    factory::TradingConfig {
        caller_rate: tc.caller_rate,
        min_notional: tc.min_notional,
        max_notional: tc.max_notional,
        fee_dom: tc.fee_dom,
        fee_non_dom: tc.fee_non_dom,
        max_util: tc.max_util,
        r_funding: tc.r_funding,
        r_base: tc.r_base,
        r_var: tc.r_var,
    }
}
