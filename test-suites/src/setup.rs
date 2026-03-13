use crate::test_fixture::{TestFixture, ETH_FEED_ID, XLM_FEED_ID};
use trading::testutils::{default_market, BTC_FEED_ID};

pub fn create_fixture_with_data<'a>() -> TestFixture<'a> {
    let fixture = TestFixture::create();

    fixture.token.mint(&fixture.owner, &100_000_000_0000000);
    // ERC-4626 deposit(assets, receiver, from, operator)
    fixture.vault.deposit(&100_000_000_0000000, &fixture.owner, &fixture.owner, &fixture.owner);

    let base_config = default_market(&fixture.env);

    // Create markets identified by feed_id (no asset field needed)
    fixture.create_market(BTC_FEED_ID, &base_config);
    fixture.create_market(ETH_FEED_ID, &base_config);
    fixture.create_market(XLM_FEED_ID, &base_config);

    // Contract starts Active from constructor, no need to set_status
    fixture
}

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;
    use super::*;

    #[test]
    fn test_create_fixture_with_data() {
        let fixture: TestFixture<'_> = create_fixture_with_data();
        let _freek = Address::generate(&fixture.env);
    }
}
