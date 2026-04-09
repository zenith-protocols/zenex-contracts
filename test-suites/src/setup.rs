use crate::test_fixture::TestFixture;
use trading::testutils::{default_market, FEED_BTC, FEED_ETH, FEED_XLM};

pub fn create_fixture_with_data<'a>() -> TestFixture<'a> {
    let fixture = TestFixture::create();

    fixture.token.mint(&fixture.owner, &10_000_000_0000000);
    // ERC-4626 deposit(assets, receiver, from, operator)
    fixture
        .vault
        .deposit(&10_000_000_0000000, &fixture.owner, &fixture.owner, &fixture.owner);

    let base_config = default_market(&fixture.env);

    // Create markets identified by feed_id (no asset field needed)
    fixture.create_market(FEED_BTC, &base_config);
    fixture.create_market(FEED_ETH, &base_config);
    fixture.create_market(FEED_XLM, &base_config);

    // Contract starts Active from constructor, no need to set_status
    fixture
}

#[cfg(test)]
mod test {
    use super::*;

    /// Verify that factory deployment wires all contracts together correctly.
    #[test]
    fn test_fixture_creation() {
        let f = create_fixture_with_data();

        // Trading ↔ vault cross-references
        assert_eq!(f.trading.get_vault(), f.vault.address);
        assert_eq!(f.vault.query_asset(), f.token.address);

        // Trading → price verifier
        assert_eq!(f.trading.get_price_verifier(), f.price_verifier.address);

        // Factory tracks the deployment
        assert!(f.factory.is_deployed(&f.trading.address));

        // Vault has liquidity from setup
        assert_eq!(f.vault.total_assets(), 10_000_000_0000000);

        // Price verification works end-to-end
        let price_bytes = f.price_for_feed(FEED_BTC, 10_000_000_000_000);
        let price_data = f.price_verifier.verify_price(&price_bytes);
        assert_eq!(price_data.feed_id, FEED_BTC);
        assert_eq!(price_data.price, 10_000_000_000_000_i128);
    }
}
