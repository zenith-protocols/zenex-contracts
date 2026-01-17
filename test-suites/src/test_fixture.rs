use crate::dependencies::vault::{VaultClient, VAULT_WASM};
use crate::token::create_stellar_token;
use sep_40_oracle::testutils::{Asset, MockPriceOracleClient, MockPriceOracleWASM};
use sep_40_oracle::Asset as StellarAsset;
use sep_41_token::testutils::MockTokenClient;
use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{vec as svec, Address, Env, String, Symbol};
use std::ops::Index;
use trading::{MarketConfig, TradingClient};

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
            oracle: oracle_id.clone(),
            caller_take_rate: 0, // 1% in SCALAR_7
            max_positions: 10,
            max_utilization: 0, // 0 = disabled. Max leverage in SCALAR_7 (e.g., 20_000_000 = 2x, 50_000_000 = 5x)
        };
        // Set the vault in trading contract
        // After initialize, status is Setup (99) which allows market queuing without delay
        trading_client.initialize(&String::from_str(&e, "Zenex"), &vault_id, &config);

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

    pub fn create_market(&mut self, asset: &StellarAsset, config: MarketConfig) {
        self.trading.queue_set_market(&asset, &config);
        self.trading.set_market(&asset);
    }

    pub fn read_config(&self) -> trading::TradingConfig {
        self.env.as_contract(&self.trading.address, || {
            self.env
                .storage()
                .instance()
                .get(&Symbol::new(&self.env, "Config"))
                .unwrap()
        })
    }

    pub fn read_market_config(&self, asset: StellarAsset) -> MarketConfig {
        self.env.as_contract(&self.trading.address, || {
            self.env
                .storage()
                .persistent()
                .get(&trading::storage::TradingDataKey::MarketConfig(asset))
                .unwrap()
        })
    }

    pub fn read_market_data(&self, asset: StellarAsset) -> trading::MarketData {
        self.env.as_contract(&self.trading.address, || {
            self.env
                .storage()
                .persistent()
                .get(&trading::storage::TradingDataKey::MarketData(asset))
                .unwrap()
        })
    }

    pub fn read_position(&self, position_id: u32) -> trading::Position {
        self.env.as_contract(&self.trading.address, || {
            self.env
                .storage()
                .persistent()
                .get(&trading::storage::TradingDataKey::Position(position_id))
                .unwrap()
        })
    }

    /********** Chain Helpers ***********/

    pub fn jump(&self, time: u64) {
        self.env.ledger().set(LedgerInfo {
            timestamp: self.env.ledger().timestamp().saturating_add(time),
            protocol_version: 23,
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
            protocol_version: 23,
            sequence_number: self.env.ledger().sequence().saturating_add(blocks as u32),
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 999999,
            min_persistent_entry_ttl: 999999,
            max_entry_ttl: 9999999,
        });
    }

}
