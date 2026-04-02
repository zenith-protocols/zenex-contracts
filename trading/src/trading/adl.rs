use crate::constants::{SCALAR_7, SCALAR_18, UTIL_ACTIVE, UTIL_ONICE};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::events::{ADLMarket, ADLTriggered, SetStatus};
use crate::storage;
use crate::dependencies::{scalar_from_exponent, PriceData};
use crate::types::{ContractStatus, MarketData};
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::{panic_with_error, Env, Map, Vec};

/// Permissionless circuit breaker and auto-deleveraging (ADL) trigger.
///
/// Computes net trader PnL across all markets using entry-weighted aggregates
/// per market. Based on current status:
///
/// - **Active**: PnL >= 95% → set `OnIce`; PnL > 100% → also run ADL
/// - **OnIce**: PnL < 90% → restore `Active`; PnL > 100% → run ADL
/// - **AdminOnIce**: PnL > 100% → run ADL (status stays AdminOnIce)
/// - **Frozen**: panics (admin's nuclear option)
///
/// # Parameters
/// - `feeds` - Verified price data for ALL registered markets (must match length)
///
/// # Panics
/// - `TradingError::ThresholdNotMet` (780) if PnL below trigger threshold
/// - `TradingError::InvalidStatus` (760) if contract is Frozen
/// - `TradingError::InvalidPrice` (720) if feeds length doesn't match markets
pub fn execute_update_status(e: &Env, feeds: &Vec<PriceData>) {
    let current = ContractStatus::from_u32(e, storage::get_status(e));
    let vault = storage::get_vault(e);
    let markets = storage::get_markets(e);
    let vault_balance = VaultClient::new(e, &vault).total_assets();

    // Ensure feeds cover all markets; mismatch means missing feeds.
    if feeds.len() != markets.len() {
        panic_with_error!(e, TradingError::InvalidPrice);
    }

    let mut cached: Map<u32, (MarketData, i128, i128)> = Map::new(e);
    let mut net_pnl: i128 = 0;
    let mut total_winner_pnl: i128 = 0;

    for f in feeds.iter() {
        let data = storage::get_market_data(e, f.feed_id);
        let ps = scalar_from_exponent(f.exponent);

        let long_pnl = f.price.fixed_mul_floor(e, &data.l_entry_wt, &ps) - data.l_notional;
        let short_pnl = data.s_notional - f.price.fixed_mul_floor(e, &data.s_entry_wt, &ps);

        net_pnl += long_pnl + short_pnl;
        if long_pnl > 0 { total_winner_pnl += long_pnl; }
        if short_pnl > 0 { total_winner_pnl += short_pnl; }

        cached.set(f.feed_id, (data, long_pnl, short_pnl));
    }

    // Duplicates collapse in the map; length mismatch means duplicate feeds
    if cached.len() != markets.len() {
        panic_with_error!(e, TradingError::InvalidPrice);
    }

    match current {
        ContractStatus::Active => {
            let onice_line = vault_balance.fixed_mul_floor(e, &UTIL_ONICE, &SCALAR_7);
            if net_pnl < onice_line {
                panic_with_error!(e, TradingError::ThresholdNotMet);
            }
            // >= 95%: set OnIce. > 100%: also run ADL.
            if net_pnl > vault_balance {
                do_adl(e, &cached, total_winner_pnl, net_pnl, vault_balance);
            }
            storage::set_status(e, ContractStatus::OnIce as u32);
            SetStatus { status: ContractStatus::OnIce as u32 }.publish(e);
        }
        ContractStatus::OnIce => {
            let active_line = vault_balance.fixed_mul_floor(e, &UTIL_ACTIVE, &SCALAR_7);
            if net_pnl < active_line {
                storage::set_status(e, ContractStatus::Active as u32);
                SetStatus { status: ContractStatus::Active as u32 }.publish(e);
            } else if net_pnl > vault_balance {
                do_adl(e, &cached, total_winner_pnl, net_pnl, vault_balance);
            } else {
                panic_with_error!(e, TradingError::ThresholdNotMet);
            }
        }
        ContractStatus::AdminOnIce => {
            if net_pnl > vault_balance {
                do_adl(e, &cached, total_winner_pnl, net_pnl, vault_balance);
            } else {
                panic_with_error!(e, TradingError::ThresholdNotMet);
            }
        }
        _ => panic_with_error!(e, TradingError::InvalidStatus),
    }
}

