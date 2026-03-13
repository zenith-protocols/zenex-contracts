use crate::constants::{MAX_MARKETS, MAX_STALENESS_KEEPER, SCALAR_7, UTIL_FREEZE, UTIL_UNFREEZE};
use crate::dependencies::VaultClient;
use crate::errors::TradingError;
use crate::trading::adl::do_adl;
use crate::trading::price_verifier::{check_staleness, scalar_from_exponent, PriceData};
use soroban_fixed_point_math::SorobanFixedPoint;
use crate::events::{SetConfig, SetMarket, SetStatus};
use crate::types::{ContractStatus, MarketConfig, TradingConfig};
use crate::validation::{require_valid_config, require_valid_market_config};
use crate::{storage, MarketData};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};
use soroban_sdk::unwrap::UnwrapOptimized;
use stellar_access::ownable;

pub fn execute_set_config(e: &Env, config: &TradingConfig) {
    require_valid_config(e, config);
    storage::set_config(e, config);
    (SetConfig {}).publish(e);
}

pub fn execute_set_market(e: &Env, feed_id: u32, config: &MarketConfig) {
    require_valid_market_config(e, config);

    let mut markets = storage::get_markets(e);
    let is_new = !markets.contains(&feed_id);

    if is_new {
        if markets.len() >= MAX_MARKETS {
            panic_with_error!(e, TradingError::MaxMarketsReached);
        }
        markets.push_back(feed_id);
        storage::set_markets(e, &markets);

        let mut initial_data = MarketData::default();
        initial_data.last_update = e.ledger().timestamp();
        storage::set_market_data(e, feed_id, &initial_data);

        if markets.len() == 1 {
            storage::set_last_funding_update(e, e.ledger().timestamp());
        }
    }

    storage::set_market_config(e, feed_id, config);
    SetMarket { feed_id }.publish(e);
}

/// Admin-only status transitions (AdminOnIce, Frozen, Active from admin states).
/// Note: caller must already be authorized (e.g. via #[only_owner] on the contract method).
pub fn execute_set_status(e: &Env, status: u32) {
    let new_status = ContractStatus::from_u32(e, status);

    // Only admin-level statuses or restoring from admin states
    match new_status {
        ContractStatus::OnIce => panic_with_error!(e, TradingError::InvalidStatus),
        _ => {}
    }

    storage::set_status(e, status);
    SetStatus { status }.publish(e);
}

/// Permissionless status update based on price data.
/// - Active: if utilization threshold met -> OnIce
/// - OnIce: if threshold dropped -> Active, else if deficit -> ADL
pub fn execute_update_status(e: &Env, feeds: &Vec<PriceData>) {
    let current = ContractStatus::from_u32(e, storage::get_status(e));

    let vault = storage::get_vault(e);
    let markets = storage::get_markets(e);

    // Build price map: feed_id → (price, price_scalar)
    let mut price_map: Map<u32, (i128, i128)> = Map::new(e);
    for feed in feeds.iter() {
        check_staleness(e, feed.publish_time, MAX_STALENESS_KEEPER);
        price_map.set(feed.feed_id, (feed.price, scalar_from_exponent(feed.exponent)));
    }

    let mut net_pnl: i128 = 0;
    for feed_id in markets.iter() {
        let data = storage::get_market_data(e, feed_id);
        let (price, ps) = price_map.get(feed_id).unwrap();

        let long_pnl = price.fixed_mul_floor(e, &data.long_entry_weighted, &ps)
            - data.long_notional_size;
        let short_pnl = data.short_notional_size
            - price.fixed_mul_floor(e, &data.short_entry_weighted, &ps);

        net_pnl += long_pnl + short_pnl;
    }

    let vault_balance = VaultClient::new(e, &vault).total_assets();

    match current {
        ContractStatus::Active => {
            let freeze_line = vault_balance.fixed_mul_floor(e, &UTIL_FREEZE, &SCALAR_7);
            if net_pnl < freeze_line {
                panic_with_error!(e, TradingError::ThresholdNotMet);
            }
            storage::set_status(e, ContractStatus::OnIce as u32);
            SetStatus { status: ContractStatus::OnIce as u32 }.publish(e);
        }
        ContractStatus::OnIce => {
            let unfreeze_line = vault_balance.fixed_mul_floor(e, &UTIL_UNFREEZE, &SCALAR_7);
            if net_pnl < unfreeze_line {
                storage::set_status(e, ContractStatus::Active as u32);
                SetStatus { status: ContractStatus::Active as u32 }.publish(e);
            } else {
                do_adl(e, &markets, &price_map);
            }
        }
        _ => panic_with_error!(e, TradingError::InvalidStatus),
    }
}

