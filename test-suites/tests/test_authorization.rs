//! Authorization negative tests for all privileged functions across all contracts.
//!
//! Proves that every `require_auth` and `#[only_owner]` call site rejects
//! unauthorized callers. Two deployment strategies are used:
//!
//! A) **Fixture-based** (trading + vault): Uses `TestFixture` with
//!    `mock_all_auths_allowing_non_root_auth()` so root-level auth is enforced.
//!
//! B) **Direct deployment** (price-verifier, treasury, governance, factory):
//!    Uses `Env::default()` without any auth mocking, then `e.mock_auths()`
//!    precisely for each test.
//!
//! No test in this file uses `mock_all_auths()`.

use soroban_sdk::testutils::{Address as _, BytesN as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, BytesN, Env, IntoVal, String, Symbol, Val, Vec};
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::TestFixture;
use test_suites::SCALAR_7;
use trading::testutils::{default_config, default_market, BTC_FEED_ID, BTC_PRICE};

// WASM bytes for factory deployment tests
const TRADING_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/trading.wasm");
const VAULT_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/strategy_vault.wasm");

// ================================================================
// Helpers
// ================================================================

/// Create a full-stack fixture with markets, vault funded, and a user.
/// The fixture uses `mock_all_auths_allowing_non_root_auth()` so
/// root-level auth (the one we're testing) is NOT mocked.
fn fixture() -> TestFixture<'static> {
    create_fixture_with_data()
}

/// Create a fixture with a funded user who can open positions.
fn fixture_with_user() -> (TestFixture<'static>, Address) {
    let f = fixture();
    let user = Address::generate(&f.env);
    f.token.mint(&user, &(100_000 * SCALAR_7));
    (f, user)
}

// ================================================================
// Trading Admin — #[only_owner] (4 tests)
// ================================================================

#[test]
fn test_set_config_rejects_non_owner() {
    let f = fixture();
    let non_owner = Address::generate(&f.env);
    let config = default_config();

    // Mock auth for non_owner (not the real owner)
    f.env.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &f.trading.address,
            fn_name: "set_config",
            args: (config.clone(),).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    let result = f.trading.try_set_config(&config);
    assert!(result.is_err(), "set_config should reject non-owner");
}

#[test]
fn test_set_market_rejects_non_owner() {
    let f = fixture();
    let non_owner = Address::generate(&f.env);
    let market_config = default_market(&f.env);
    let feed_id: u32 = 99;

    f.env.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &f.trading.address,
            fn_name: "set_market",
            args: (feed_id, market_config.clone()).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    let result = f.trading.try_set_market(&feed_id, &market_config);
    assert!(result.is_err(), "set_market should reject non-owner");
}

#[test]
fn test_del_market_rejects_non_owner() {
    let f = fixture();
    let non_owner = Address::generate(&f.env);
    let feed_id: u32 = BTC_FEED_ID;

    f.env.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &f.trading.address,
            fn_name: "del_market",
            args: (feed_id,).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    let result = f.trading.try_del_market(&feed_id);
    assert!(result.is_err(), "del_market should reject non-owner");
}

#[test]
fn test_set_status_rejects_non_owner() {
    let f = fixture();
    let non_owner = Address::generate(&f.env);
    let status: u32 = 2; // AdminOnIce

    f.env.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &f.trading.address,
            fn_name: "set_status",
            args: (status,).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    let result = f.trading.try_set_status(&status);
    assert!(result.is_err(), "set_status should reject non-owner");
}

// ================================================================
// Trading User — user.require_auth() (5 tests)
// ================================================================

#[test]
fn test_open_market_rejects_wrong_user() {
    let f = fixture();
    let non_owner = Address::generate(&f.env);
    f.token.mint(&non_owner, &(100_000 * SCALAR_7));

    // Mock auth only for non_owner, but call open_market with non_owner as user.
    // The fixture's mock_all_auths_allowing_non_root_auth does NOT mock root-level
    // user.require_auth(), so if we clear that and use mock_auths for the wrong
    // address, it should fail.
    // Actually the fixture already has mock_all_auths_allowing_non_root_auth
    // which DOES allow root auth for addresses that provide it via mock_auths.
    // The key is that open_market calls user.require_auth() where user = the
    // passed address. If we pass non_owner and mock auth for non_owner, the auth
    // check passes because non_owner IS the user. The real test is: can someone
    // else open a position on behalf of a user without the user's auth?

    // Create a real user who has NOT provided auth
    let real_user = Address::generate(&f.env);
    f.token.mint(&real_user, &(100_000 * SCALAR_7));

    // Mock auth for non_owner trying to act as real_user
    f.env.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &f.trading.address,
            fn_name: "open_market",
            args: (
                real_user.clone(),
                BTC_FEED_ID,
                1_000i128 * SCALAR_7,
                10_000i128 * SCALAR_7,
                true,
                0i128,
                0i128,
                f.btc_price(BTC_PRICE as i64),
            )
                .into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    // Call open_market with real_user as the user param, but only non_owner has auth
    let result = f.trading.try_open_market(
        &real_user,
        &BTC_FEED_ID,
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &0,
        &0,
        &f.btc_price(BTC_PRICE as i64),
    );
    assert!(
        result.is_err(),
        "open_market should reject when caller is not the user"
    );
}