/// Reduce winning-side notionals proportionally to bring net PnL within vault capacity.
///
/// Computes `reduction_pct = deficit / total_winner_pnl`, then applies
/// `factor = 1 - reduction_pct` to each winning side's notional, entry_wt, and ADL index.
///
/// Entry-weighted aggregate (`entry_wt`) enables O(1) per-market PnL calculation:
/// `side_pnl = current_price * entry_wt - notional` for longs. Without it, we'd need
/// to iterate every position on-chain, which is infeasible in a single transaction.
///
/// ADL applies to the index (not individual positions) so that individual positions
/// are lazily adjusted at their next settlement. This avoids iterating all positions.
///
/// # Funding residual (known, accepted)
///
/// ADL reduces notional but does NOT adjust funding/borrowing indices. When an
/// ADL'd position later settles, its funding is computed as `reduced_notional ×
/// full_index_delta`, which under-counts funding accrued during the pre-ADL period
/// (when the position had larger notional). The shortfall is:
///
///   `original_notional × (1 - factor) × pre_adl_fund_delta`
///
/// This is accepted because:
/// 1. Funding rates are small (~0.001%/hr), so the gap is negligible vs vault TVL.
/// 2. The vault already acts as funding counterparty (all funding flows through
///    vault_transfer on settlement), so the vault implicitly absorbs the residual.
/// 3. ADL only triggers under extreme stress (net PnL > vault), and the funding
///    residual is orders of magnitude smaller than the PnL deficit that caused ADL.
/// 4. Splitting the calculation at the ADL boundary is infeasible without per-position
///    iteration or additional storage for index checkpoints at each ADL event.
fn do_adl(
    e: &Env,
    cached: &Map<u32, (MarketData, i128, i128)>,
    total_winner_pnl: i128,
    net_pnl: i128,
    vault_balance: i128,
) {
    let deficit = net_pnl - vault_balance;
    let reduction_pct = deficit.fixed_div_floor(e, &total_winner_pnl, &SCALAR_18);
    let reduction_pct = reduction_pct.min(SCALAR_18);
    let factor = SCALAR_18 - reduction_pct;

    let trading_config = storage::get_config(e);

    // Compute total notional from cached data for vault-level utilization
    let total_notional: i128 = cached
        .iter()
        .map(|(_, (d, _, _))| d.l_notional + d.s_notional)
        .sum();

    let mut new_total: i128 = 0;
    for (feed_id, (mut data, long_pnl, short_pnl)) in cached.iter() {
        // Accrue indices against pre-ADL notionals before reducing them
        let config = storage::get_market_config(e, feed_id);
        data.accrue(
            e,
            trading_config.r_base,
            trading_config.r_var,
            config.r_var_market,
            vault_balance,
            total_notional,
            trading_config.max_util,
            config.max_util,
        );

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
    #[should_panic(expected = "Error(Contract, #750)")]
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
        // reduction_pct = floor(15k / 50k × S18) = 0.3 × S18
        // factor = 0.7 × S18
        set_market_positions(&e, &contract, 50_000 * SCALAR_7, 30_000 * SCALAR_7, 50_000 * PRICE_SCALAR);

        e.as_contract(&contract, || {
            storage::set_status(&e, ContractStatus::OnIce as u32);

            let data_before = storage::get_market_data(&e, BTC_FEED_ID);

            let feeds = vec![&e, btc_feed(&e)];
            super::execute_update_status(&e, &feeds);

            let data_after = storage::get_market_data(&e, BTC_FEED_ID);

            // Longs reduced by 30%: 50k × 0.7 = 35k
            assert_eq!(data_after.l_notional, 350_000_000_000);
            let expected_ew = data_before.l_entry_wt * 700_000_000_000_000_000 / SCALAR_18;
            assert_eq!(data_after.l_entry_wt, expected_ew);
            assert_eq!(data_after.l_adl_idx, 700_000_000_000_000_000);

            // Shorts were losing — untouched
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
        // Net = 90k, vault = 1k => deficit = 89k
        // total_winner_pnl = 100k (longs)
        // reduction_pct = floor(89k / 100k × S18) = 0.89 × S18
        // factor = 0.11 × S18
        set_market_positions(&e, &contract, 100_000 * SCALAR_7, 10_000 * SCALAR_7, 50_000 * PRICE_SCALAR);

        e.as_contract(&contract, || {
            storage::set_status(&e, ContractStatus::OnIce as u32);

            let data_before = storage::get_market_data(&e, BTC_FEED_ID);

            let feeds = vec![&e, btc_feed(&e)];
            super::execute_update_status(&e, &feeds);

            let data_after = storage::get_market_data(&e, BTC_FEED_ID);

            // Longs reduced by 89%: 100k × 0.11 = 11k
            assert_eq!(data_after.l_notional, 110_000_000_000);
            assert_eq!(data_after.l_adl_idx, 110_000_000_000_000_000);

            // Shorts lost — untouched
            assert_eq!(data_after.s_notional, data_before.s_notional);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #740)")]
    fn test_update_status_frozen_rejected() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let contract = setup_small_vault(&e, 100 * SCALAR_7);
        set_market_positions(&e, &contract, 1000 * SCALAR_7, 0, 50_000 * PRICE_SCALAR);

        e.as_contract(&contract, || {
            storage::set_status(&e, ContractStatus::Frozen as u32);

            let feeds = vec![&e, btc_feed(&e)];
            super::execute_update_status(&e, &feeds);
        });
    }

    #[test]
    fn test_update_status_admin_onice_adl() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let contract = setup_small_vault(&e, 100 * SCALAR_7);
        set_market_positions(&e, &contract, 1000 * SCALAR_7, 0, 50_000 * PRICE_SCALAR);

        e.as_contract(&contract, || {
            storage::set_status(&e, ContractStatus::AdminOnIce as u32);

            let feeds = vec![&e, btc_feed(&e)];
            super::execute_update_status(&e, &feeds);

            // ADL runs but status stays AdminOnIce (admin controls the unlock)
            assert_eq!(storage::get_status(&e), ContractStatus::AdminOnIce as u32);

            let data = storage::get_market_data(&e, BTC_FEED_ID);
            assert!(data.l_adl_idx < SCALAR_18);
        });
    }
}