#[cfg(test)]
mod tests {
    use crate::constants::{SCALAR_7, SCALAR_18};
    use crate::storage;
    use crate::testutils::{
        create_trading, create_trading_with_vault, default_market,
        default_market_data, jump, BTC_FEED_ID, BTC_PRICE, PRICE_SCALAR,
    };
    use crate::trading::price_verifier::PriceData;
    use crate::types::ContractStatus;
    use soroban_sdk::{vec, Env};

    #[test]
    fn test_constructor_initializes() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let (contract, _owner) = create_trading(&e);

        e.as_contract(&contract, || {
            assert_eq!(storage::get_status(&e), ContractStatus::Active as u32);
            // Verify storage was populated by constructor
            let _ = storage::get_vault(&e);
            let _ = storage::get_price_verifier(&e);
            let _ = storage::get_token(&e);
            let _ = storage::get_config(&e);
        });
    }

    #[test]
    fn test_set_config() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let (contract, _owner) = create_trading(&e);

        e.as_contract(&contract, || {
            let mut new_config = crate::testutils::default_config();
            new_config.caller_take_rate = 0_0500000; // 5%
            super::execute_set_config(&e, &new_config);

            let stored = storage::get_config(&e);
            assert_eq!(stored.caller_take_rate, 0_0500000);
        });
    }

    #[test]
    fn test_set_market() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let (contract, _owner) = create_trading(&e);

        e.as_contract(&contract, || {
            let market_config = default_market(&e);
            super::execute_set_market(&e, BTC_FEED_ID, &market_config);

            let markets = storage::get_markets(&e);
            assert_eq!(markets.len(), 1);
            assert_eq!(markets.get(0).unwrap(), BTC_FEED_ID);
            let stored = storage::get_market_config(&e, BTC_FEED_ID);
            assert_eq!(stored.enabled, market_config.enabled);

            let data = storage::get_market_data(&e, BTC_FEED_ID);
            assert_eq!(data.long_notional_size, 0);
            assert_eq!(data.short_notional_size, 0);
            assert_eq!(data.long_adl_index, SCALAR_18);
            assert_eq!(data.short_adl_index, SCALAR_18);
        });
    }

    #[test]
    fn test_set_status() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let (contract, _owner) = create_trading(&e);
        let client = crate::TradingClient::new(&e, &contract);

        // Admin can set to AdminOnIce
        client.set_status(&(ContractStatus::AdminOnIce as u32));
        e.as_contract(&contract, || {
            assert_eq!(storage::get_status(&e), ContractStatus::AdminOnIce as u32);
        });

        // Admin can set back to Active from AdminOnIce
        client.set_status(&(ContractStatus::Active as u32));
        e.as_contract(&contract, || {
            assert_eq!(storage::get_status(&e), ContractStatus::Active as u32);
        });
    }

    #[test]
    fn test_update_status_active_to_on_ice() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        // Very small vault so net PnL easily exceeds 90%
        let (contract, _owner) = create_trading_with_vault(&e, 100 * SCALAR_7);

        e.as_contract(&contract, || {
            storage::set_status(&e, ContractStatus::Active as u32);

            let market_config = default_market(&e);
            super::execute_set_market(&e, BTC_FEED_ID, &market_config);

            // Create a market state where net PnL >= 90% of vault
            let mut data = default_market_data();
            data.last_update = e.ledger().timestamp();
            data.long_notional_size = 1000 * SCALAR_7;
            data.long_entry_weighted = 1000 * SCALAR_7 * PRICE_SCALAR / (50_000 * PRICE_SCALAR);
            storage::set_market_data(&e, BTC_FEED_ID, &data);

            let feeds = vec![&e, PriceData {
                feed_id: BTC_FEED_ID,
                price: BTC_PRICE,
                exponent: -8,
                publish_time: e.ledger().timestamp(),
            }];
            super::execute_update_status(&e, &feeds);
            assert_eq!(storage::get_status(&e), ContractStatus::OnIce as u32);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #782)")]
    fn test_update_status_threshold_not_met() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        // Large vault relative to positions
        let (contract, _owner) = create_trading_with_vault(&e, 100_000_000 * SCALAR_7);

        e.as_contract(&contract, || {
            storage::set_status(&e, ContractStatus::Active as u32);

            let market_config = default_market(&e);
            super::execute_set_market(&e, BTC_FEED_ID, &market_config);

            // Small positions, net PnL is tiny relative to vault
            let mut data = default_market_data();
            data.last_update = e.ledger().timestamp();
            data.long_notional_size = 100 * SCALAR_7;
            data.long_entry_weighted = 100 * SCALAR_7 * PRICE_SCALAR / (100_000 * PRICE_SCALAR);
            storage::set_market_data(&e, BTC_FEED_ID, &data);

            let feeds = vec![&e, PriceData {
                feed_id: BTC_FEED_ID,
                price: BTC_PRICE,
                exponent: -8,
                publish_time: e.ledger().timestamp(),
            }];
            super::execute_update_status(&e, &feeds);
        });
    }
}
