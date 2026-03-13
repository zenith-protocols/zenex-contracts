use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::{TreasuryClient, TreasuryContract};

fn setup() -> (Env, TreasuryClient<'static>, Address) {
    let e = Env::default();
    e.mock_all_auths();
    let owner = Address::generate(&e);
    let contract_id = e.register(TreasuryContract, (&owner, &1_000_000i128));
    let client = TreasuryClient::new(&e, &contract_id);
    (e, client, owner)
}

#[test]
fn test_get_rate() {
    let (_e, client, _owner) = setup();
    assert_eq!(client.get_rate(), 1_000_000);
}

#[test]
fn test_set_rate() {
    let (_e, client, _owner) = setup();
    client.set_rate(&2_000_000);
    assert_eq!(client.get_rate(), 2_000_000);
}

#[test]
fn test_withdraw() {
    let (e, client, owner) = setup();
    let token_admin = Address::generate(&e);
    let token_id = e.register_stellar_asset_contract_v2(token_admin.clone());
    let token_client = sep_41_token::StellarAssetClient::new(&e, &token_id.address());

    // Mint tokens to the treasury contract
    token_client.mint(&client.address, &1_000);

    // Withdraw to owner
    client.withdraw(&token_id.address(), &owner, &500);

    let balance_client = soroban_sdk::token::TokenClient::new(&e, &token_id.address());
    assert_eq!(balance_client.balance(&owner), 500);
    assert_eq!(balance_client.balance(&client.address), 500);
}
