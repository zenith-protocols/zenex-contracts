use crate::test_fixture::{AssetIndex, TestFixture};
use trading::testutils::default_market;

pub fn create_fixture_with_data<'a>(wasm: bool) -> TestFixture<'a> {
    let mut fixture = TestFixture::create(wasm);

    fixture.token.mint(&fixture.admin, &100_000_000_0000000);
    fixture.vault.deposit(&100_000_000_0000000, &fixture.admin);

    let mut market_config = default_market();
    market_config.total_available = 100_000_000_0000000; // 100M tokens available from vault

    // Extract the assets before the mutable borrows
    let btc_asset = fixture.assets[AssetIndex::BTC].clone();
    let eth_asset = fixture.assets[AssetIndex::ETH].clone();
    let xlm_asset = fixture.assets[AssetIndex::XLM].clone();

    fixture.create_market(&btc_asset, market_config.clone());
    fixture.create_market(&eth_asset, market_config.clone());
    fixture.create_market(&xlm_asset, market_config.clone());

    fixture.trading.set_status(&0u32);

    fixture
}

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;
    use crate::SCALAR_7;
    use super::*;

    #[test]
    fn test_create_fixture_with_data() {
        let fixture: TestFixture<'_> = create_fixture_with_data(false);
        let freek = Address::generate(&fixture.env);
    }
}