use crate::constants::{SCALAR_7, SCALAR_18, UTIL_ACTIVE, UTIL_ONICE};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{ADLMarket, ADLTriggered, SetStatus};
use crate::storage;
use crate::dependencies::{scalar_from_exponent, PriceData};
use crate::types::{ContractStatus, MarketData};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{panic_with_error, Env, Vec};

/// Permissionless status update based on price data.
/// - Active: if threshold met -> ADL (sets OnIce)
/// - OnIce: if threshold dropped -> Active, else -> ADL
pub fn execute_update_status(e: &Env, feeds: &Vec<PriceData>) {
    let current = ContractStatus::from_u32(e, storage::get_status(e));
    let vault = storage::get_vault(e);
    let markets = storage::get_markets(e);
    let vault_balance = VaultClient::new(e, &vault).total_assets();

    // Single pass: compute per-market per-side PnL, net PnL, and winner total
    let mut cached: Vec<(u32, MarketData, i128, i128)> = Vec::new(e);
    let mut net_pnl: i128 = 0;
    let mut total_winner_pnl: i128 = 0;

    for feed_id in markets.iter() {
        let data = storage::get_market_data(e, feed_id);
        let (price, ps) = feeds
            .iter()
            .find(|f| f.feed_id == feed_id)
            .map(|f| (f.price, scalar_from_exponent(f.exponent)))
            .unwrap();

        let long_pnl = price.fixed_mul_floor(e, &data.l_entry_wt, &ps) - data.l_notional;
        let short_pnl = data.s_notional - price.fixed_mul_floor(e, &data.s_entry_wt, &ps);

        net_pnl += long_pnl + short_pnl;
        if long_pnl > 0 { total_winner_pnl += long_pnl; }
        if short_pnl > 0 { total_winner_pnl += short_pnl; }

        cached.push_back((feed_id, data, long_pnl, short_pnl));
    }

    match current {
        ContractStatus::Active => {
            let onice_line = vault_balance.fixed_mul_floor(e, &UTIL_ONICE, &SCALAR_7);
            if net_pnl < onice_line {
                panic_with_error!(e, TradingError::ThresholdNotMet);
            }
            do_adl(e, &cached, total_winner_pnl, net_pnl, vault_balance);
        }
        ContractStatus::OnIce => {
            let active_line = vault_balance.fixed_mul_floor(e, &UTIL_ACTIVE, &SCALAR_7);
            if net_pnl < active_line {
                storage::set_status(e, ContractStatus::Active as u32);
                SetStatus { status: ContractStatus::Active as u32 }.publish(e);
            } else {
                do_adl(e, &cached, total_winner_pnl, net_pnl, vault_balance);
            }
        }
        _ => panic_with_error!(e, TradingError::InvalidStatus),
    }
}

