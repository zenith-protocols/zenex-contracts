use crate::dependencies::trading::trading_contract_wasm::WASM as TRADING_WASM;
use crate::dependencies::vault::{VaultClient, VAULT_WASM};
use crate::pyth_helper;
use crate::token::create_stellar_token;
use ed25519_dalek::SigningKey;
use price_verifier::{PriceVerifier, PriceVerifierClient};
use soroban_sdk::testutils::{Address as _, BytesN as _, Ledger, LedgerInfo};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Bytes, BytesN, Env, String};
use trading::testutils::{default_config, PRICE_SCALAR};
use trading::{MarketConfig, TradingClient};
use treasury::{TreasuryClient, TreasuryContract};
use factory::{FactoryClient, FactoryContract, FactoryInitMeta};

/// Feed IDs matching Pyth Lazer conventions
pub const ETH_FEED_ID: u32 = 2;
pub const XLM_FEED_ID: u32 = 3;

/// Prices in raw Pyth format (exponent -8, so multiply dollars by PRICE_SCALAR)
pub const ETH_PRICE: i128 = 2_000 * PRICE_SCALAR; // $2,000
pub const XLM_PRICE: i128 = PRICE_SCALAR / 10; // $0.10

/// Asset/feed-id enum for readable test code
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum AssetIndex {
    BTC = 1, // Pyth feed ID
    ETH = 2,
    XLM = 3,
}

pub struct TestFixture<'a> {
    pub env: Env,
    pub owner: Address,
    pub users: Vec<Address>,
    pub vault: VaultClient<'a>,
    pub trading: TradingClient<'a>,
    pub price_verifier: PriceVerifierClient<'a>,
    pub token: StellarAssetClient<'a>,
    pub factory: FactoryClient<'a>,
    pub treasury: TreasuryClient<'a>,
    pub signing_key: SigningKey,
}

impl TestFixture<'_> {
    pub fn create<'a>() -> TestFixture<'a> {
        let e = Env::default();
        e.cost_estimate().budget().reset_unlimited();
        e.mock_all_auths_allowing_non_root_auth();

        let owner = Address::generate(&e);
        let (token_id, token_client) = create_stellar_token(&e, &owner);

        // Generate deterministic Ed25519 keypair for signing price updates
        let (signing_key, pubkey_bytes) = pyth_helper::test_keypair();

        // Deploy real PriceVerifier contract (no mock)
        let pv_id = e.register(
            PriceVerifier,
            (
                &owner,
                BytesN::<32>::from_array(&e, &pubkey_bytes),
                200u32, // max_confidence_bps: 2%
                60u64,  // max_staleness: 60 seconds (generous for tests)
            ),
        );
        let pv_client = PriceVerifierClient::new(&e, &pv_id);

        // Deploy real TreasuryContract (no mock)
        let treasury_rate: i128 = 500_000; // 5% in SCALAR_7
        let treasury_id = e.register(TreasuryContract, (&owner, treasury_rate));
        let treasury_client = TreasuryClient::new(&e, &treasury_id);

        // Upload trading + vault WASMs and get hashes for factory
        let trading_hash = e.deployer().upload_contract_wasm(TRADING_WASM);
        let vault_hash = e.deployer().upload_contract_wasm(VAULT_WASM);

        let init_meta = FactoryInitMeta {
            trading_hash,
            vault_hash,
            treasury: treasury_id,
        };
        let factory_id = e.register(FactoryContract {}, (init_meta,));
        let factory_client = FactoryClient::new(&e, &factory_id);

        // Deploy trading + vault atomically via factory
        let config = default_config();
        let salt = BytesN::<32>::random(&e);
        let trading_id = factory_client.deploy(
            &owner,
            &salt,
            &token_id,
            &pv_id,
            &config,
            &String::from_str(&e, "Zenex LP"),
            &String::from_str(&e, "zLP"),
            &0u32,
            &300u64,
        );

        let trading_client = TradingClient::new(&e, &trading_id);
        let vault_id = trading_client.get_vault();
        let vault_client = VaultClient::new(&e, &vault_id);

        TestFixture {
            env: e,
            owner,
            users: vec![],
            vault: vault_client,
            trading: trading_client,
            price_verifier: pv_client,
            token: token_client,
            factory: factory_client,
            treasury: treasury_client,
            signing_key,
        }
    }

    pub fn create_market(&self, feed_id: u32, config: &MarketConfig) {
        self.trading.set_market(&feed_id, config);
    }

    /// Build a signed price update blob for the given feeds at the current ledger timestamp.
    pub fn price_update(&self, feeds: &[pyth_helper::FeedInput]) -> Bytes {
        let ts = self.env.ledger().timestamp();
        pyth_helper::build_price_update(&self.env, &self.signing_key, feeds, ts)
    }

    /// Convenience: BTC-only price update at current timestamp.
    pub fn btc_price(&self, price: i64) -> Bytes {
        pyth_helper::btc_price_update(
            &self.env,
            &self.signing_key,
            price,
            self.env.ledger().timestamp(),
        )
    }

    /// Build a price update for any single feed at current timestamp.
    pub fn price_for_feed(&self, feed_id: u32, price: i64) -> Bytes {
        pyth_helper::build_price_update(
            &self.env,
            &self.signing_key,
            &[pyth_helper::FeedInput {
                feed_id,
                price,
                exponent: -8,
                confidence: None,
            }],
            self.env.ledger().timestamp(),
        )
    }

    /// Convenience: default BTC+ETH+XLM prices at current timestamp.
    pub fn default_prices(&self) -> Bytes {
        pyth_helper::multi_price_update(
            &self.env,
            &self.signing_key,
            10_000_000_000_000, // BTC $100k
            200_000_000_000,    // ETH $2k
            10_000_000,         // XLM $0.10
            self.env.ledger().timestamp(),
        )
    }

    /// Open a market order that fills immediately at the given price.
    /// Returns position_id.
    pub fn open_and_fill(
        &self,
        user: &Address,
        feed_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i64,
        take_profit: i128,
        stop_loss: i128,
    ) -> u32 {
        let price_bytes = self.price_for_feed(feed_id, entry_price);
        self.trading.open_market(
            user,
            &feed_id,
            &collateral,
            &notional_size,
            &is_long,
            &take_profit,
            &stop_loss,
            &price_bytes,
        )
    }

    pub fn position_exists(&self, position_id: u32) -> bool {
        self.env.as_contract(&self.trading.address, || {
            self.env
                .storage()
                .persistent()
                .has(&trading::storage::TradingStorageKey::Position(position_id))
        })
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
            sequence_number: self
                .env
                .ledger()
                .sequence()
                .saturating_add(blocks as u32),
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 999999,
            min_persistent_entry_ttl: 999999,
            max_entry_ttl: 9999999,
        });
    }
}
