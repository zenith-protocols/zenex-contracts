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