/// Reduce winning-side positions when vault cannot cover net liability.
/// Sets status to OnIce.
fn do_adl(
    e: &Env,
    cached: &Vec<(u32, MarketData, i128, i128)>,
    total_winner_pnl: i128,
    net_pnl: i128,
    vault_balance: i128,
) {
    if net_pnl <= vault_balance {
        panic_with_error!(e, TradingError::ThresholdNotMet);
    }

    let deficit = net_pnl - vault_balance;
    let reduction_pct = deficit.fixed_div_floor(e, &total_winner_pnl, &SCALAR_18);
    let reduction_pct = reduction_pct.min(SCALAR_18);
    let factor = SCALAR_18 - reduction_pct;

    let trading_config = storage::get_config(e);

    let mut new_total: i128 = 0;
    for i in 0..cached.len() {
        let (feed_id, mut data, long_pnl, short_pnl) = cached.get(i).unwrap();

        // Accrue indices against pre-ADL notionals before reducing them
        let config = storage::get_market_config(e, feed_id);
        data.accrue(e, trading_config.r_base, trading_config.r_var, config.r_borrow, vault_balance);

        let mut changed = false;

        if long_pnl > 0 {
            data.l_notional = data.l_notional.fixed_mul_floor(e, &factor, &SCALAR_18);
            data.l_entry_wt = data.l_entry_wt.fixed_mul_floor(e, &factor, &SCALAR_18);
            data.l_adl_idx = data.l_adl_idx.fixed_mul_floor(e, &factor, &SCALAR_18);
            ADLMarket { feed_id, factor, long: true }.publish(e);
            changed = true;
        }

        if short_pnl > 0 {
            data.s_notional = data.s_notional.fixed_mul_floor(e, &factor, &SCALAR_18);
            data.s_entry_wt = data.s_entry_wt.fixed_mul_floor(e, &factor, &SCALAR_18);
            data.s_adl_idx = data.s_adl_idx.fixed_mul_floor(e, &factor, &SCALAR_18);
            ADLMarket { feed_id, factor, long: false }.publish(e);
            changed = true;
        }

        new_total += data.l_notional + data.s_notional;

        if changed {
            storage::set_market_data(e, feed_id, &data);
        }
    }
    storage::set_total_notional(e, new_total);
    storage::set_status(e, ContractStatus::OnIce as u32);

    ADLTriggered {
        reduction_pct,
        deficit,
    }
    .publish(e);
}

#[cfg(test)]
mod tests {
    use crate::constants::{SCALAR_18, SCALAR_7};
    use crate::storage;
    use crate::testutils::{
        create_trading_with_vault, default_market, jump,
        BTC_FEED_ID, BTC_PRICE, PRICE_SCALAR,
    };
    use crate::dependencies::PriceData;
    use crate::types::ContractStatus;
    use soroban_sdk::{vec, Address, Env};

    fn btc_feed(e: &Env) -> PriceData {
        PriceData {
            feed_id: BTC_FEED_ID,
            price: BTC_PRICE,
            exponent: -8,
            publish_time: e.ledger().timestamp(),
        }
    }

    fn setup_small_vault(e: &Env, vault_amount: i128) -> Address {
        let (contract, _owner) = create_trading_with_vault(e, vault_amount);

        e.as_contract(&contract, || {
            let market_config = default_market(e);
            crate::trading::config::execute_set_market(e, BTC_FEED_ID, &market_config);
        });

        contract
    }

    fn set_market_positions(
        e: &Env,
        contract: &Address,
        long_notional: i128,
        short_notional: i128,
        entry_price: i128,
    ) {
        let long_entry_wt = long_notional * PRICE_SCALAR / entry_price;
        let short_entry_wt = short_notional * PRICE_SCALAR / entry_price;

        e.as_contract(contract, || {
            let mut data = storage::get_market_data(e, BTC_FEED_ID);
            data.l_notional = long_notional;
            data.s_notional = short_notional;
            data.l_entry_wt = long_entry_wt;
            data.s_entry_wt = short_entry_wt;
            data.l_adl_idx = SCALAR_18;
            data.s_adl_idx = SCALAR_18;
            storage::set_market_data(e, BTC_FEED_ID, &data);
            storage::set_total_notional(e, long_notional + short_notional);
        });
    }

    #[test]
    fn test_update_status_active_to_on_ice_with_adl() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        // Small vault: 100 tokens
        let contract = setup_small_vault(&e, 100 * SCALAR_7);

        // Longs with huge PnL relative to vault
        set_market_positions(&e, &contract, 1000 * SCALAR_7, 0, 50_000 * PRICE_SCALAR);

