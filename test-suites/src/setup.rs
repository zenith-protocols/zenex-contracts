use crate::test_fixture::{AssetIndex, TestFixture};
use trading::testutils::default_market;

pub fn create_fixture_with_data<'a>(wasm: bool) -> TestFixture<'a> {
    let mut fixture = TestFixture::create(wasm);

    fixture.token.mint(&fixture.owner, &100_000_000_0000000);
    // ERC-4626 deposit(assets, receiver, from, operator)
    fixture.vault.deposit(&100_000_000_0000000, &fixture.owner, &fixture.owner, &fixture.owner);

    let base_config = default_market(&fixture.env);

    // Create market configs with correct assets
    let btc_config = trading::MarketConfig {
        asset: fixture.assets[AssetIndex::BTC].clone(),
        ..base_config.clone()
    };
    let eth_config = trading::MarketConfig {
        asset: fixture.assets[AssetIndex::ETH].clone(),
        ..base_config.clone()
    };
    let xlm_config = trading::MarketConfig {
        asset: fixture.assets[AssetIndex::XLM].clone(),
        ..base_config.clone()
    };

    fixture.create_market(&btc_config);
    fixture.create_market(&eth_config);
    fixture.create_market(&xlm_config);

    // Set status to Active (0) so tests can open positions
    // Status values: 0=Active, 1=OnIce, 2=Frozen, 99=Setup
    fixture.trading.set_status(&0u32);

    fixture
}

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;
    use super::*;

    #[test]
    fn test_create_fixture_with_data() {
        let fixture: TestFixture<'_> = create_fixture_with_data(false);
        let _freek = Address::generate(&fixture.env);
    }
}