#[test]
fn test_close_position_rejects_non_position_owner() {
    let (f, user_a) = fixture_with_user();

    // User A opens a position
    let position_id = f.open_and_fill(&user_a, BTC_FEED_ID, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, BTC_PRICE as i64, 0, 0);

    // Jump past MIN_OPEN_TIME so close is allowed
    f.jump(31);

    // User B tries to close user A's position
    let user_b = Address::generate(&f.env);

    f.env.mock_auths(&[MockAuth {
        address: &user_b,
        invoke: &MockAuthInvoke {
            contract: &f.trading.address,
            fn_name: "close_position",
            args: (position_id, f.btc_price(BTC_PRICE as i64)).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    let result = f.trading.try_close_position(&position_id, &f.btc_price(BTC_PRICE as i64));
    assert!(
        result.is_err(),
        "close_position should reject non-position-owner"
    );
}

#[test]
fn test_cancel_limit_rejects_non_position_owner() {
    let (f, user_a) = fixture_with_user();

    // User A places a limit order
    let position_id = f.trading.place_limit(
        &user_a,
        &BTC_FEED_ID,
        &(1_000 * SCALAR_7),
        &(10_000 * SCALAR_7),
        &true,
        &(BTC_PRICE - 1_000_000_000), // lower price for limit
        &0,
        &0,
    );

    // User B tries to cancel
    let user_b = Address::generate(&f.env);

    f.env.mock_auths(&[MockAuth {
        address: &user_b,
        invoke: &MockAuthInvoke {
            contract: &f.trading.address,
            fn_name: "cancel_limit",
            args: (position_id,).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    let result = f.trading.try_cancel_limit(&position_id);
    assert!(
        result.is_err(),
        "cancel_limit should reject non-position-owner"
    );
}

#[test]
fn test_modify_collateral_rejects_non_position_owner() {
    let (f, user_a) = fixture_with_user();

    let position_id = f.open_and_fill(&user_a, BTC_FEED_ID, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, BTC_PRICE as i64, 0, 0);

    let user_b = Address::generate(&f.env);
    let new_col = 2_000 * SCALAR_7;

    f.env.mock_auths(&[MockAuth {
        address: &user_b,
        invoke: &MockAuthInvoke {
            contract: &f.trading.address,
            fn_name: "modify_collateral",
            args: (position_id, new_col, f.btc_price(BTC_PRICE as i64)).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    let result = f
        .trading
        .try_modify_collateral(&position_id, &new_col, &f.btc_price(BTC_PRICE as i64));
    assert!(
        result.is_err(),
        "modify_collateral should reject non-position-owner"
    );
}

#[test]
fn test_set_triggers_rejects_non_position_owner() {
    let (f, user_a) = fixture_with_user();

    let position_id = f.open_and_fill(&user_a, BTC_FEED_ID, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, BTC_PRICE as i64, 0, 0);

    let user_b = Address::generate(&f.env);

    f.env.mock_auths(&[MockAuth {
        address: &user_b,
        invoke: &MockAuthInvoke {
            contract: &f.trading.address,
            fn_name: "set_triggers",
            args: (position_id, BTC_PRICE * 2, 0i128).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    let result = f
        .trading
        .try_set_triggers(&position_id, &(BTC_PRICE * 2), &0);
    assert!(
        result.is_err(),
        "set_triggers should reject non-position-owner"
    );
}

// ================================================================
// Price Verifier Admin — #[only_owner] (3 tests)
// Direct deployment: register PriceVerifier with e.register()
// ================================================================

#[test]
fn test_update_trusted_signer_rejects_non_owner() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let signer = BytesN::<32>::random(&e);

    let pv_id = e.register(
        price_verifier::PriceVerifier,
        (owner.clone(), signer.clone(), 100u32, 60u64),
    );
    let pv_client = price_verifier::PriceVerifierClient::new(&e, &pv_id);

    let new_signer = BytesN::<32>::random(&e);

    e.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &pv_id,
            fn_name: "update_trusted_signer",
            args: (new_signer.clone(),).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = pv_client.try_update_trusted_signer(&new_signer);
    assert!(
        result.is_err(),
        "update_trusted_signer should reject non-owner"
    );
}

#[test]
fn test_update_max_confidence_rejects_non_owner() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let signer = BytesN::<32>::random(&e);

    let pv_id = e.register(
        price_verifier::PriceVerifier,
        (owner.clone(), signer, 100u32, 60u64),
    );
    let pv_client = price_verifier::PriceVerifierClient::new(&e, &pv_id);

    let new_bps: u32 = 200;

    e.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &pv_id,
            fn_name: "update_max_confidence_bps",
            args: (new_bps,).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = pv_client.try_update_max_confidence_bps(&new_bps);
    assert!(
        result.is_err(),
        "update_max_confidence_bps should reject non-owner"
    );
}

#[test]
fn test_update_max_staleness_rejects_non_owner() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let signer = BytesN::<32>::random(&e);

    let pv_id = e.register(
        price_verifier::PriceVerifier,
        (owner.clone(), signer, 100u32, 60u64),
    );
    let pv_client = price_verifier::PriceVerifierClient::new(&e, &pv_id);

    let new_staleness: u64 = 120;

    e.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &pv_id,
            fn_name: "update_max_staleness",
            args: (new_staleness,).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = pv_client.try_update_max_staleness(&new_staleness);
    assert!(
        result.is_err(),
        "update_max_staleness should reject non-owner"
    );
}

// ================================================================
// Governance/Timelock Admin — #[only_owner] (4 tests)
// Direct deployment: register GovernanceContract with e.register()
// ================================================================

#[test]
fn test_timelock_queue_rejects_non_owner() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let target_addr = Address::generate(&e);
    let delay: u64 = 86400;

    let gov_id = e.register(
        governance::GovernanceContract,
        (owner.clone(), delay),
    );
    let gov_client = governance::GovernanceClient::new(&e, &gov_id);

    let fn_name = Symbol::new(&e, "set_config");
    let args = Vec::<Val>::new(&e);

    e.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &gov_id,
            fn_name: "queue",
            args: (target_addr.clone(), fn_name.clone(), args.clone()).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = gov_client.try_queue(&target_addr, &fn_name, &args);
    assert!(
        result.is_err(),
        "queue should reject non-owner"
    );
}

#[test]
fn test_timelock_cancel_rejects_non_owner() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();
    e.mock_all_auths_allowing_non_root_auth();

    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let target_addr = Address::generate(&e);
    let delay: u64 = 86400;

    let gov_id = e.register(
        governance::GovernanceContract,
        (owner.clone(), delay),
    );
    let gov_client = governance::GovernanceClient::new(&e, &gov_id);

    // First queue something as owner (so there's something to cancel)
    let fn_name = Symbol::new(&e, "set_config");
    let args = Vec::<Val>::new(&e);
    let nonce = gov_client.queue(&target_addr, &fn_name, &args);

    // Now try to cancel as non_owner
    e.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &gov_id,
            fn_name: "cancel",
            args: (nonce,).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = gov_client.try_cancel(&nonce);
    assert!(
        result.is_err(),
        "cancel should reject non-owner"
    );
}

#[test]
fn test_timelock_set_status_rejects_non_owner() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let target_addr = Address::generate(&e);
    let delay: u64 = 86400;

    let gov_id = e.register(
        governance::GovernanceContract,
        (owner.clone(), delay),
    );
    let gov_client = governance::GovernanceClient::new(&e, &gov_id);

    e.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &gov_id,
            fn_name: "set_status",
            args: (target_addr.clone(), 1u32).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = gov_client.try_set_status(&target_addr, &1u32);
    assert!(
        result.is_err(),
        "set_status (governance) should reject non-owner"
    );
}

#[test]
fn test_timelock_set_delay_rejects_non_owner() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let delay: u64 = 86400;

    let gov_id = e.register(
        governance::GovernanceContract,
        (owner.clone(), delay),
    );
    let gov_client = governance::GovernanceClient::new(&e, &gov_id);

    let new_delay: u64 = 172800; // 2 days

    e.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &gov_id,
            fn_name: "set_delay",
            args: (new_delay,).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = gov_client.try_set_delay(&new_delay);
    assert!(
        result.is_err(),
        "set_delay should reject non-owner"
    );
}

// ================================================================
// Vault Strategy-gated — strategy.require_auth() (1 test)
// Via fixture: the vault's strategy is the trading contract address
// ================================================================

#[test]
fn test_strategy_withdraw_rejects_non_strategy() {
    let f = fixture();
    let non_strategy = Address::generate(&f.env);

    // The vault's strategy is the trading contract.
    // A non-strategy address should be rejected.
    f.env.mock_auths(&[MockAuth {
        address: &non_strategy,
        invoke: &MockAuthInvoke {
            contract: &f.vault.address,
            fn_name: "strategy_withdraw",
            args: (non_strategy.clone(), 1_000i128 * SCALAR_7).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    let result = f
        .vault
        .try_strategy_withdraw(&non_strategy, &(1_000 * SCALAR_7));
    assert!(
        result.is_err(),
        "strategy_withdraw should reject non-strategy address"
    );
}

// ================================================================
// Treasury Admin — #[only_owner] (2 tests)
// Direct deployment
// ================================================================

#[test]
fn test_treasury_set_rate_rejects_non_owner() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let rate: i128 = 500_000; // 5% in SCALAR_7

    let treasury_id = e.register(
        treasury::TreasuryContract,
        (owner.clone(), rate),
    );
    let treasury_client = treasury::TreasuryClient::new(&e, &treasury_id);

    let new_rate: i128 = 1_000_000; // 10%

    e.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &treasury_id,
            fn_name: "set_rate",
            args: (new_rate,).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = treasury_client.try_set_rate(&new_rate);
    assert!(
        result.is_err(),
        "set_rate (treasury) should reject non-owner"
    );
}

#[test]
fn test_treasury_withdraw_rejects_non_owner() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let owner = Address::generate(&e);
    let non_owner = Address::generate(&e);
    let rate: i128 = 500_000;

    let treasury_id = e.register(
        treasury::TreasuryContract,
        (owner.clone(), rate),
    );
    let treasury_client = treasury::TreasuryClient::new(&e, &treasury_id);

    let token_addr = Address::generate(&e);
    let to_addr = Address::generate(&e);
    let amount: i128 = 1_000;

    e.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &treasury_id,
            fn_name: "withdraw",
            args: (token_addr.clone(), to_addr.clone(), amount).into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = treasury_client.try_withdraw(&token_addr, &to_addr, &amount);
    assert!(
        result.is_err(),
        "withdraw (treasury) should reject non-owner"
    );
}

// ================================================================
// Factory Deployer — admin.require_auth() (1 test)
// Direct deployment: deploy via e.register() with factory init meta
// ================================================================

#[test]
fn test_factory_deploy_rejects_wrong_admin() {
    let e = Env::default();
    e.cost_estimate().budget().reset_unlimited();

    let real_admin = Address::generate(&e);
    let wrong_admin = Address::generate(&e);

    // Upload WASMs and create factory
    let trading_hash = e.deployer().upload_contract_wasm(TRADING_WASM);
    let vault_hash = e.deployer().upload_contract_wasm(VAULT_WASM);

    let treasury_id = Address::generate(&e);
    let init_meta = factory::FactoryInitMeta {
        trading_hash,
        vault_hash,
        treasury: treasury_id,
    };

    let factory_id = e.register(factory::FactoryContract {}, (init_meta,));
    let factory_client = factory::FactoryClient::new(&e, &factory_id);

    let salt = BytesN::<32>::random(&e);
    let token = Address::generate(&e);
    let pv = Address::generate(&e);
    let config = test_suites::to_factory_config(&default_config());

    // Mock auth for wrong_admin, but the deploy call passes real_admin as the admin param
    e.mock_auths(&[MockAuth {
        address: &wrong_admin,
        invoke: &MockAuthInvoke {
            contract: &factory_id,
            fn_name: "deploy",
            args: (
                real_admin.clone(),
                salt.clone(),
                token.clone(),
                pv.clone(),
                config.clone(),
                String::from_str(&e, "Zenex LP"),
                String::from_str(&e, "zLP"),
                0u32,
                300u64,
            )
                .into_val(&e),
            sub_invokes: &[],
        },
    }]);

    let result = factory_client.try_deploy(
        &real_admin,
        &salt,
        &token,
        &pv,
        &config,
        &String::from_str(&e, "Zenex LP"),
        &String::from_str(&e, "zLP"),
        &0u32,
        &300u64,
    );
    assert!(
        result.is_err(),
        "deploy should reject when wrong admin provides auth"
    );
}
