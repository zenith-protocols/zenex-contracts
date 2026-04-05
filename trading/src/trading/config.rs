use crate::constants::MAX_ENTRIES;
use crate::errors::TradingError;
use crate::events::{DelMarket, SetConfig, SetMarket, SetStatus};
use crate::types::{ContractStatus, MarketConfig, TradingConfig};
use crate::validation::{require_valid_config, require_valid_market_config};
use crate::{storage, MarketData};
use soroban_sdk::{panic_with_error, Env};

/// Validate and store a new global trading configuration.
pub fn execute_set_config(e: &Env, config: &TradingConfig) {
    require_valid_config(e, config);
    storage::set_config(e, config);
    (SetConfig {}).publish(e);
}

/// Register a new market or update an existing market's configuration.
///
/// On first registration: initializes `MarketData` with zero OI, ADL indices at 1e18,
/// and `last_update` at current timestamp. Also seeds `last_funding_update` for the
/// first market to establish the funding cadence.
pub fn execute_set_market(e: &Env, feed_id: u32, config: &MarketConfig) {
    require_valid_market_config(e, config);

    let mut markets = storage::get_markets(e);
    let is_new = !markets.contains(feed_id);

    if is_new {
        if markets.len() >= MAX_ENTRIES {
            panic_with_error!(e, TradingError::MaxMarketsReached);
        }
        markets.push_back(feed_id);
        storage::set_markets(e, &markets);

        let initial_data = MarketData {
            last_update: e.ledger().timestamp(),
            ..Default::default()
        };
        storage::set_market_data(e, feed_id, &initial_data);
    }

    storage::set_market_config(e, feed_id, config);
    SetMarket { feed_id }.publish(e);
}

/// Remove a market. Subtracts remaining OI from total_notional and cleans up
/// market storage. Existing positions are refunded via cancel_position.
pub fn execute_del_market(e: &Env, feed_id: u32) {
    let mut markets = storage::get_markets(e);
    let idx = markets
        .iter()
        .position(|id| id == feed_id)
        .unwrap_or_else(|| panic_with_error!(e, TradingError::MarketNotFound));

    // Subtract this market's OI from total_notional
    let data = storage::get_market_data(e, feed_id);
    let market_notional = data.l_notional + data.s_notional;
    if market_notional > 0 {
        let total = storage::get_total_notional(e) - market_notional;
        storage::set_total_notional(e, total);
    }

    markets.remove(idx as u32);
    storage::set_markets(e, &markets);
    storage::remove_market_config(e, feed_id);
    storage::remove_market_data(e, feed_id);
    DelMarket { feed_id }.publish(e);
}

/// Admin-only status transitions (AdminOnIce, Frozen, Active from admin states).
/// Note: caller must already be authorized (e.g. via #[only_owner] on the contract method).
pub fn execute_set_status(e: &Env, status: u32) {
    let new_status = ContractStatus::from_u32(e, status);

    // Only admin-level statuses or restoring from admin states
    if new_status == ContractStatus::OnIce {
        panic_with_error!(e, TradingError::InvalidStatus);
    }

    storage::set_status(e, status);
    SetStatus { status }.publish(e);
}

#[cfg(test)]
mod tests {
    use crate::constants::SCALAR_18;
    use crate::storage;
    use crate::testutils::{
        create_trading, default_market, jump, FEED_BTC,
    };
    use crate::types::ContractStatus;
    use soroban_sdk::Env;

    #[test]
    fn test_constructor_initializes() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let (contract, _owner) = create_trading(&e);

        e.as_contract(&contract, || {
            assert_eq!(storage::get_status(&e), ContractStatus::Active as u32);
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
            new_config.caller_rate = 500_000;
            super::execute_set_config(&e, &new_config);

            let stored = storage::get_config(&e);
            assert_eq!(stored.caller_rate, 500_000);
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
            super::execute_set_market(&e, FEED_BTC, &market_config);

            let markets = storage::get_markets(&e);
            assert_eq!(markets.len(), 1);
            assert_eq!(markets.get(0).unwrap(), FEED_BTC);
            let stored = storage::get_market_config(&e, FEED_BTC);
            assert_eq!(stored.enabled, true);

            let data = storage::get_market_data(&e, FEED_BTC);
            assert_eq!(data.l_notional, 0);
            assert_eq!(data.s_notional, 0);
            assert_eq!(data.l_adl_idx, SCALAR_18);
            assert_eq!(data.s_adl_idx, SCALAR_18);
        });
    }

    #[test]
    fn test_del_market() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let (contract, _owner) = create_trading(&e);

        e.as_contract(&contract, || {
            let market_config = default_market(&e);
            super::execute_set_market(&e, FEED_BTC, &market_config);
            assert!(storage::has_market(&e, FEED_BTC));

            // Set OI to verify total_notional adjustment on deletion
            let mut data = storage::get_market_data(&e, FEED_BTC);
            data.l_notional = 10_000_000_000;
            data.s_notional = 5_000_000_000;
            storage::set_market_data(&e, FEED_BTC, &data);
            storage::set_total_notional(&e, 15_000_000_000);

            super::execute_del_market(&e, FEED_BTC);

            assert_eq!(storage::get_markets(&e).len(), 0);
            assert!(!storage::has_market(&e, FEED_BTC));
            assert_eq!(storage::get_total_notional(&e), 0);
        });
    }

    #[test]
    fn test_set_status() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let (contract, _owner) = create_trading(&e);
        let client = crate::TradingClient::new(&e, &contract);

        client.set_status(&(ContractStatus::AdminOnIce as u32));
        e.as_contract(&contract, || {
            assert_eq!(storage::get_status(&e), ContractStatus::AdminOnIce as u32);
        });

        client.set_status(&(ContractStatus::Active as u32));
        e.as_contract(&contract, || {
            assert_eq!(storage::get_status(&e), ContractStatus::Active as u32);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #740)")]
    fn test_set_status_onice_rejected() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let (contract, _owner) = create_trading(&e);
        let client = crate::TradingClient::new(&e, &contract);

        // OnIce is reserved for circuit breaker (update_status), admin can't set it directly
        client.set_status(&(ContractStatus::OnIce as u32));
    }

    #[test]
    fn test_set_market_enabled_toggle() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let (contract, _owner) = create_trading(&e);

        e.as_contract(&contract, || {
            let mut mc = default_market(&e);
            super::execute_set_market(&e, FEED_BTC, &mc);
            assert!(storage::get_market_config(&e, FEED_BTC).enabled);

            // Disable
            mc.enabled = false;
            super::execute_set_market(&e, FEED_BTC, &mc);
            assert!(!storage::get_market_config(&e, FEED_BTC).enabled);

            // Re-enable
            mc.enabled = true;
            super::execute_set_market(&e, FEED_BTC, &mc);
            assert!(storage::get_market_config(&e, FEED_BTC).enabled);
        });
    }
}
