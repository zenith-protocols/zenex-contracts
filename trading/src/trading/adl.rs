use crate::constants::SCALAR_18;
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::ADLTriggered;
use crate::storage;
use crate::trading::oracle::{get_price_scalar, load_price};
use crate::types::MarketData;
use crate::validation::require_on_ice;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{panic_with_error, Env, Vec};

/// Permissionless ADL trigger. Anyone can call.
/// Computes vault deficit from aggregates (O(num_markets)),
/// then adjusts winning-side aggregates and factors.
pub fn execute_trigger_adl(e: &Env) {
    require_on_ice(e);

    let oracle = storage::get_oracle(e);
    let vault = storage::get_vault(e);
    let price_scalar = get_price_scalar(e, &oracle);
    let market_count = storage::get_market_count(e);

    // Pass 1: load all market data + configs, compute winner/loser PnL
    let mut total_winner_pnl: i128 = 0;
    let mut total_loser_pnl: i128 = 0;
    let mut cached: Vec<(MarketData, i128, i128)> = Vec::new(e); // (data, long_pnl, short_pnl)

    for i in 0..market_count {
        let data = storage::get_market_data(e, i);
        let mkt_config = storage::get_market_config(e, i);
        let price = load_price(e, &oracle, &mkt_config.asset);

        let long_pnl = price.fixed_mul_floor(e, &data.long_entry_weighted, &price_scalar)
            - data.long_notional_size;
        let short_pnl = data.short_notional_size
            - price.fixed_mul_floor(e, &data.short_entry_weighted, &price_scalar);

        if long_pnl > 0 {
            total_winner_pnl += long_pnl;
        } else {
            total_loser_pnl += long_pnl.abs();
        }

        if short_pnl > 0 {
            total_winner_pnl += short_pnl;
        } else {
            total_loser_pnl += short_pnl.abs();
        }

        cached.push_back((data, long_pnl, short_pnl));
    }

    let net_liability = total_winner_pnl - total_loser_pnl;
    let vault_balance = VaultClient::new(e, &vault).total_assets();

    // Revert if vault is healthy
    if net_liability <= vault_balance {
        panic_with_error!(e, TradingError::NoDeficit);
    }

    let deficit = net_liability - vault_balance;
    // reduction_pct in SCALAR_18
    let reduction_pct = deficit.fixed_div_floor(e, &total_winner_pnl, &SCALAR_18);
    // Cap at 100%
    let reduction_pct = reduction_pct.min(SCALAR_18);
    let factor = SCALAR_18 - reduction_pct;

    // Pass 2: adjust winning-side aggregates using cached data
    for i in 0..market_count {
        let (mut data, long_pnl, short_pnl) = cached.get(i).unwrap();
        let mut changed = false;

        if long_pnl > 0 {
            data.long_notional_size = data
                .long_notional_size
                .fixed_mul_floor(e, &factor, &SCALAR_18);
            data.long_entry_weighted = data
                .long_entry_weighted
                .fixed_mul_floor(e, &factor, &SCALAR_18);
            data.long_adl_index = data
                .long_adl_index
                .fixed_mul_floor(e, &factor, &SCALAR_18);
            changed = true;
        }

        if short_pnl > 0 {
            data.short_notional_size = data
                .short_notional_size
                .fixed_mul_floor(e, &factor, &SCALAR_18);
            data.short_entry_weighted = data
                .short_entry_weighted
                .fixed_mul_floor(e, &factor, &SCALAR_18);
            data.short_adl_index = data
                .short_adl_index
                .fixed_mul_floor(e, &factor, &SCALAR_18);
            changed = true;
        }

        if changed {
            storage::set_market_data(e, i, &data);
        }
    }

    // Emit event
    ADLTriggered {
        reduction_pct,
        deficit,
    }
    .publish(e);
}
