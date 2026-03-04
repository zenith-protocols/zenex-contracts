use crate::constants::MAX_PRICE_AGE;
use crate::errors::TradingError;
use sep_40_oracle::{Asset, PriceFeedClient};
use soroban_sdk::{panic_with_error, Address, Env};

/// Get the price scalar (10^decimals) from the oracle
pub fn get_price_scalar(e: &Env, oracle: &Address) -> i128 {
    let decimals = PriceFeedClient::new(e, oracle).decimals();
    10i128.pow(decimals)
}

/// Load the current price for an asset from the oracle
pub fn load_price(e: &Env, oracle: &Address, asset: &Asset) -> i128 {
    let price_data = match PriceFeedClient::new(e, oracle).lastprice(asset) {
        Some(price) => price,
        None => panic_with_error!(e, TradingError::PriceNotFound),
    };
    if price_data.timestamp + (MAX_PRICE_AGE as u64) < e.ledger().timestamp() {
        panic_with_error!(e, TradingError::PriceStale);
    }
    price_data.price
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils::{create_oracle, setup_env, BTC_PRICE};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{contract, contractimpl, Symbol};

    /// Test oracle that only knows BTC and always returns timestamp=0 (stale).
    /// - Query BTC → Some(price) with timestamp=0
    /// - Query anything else → None
    #[contract]
    struct TestOracle;

    #[contractimpl]
    impl TestOracle {
        pub fn lastprice(_e: Env, asset: Asset) -> Option<sep_40_oracle::PriceData> {
            match asset {
                Asset::Other(ref sym) if *sym == Symbol::new(&_e, "BTC") => {
                    Some(sep_40_oracle::PriceData {
                        price: BTC_PRICE,
                        timestamp: 0,
                    })
                }
                _ => None,
            }
        }
    }

    #[test]
    fn test_load_price_success() {
        let e = setup_env();
        let (oracle, _) = create_oracle(&e);
        let asset = Asset::Other(Symbol::new(&e, "BTC"));
        let caller = Address::generate(&e);

        let contract = e.register(crate::contract::TradingContract {}, (caller,));
        e.as_contract(&contract, || {
            let price = load_price(&e, &oracle, &asset);
            assert_eq!(price, BTC_PRICE);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #720)")]
    fn test_load_price_not_found() {
        let e = setup_env();
        let oracle = e.register(TestOracle, ());
        let asset = Asset::Other(Symbol::new(&e, "ETH"));
        let caller = Address::generate(&e);

        let contract = e.register(crate::contract::TradingContract {}, (caller,));
        e.as_contract(&contract, || {
            load_price(&e, &oracle, &asset);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #721)")]
    fn test_load_price_stale() {
        let e = setup_env();
        let oracle = e.register(TestOracle, ());
        let asset = Asset::Other(Symbol::new(&e, "BTC"));
        let caller = Address::generate(&e);

        let contract = e.register(crate::contract::TradingContract {}, (caller,));
        e.as_contract(&contract, || {
            // timestamp=0, MAX_PRICE_AGE=900, ledger=1000 → 0 + 900 < 1000 → stale
            load_price(&e, &oracle, &asset);
        });
    }
}