        e.as_contract(&contract, || {
            let data_before = storage::get_market_data(&e, BTC_FEED_ID);

            let feeds = vec![&e, btc_feed(&e)];
            super::execute_update_status(&e, &feeds);

            assert_eq!(storage::get_status(&e), ContractStatus::OnIce as u32);

            // ADL should have reduced the winning long side
            let data_after = storage::get_market_data(&e, BTC_FEED_ID);
            assert!(data_after.l_notional < data_before.l_notional);
            assert!(data_after.l_adl_idx < SCALAR_18);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #780)")]
    fn test_update_status_threshold_not_met() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        // Large vault relative to positions
        let contract = setup_small_vault(&e, 100_000_000 * SCALAR_7);

        // Tiny positions — net PnL is negligible
        set_market_positions(&e, &contract, 100 * SCALAR_7, 0, BTC_PRICE);

        e.as_contract(&contract, || {
            let feeds = vec![&e, btc_feed(&e)];
            super::execute_update_status(&e, &feeds);
        });
    }

    #[test]
    fn test_update_status_onice_to_active() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        // Large vault — positions are small
        let contract = setup_small_vault(&e, 100_000_000 * SCALAR_7);

        set_market_positions(&e, &contract, 100 * SCALAR_7, 100 * SCALAR_7, BTC_PRICE);

        e.as_contract(&contract, || {
            storage::set_status(&e, ContractStatus::OnIce as u32);

            let feeds = vec![&e, btc_feed(&e)];
            super::execute_update_status(&e, &feeds);

            assert_eq!(storage::get_status(&e), ContractStatus::Active as u32);
        });
    }

    #[test]
    fn test_adl_reduces_winning_side() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        // Vault with 5_000 tokens
        let contract = setup_small_vault(&e, 5_000 * SCALAR_7);

        // Longs: 50k notional entered at $50k, current price $100k => +50k PnL
        // Shorts: 30k notional entered at $50k, current price $100k => -30k PnL
        // Net = 20k, vault = 5k => deficit = 15k
        set_market_positions(&e, &contract, 50_000 * SCALAR_7, 30_000 * SCALAR_7, 50_000 * PRICE_SCALAR);

        e.as_contract(&contract, || {
            storage::set_status(&e, ContractStatus::OnIce as u32);

            let data_before = storage::get_market_data(&e, BTC_FEED_ID);

            let feeds = vec![&e, btc_feed(&e)];
            super::execute_update_status(&e, &feeds);

            let data_after = storage::get_market_data(&e, BTC_FEED_ID);

            // Longs were winning — should be reduced
            assert!(data_after.l_notional < data_before.l_notional);
            assert!(data_after.l_entry_wt < data_before.l_entry_wt);
            assert!(data_after.l_adl_idx < data_before.l_adl_idx);

            // Shorts were losing — should be unchanged
            assert_eq!(data_after.s_notional, data_before.s_notional);
            assert_eq!(data_after.s_entry_wt, data_before.s_entry_wt);
            assert_eq!(data_after.s_adl_idx, data_before.s_adl_idx);

            // Total notional updated
            let new_total = storage::get_total_notional(&e);
            assert_eq!(new_total, data_after.l_notional + data_after.s_notional);
        });
    }

    #[test]
    fn test_adl_heavy_reduction() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        // Tiny vault: 1_000 tokens
        let contract = setup_small_vault(&e, 1_000 * SCALAR_7);

        // Longs: 100k notional entered at $50k, current $100k => +100k PnL
        // Shorts: 10k entered at $50k => -10k PnL
        // Net = 90k, vault = 1k => massive deficit
        set_market_positions(&e, &contract, 100_000 * SCALAR_7, 10_000 * SCALAR_7, 50_000 * PRICE_SCALAR);

        e.as_contract(&contract, || {
            storage::set_status(&e, ContractStatus::OnIce as u32);

            let data_before = storage::get_market_data(&e, BTC_FEED_ID);

            let feeds = vec![&e, btc_feed(&e)];
            super::execute_update_status(&e, &feeds);

            let data_after = storage::get_market_data(&e, BTC_FEED_ID);

            // Longs heavily reduced
            assert!(data_after.l_notional < data_before.l_notional);
            assert!(data_after.l_adl_idx < SCALAR_18);

            // Shorts lost — untouched
            assert_eq!(data_after.s_notional, data_before.s_notional);
        });
    }
}
