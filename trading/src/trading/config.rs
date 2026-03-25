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

        if markets.len() == 1 {
            storage::set_last_funding_update(e, e.ledger().timestamp());
        }
    }

    storage::set_market_config(e, feed_id, config);
    SetMarket { feed_id }.publish(e);
}

/// Remove a market. Panics if any open interest remains.
pub fn execute_del_market(e: &Env, feed_id: u32) {
    let mut markets = storage::get_markets(e);
    let idx = markets
        .iter()
        .position(|id| id == feed_id)
        .unwrap_or_else(|| panic_with_error!(e, TradingError::MarketNotFound));

    let data = storage::get_market_data(e, feed_id);
    if data.l_notional != 0 || data.s_notional != 0 {
        panic_with_error!(e, TradingError::MarketHasOpenPositions);
    }

    markets.remove(idx as u32);
    storage::set_markets(e, &markets);
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
        create_trading, default_market, jump, BTC_FEED_ID,
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
            super::execute_set_market(&e, BTC_FEED_ID, &market_config);

            let markets = storage::get_markets(&e);
            assert_eq!(markets.len(), 1);
            assert_eq!(markets.get(0).unwrap(), BTC_FEED_ID);
            let stored = storage::get_market_config(&e, BTC_FEED_ID);
            assert_eq!(stored.enabled, market_config.enabled);

            let data = storage::get_market_data(&e, BTC_FEED_ID);
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
            super::execute_set_market(&e, BTC_FEED_ID, &market_config);
            assert_eq!(storage::get_markets(&e).len(), 1);

            super::execute_del_market(&e, BTC_FEED_ID);
            assert_eq!(storage::get_markets(&e).len(), 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #771)")]
    fn test_del_market_with_open_positions() {
        let e = Env::default();
        e.mock_all_auths();
        jump(&e, 1000);

        let (contract, _owner) = create_trading(&e);

        e.as_contract(&contract, || {
            let market_config = default_market(&e);
            super::execute_set_market(&e, BTC_FEED_ID, &market_config);

            let mut data = storage::get_market_data(&e, BTC_FEED_ID);
            data.l_notional = 10_000_000_000;
            storage::set_market_data(&e, BTC_FEED_ID, &data);

            super::execute_del_market(&e, BTC_FEED_ID);
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
}
