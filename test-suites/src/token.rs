use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env};

pub fn create_stellar_token<'a>(e: &Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
    let address = e.register_stellar_asset_contract_v2(admin.clone()).address();
    let client = StellarAssetClient::new(e, &address);
    (address, client)
}
