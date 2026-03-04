use crate::dependencies::vault::{VaultClient, VAULT_WASM};
use crate::token::create_stellar_token;
use sep_40_oracle::testutils::{Asset, MockPriceOracleClient, MockPriceOracleWASM};
use sep_40_oracle::Asset as StellarAsset;
use sep_41_token::testutils::MockTokenClient;
use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{vec as svec, Address, Env, String, Symbol, Vec as SorobanVec};
use std::ops::Index;
use trading::{ExecuteRequest, MarketConfig, TradingClient};

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum AssetIndex {
    BTC = 0,
    ETH = 1,
    XLM = 2,
}

// Index implementation for Vec<StellarAsset> using AssetIndex
impl Index<AssetIndex> for Vec<StellarAsset> {
    type Output = StellarAsset;

    fn index(&self, index: AssetIndex) -> &Self::Output {
        &self[index as usize]
    }
}

pub struct TestFixture<'a> {
    pub env: Env,
    pub owner: Address,
    pub users: Vec<Address>,
    pub vault: VaultClient<'a>,
    pub trading: TradingClient<'a>,
    pub oracle: MockPriceOracleClient<'a>,
    pub token: MockTokenClient<'a>,
    pub assets: Vec<StellarAsset>, // Now using StellarAsset
}

impl TestFixture<'_> {
    pub fn create<'a>(wasm: bool) -> TestFixture<'a> {
        let e = Env::default();
        e.mock_all_auths();

        let owner = Address::generate(&e);

        let (token_id, token_client) = create_stellar_token(&e, &owner);

        let oracle_id = e.register(MockPriceOracleWASM, ());
        let oracle_client = MockPriceOracleClient::new(&e, &oracle_id);

        // Create StellarAssets in order matching AssetIndex
        let assets = vec![
            StellarAsset::Other(Symbol::new(&e, "BTC")), // AssetIndex::BTC = 0
            StellarAsset::Other(Symbol::new(&e, "ETH")), // AssetIndex::ETH = 1
            StellarAsset::Other(Symbol::new(&e, "XLM")), // AssetIndex::XLM = 2
        ];

        oracle_client.set_data(
            &owner,
            &Asset::Other(Symbol::new(&e, "USD")),
            &svec![
                &e,
                Asset::Stellar(token_id.clone()),
                Asset::Other(Symbol::new(&e, "BTC")),
                Asset::Other(Symbol::new(&e, "ETH")),
                Asset::Other(Symbol::new(&e, "XLM")),
            ],
            &7,
            &300,
        );

        oracle_client.set_price_stable(&svec![
            &e,
            1_0000000,       // 1 USD
            100_000_0000000, // BTC = 100K
            2000_0000000,    // ETH = 2K
            0_1000000,       // XLM = 0.1
        ]);

        let trading_args = (owner.clone(),);
        let trading_id = if wasm {
            e.register(
                crate::dependencies::trading::trading_contract_wasm::WASM,
                trading_args,
            )
        } else {
            e.register(trading::TradingContract {}, trading_args)
        };
        let trading_client = TradingClient::new(&e, &trading_id);

        let strategies = soroban_sdk::Vec::from_array(&e, [trading_id.clone(), owner.clone()]);
        let vault_args = (
            String::from_str(&e, "Vault Shares"), // name: String
            String::from_str(&e, "VSHR"),         // symbol: String
            token_id.clone(),                     // asset: Address
            0u32,                                 // decimals_offset: u32
            strategies,                           // strategies: Vec<Address>
            300u64,                               // lock_time: u64 (5 minutes)
        );
        let vault_id = e.register(VAULT_WASM, vault_args);
        let vault_client = VaultClient::new(&e, &vault_id);

        let config = trading::TradingConfig {
            caller_take_rate: 0,
            min_open_time: 0,
            vault_skim: 0_2000000,       // 20%
            min_collateral: 10_000_000,             // 1 token minimum (SCALAR_7)
            max_collateral: 1_000_000 * 10_000_000, // 1M tokens maximum
            max_payout: 10 * 10_000_000,            // 1000% max payout
            base_fee_dominant: 0_0005000,     // 0.05%
            base_fee_non_dominant: 0_0001000, // 0.01%
        };
        // Set the vault in trading contract
        // After initialize, status is Setup (99) which allows market queuing without delay
        trading_client.initialize(&String::from_str(&e, "Zenex"), &vault_id, &oracle_id, &config);

        let fixture = TestFixture {
            env: e,
            owner,
            users: vec![],
            vault: vault_client,
            trading: trading_client,
            oracle: oracle_client,
            token: token_client,
            assets,
        };
        fixture
    }

    pub fn create_market(&mut self, config: &MarketConfig) {
        self.trading.set_market(config);
    }

    pub fn position_exists(&self, position_id: u32) -> bool {
        self.env.as_contract(&self.trading.address, || {
            self.env
                .storage()
                .persistent()
                .has(&trading::storage::TradingStorageKey::Position(position_id))
        })
    }

    /// Place a limit order and immediately fill it via keeper execute.
    /// Equivalent to the old "market order" pattern. Returns (position_id, open_fee).
    pub fn open_and_fill(
        &self,
        user: &Address,
        asset_index: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
        take_profit: i128,
        stop_loss: i128,
    ) -> (u32, i128) {
        let (id, fee) = self.trading.open_position(
            user, &asset_index, &collateral, &notional_size, &is_long,
            &entry_price, &take_profit, &stop_loss,
        );
        let requests = SorobanVec::from_array(
            &self.env,
            [ExecuteRequest { request_type: 0, position_id: id }], // Fill = 0
        );
        self.trading.execute(user, &requests);
        (id, fee)
    }

    /********** Chain Helpers ***********/

    pub fn jump(&self, time: u64) {
        self.env.ledger().set(LedgerInfo {
            timestamp: self.env.ledger().timestamp().saturating_add(time),
            protocol_version: 25,
            sequence_number: self.env.ledger().sequence(),
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 999999,
            min_persistent_entry_ttl: 999999,
            max_entry_ttl: 9999999,
        });
    }

    pub fn jump_with_sequence(&self, time: u64) {
        let blocks = time / 5;
        self.env.ledger().set(LedgerInfo {
            timestamp: self.env.ledger().timestamp().saturating_add(time),
            protocol_version: 25,
            sequence_number: self.env.ledger().sequence().saturating_add(blocks as u32),
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 999999,
            min_persistent_entry_ttl: 999999,
            max_entry_ttl: 9999999,
        });
    }

}
