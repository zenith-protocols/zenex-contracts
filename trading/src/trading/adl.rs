use crate::constants::SCALAR_18;
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::ADLTriggered;
use crate::storage;
use crate::types::MarketData;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{panic_with_error, Env, Map, Vec};

/// Core ADL logic operating on pre-verified prices.
/// Called by `execute_update_status` when OnIce and threshold still met.
pub(crate) fn do_adl(e: &Env, markets: &Vec<u32>, price_map: &Map<u32, (i128, i128)>) {
    let vault = storage::get_vault(e);

    // Pass 1: load all market data, compute winner/loser PnL
    let mut total_winner_pnl: i128 = 0;
    let mut total_loser_pnl: i128 = 0;
    let mut cached: Vec<(u32, MarketData, i128, i128)> = Vec::new(e);

    for feed_id in markets.iter() {
        let data = storage::get_market_data(e, feed_id);
        let (price, ps) = price_map.get(feed_id).unwrap();

        let long_pnl = price.fixed_mul_floor(e, &data.long_entry_weighted, &ps)
            - data.long_notional_size;
        let short_pnl = data.short_notional_size
            - price.fixed_mul_floor(e, &data.short_entry_weighted, &ps);

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

        cached.push_back((feed_id, data, long_pnl, short_pnl));
    }

    let net_liability = total_winner_pnl - total_loser_pnl;
    let vault_balance = VaultClient::new(e, &vault).total_assets();

    if net_liability <= vault_balance {
        panic_with_error!(e, TradingError::NoDeficit);
    }

    let deficit = net_liability - vault_balance;
    let reduction_pct = deficit.fixed_div_floor(e, &total_winner_pnl, &SCALAR_18);
    let reduction_pct = reduction_pct.min(SCALAR_18);
    let factor = SCALAR_18 - reduction_pct;

    // Pass 2: adjust winning-side aggregates
    for i in 0..cached.len() {
        let (feed_id, mut data, long_pnl, short_pnl) = cached.get(i).unwrap();
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
            storage::set_market_data(e, feed_id, &data);
        }
    }

    ADLTriggered {
        reduction_pct,
        deficit,
    }
    .publish(e);
}
