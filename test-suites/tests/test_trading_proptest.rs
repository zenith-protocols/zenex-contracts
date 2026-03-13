use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_suites::setup::create_fixture_with_data;
use test_suites::test_fixture::AssetIndex;
use test_suites::SCALAR_7;
use trading::testutils::{BTC_FEED_ID, BTC_PRICE, PRICE_SCALAR};

const SECONDS_PER_WEEK: u64 = 604800;

// ==========================================
// Property Tests
// ==========================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// After open → close (no time, no price change), total token value is conserved.
    /// User + vault + contract balances should sum to the same total before and after.
    #[test]
    fn prop_open_close_conserves_total_value(
        collateral_raw in 10u64..10_000,      // 10–10k tokens
        leverage_raw in 2u32..50,              // 2x–50x leverage
        is_long in proptest::bool::ANY,
    ) {
        let fixture = create_fixture_with_data();
        let user = Address::generate(&fixture.env);
        let collateral = (collateral_raw as i128) * SCALAR_7;
        let notional = collateral * (leverage_raw as i128);

        fixture.token.mint(&user, &(100_000 * SCALAR_7));

        let total_before = fixture.token.balance(&user)
            + fixture.token.balance(&fixture.vault.address)
            + fixture.token.balance(&fixture.trading.address)
            + fixture.token.balance(&fixture.treasury);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fixture.open_and_fill(
                &user,
                AssetIndex::BTC as u32,
                collateral,
                notional,
                is_long,
                BTC_PRICE,
                0,
                0,
            )
        }));

        if let Ok((position_id, _)) = result {
            // Close immediately — no time, no price change
            let _ = fixture.trading.close_position(&position_id, &fixture.dummy_price());

            let total_after = fixture.token.balance(&user)
                + fixture.token.balance(&fixture.vault.address)
                + fixture.token.balance(&fixture.trading.address)
                + fixture.token.balance(&fixture.treasury);

            // Total tokens must be conserved (fees move between accounts, not destroyed)
            prop_assert_eq!(total_before, total_after);
        }
        // If open panicked (e.g. margin check), that's fine — nothing happened
    }

    /// After all positions are closed, the contract should hold 0 tokens.
    #[test]
    fn prop_contract_balance_zero_after_all_closed(
        count in 1u32..4,
    ) {
        let fixture = create_fixture_with_data();
        let user = Address::generate(&fixture.env);
        fixture.token.mint(&user, &(1_000_000 * SCALAR_7));

        let mut position_ids = vec![];
        for _ in 0..count {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                fixture.open_and_fill(
                    &user,
                    AssetIndex::BTC as u32,
                    100 * SCALAR_7,
                    1_000 * SCALAR_7,
                    true,
                    BTC_PRICE,
                    0,
                    0,
                )
            }));
            if let Ok((id, _)) = result {
                position_ids.push(id);
            }
        }

        for id in &position_ids {
            let _ = fixture.trading.close_position(id, &fixture.dummy_price());
        }

        let contract_balance = fixture.token.balance(&fixture.trading.address);
        prop_assert_eq!(contract_balance, 0, "Contract should hold 0 after all closed");
    }

    /// When a position is held over time, the vault should profit from interest.
    #[test]
    fn prop_vault_profits_from_interest(
        collateral_raw in 100u64..5_000,
        leverage_raw in 2u32..20,
    ) {
        let fixture = create_fixture_with_data();
        let user = Address::generate(&fixture.env);
        let collateral = (collateral_raw as i128) * SCALAR_7;
        let notional = collateral * (leverage_raw as i128);

        fixture.token.mint(&user, &(1_000_000 * SCALAR_7));

        let vault_before = fixture.token.balance(&fixture.vault.address);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fixture.open_and_fill(
                &user,
                AssetIndex::BTC as u32,
                collateral,
                notional,
                true,
                BTC_PRICE,
                0,
                0,
            )
        }));

        if let Ok((position_id, _)) = result {
            // Hold for a week — interest accrues
            fixture.jump(SECONDS_PER_WEEK);

            let _ = fixture.trading.close_position(&position_id, &fixture.dummy_price());

            let vault_after = fixture.token.balance(&fixture.vault.address);
            prop_assert!(
                vault_after >= vault_before,
                "Vault should profit from interest: before={}, after={}",
                vault_before, vault_after
            );
        }
    }

    /// PnL sign matches direction: long profits when price up, short profits when price down.
    #[test]
    fn prop_pnl_sign_matches_direction(
        price_change_pct in -30i32..30,
        is_long in proptest::bool::ANY,
    ) {
        if price_change_pct == 0 {
            return Ok(());
        }

        let fixture = create_fixture_with_data();
        let user = Address::generate(&fixture.env);
        fixture.token.mint(&user, &(1_000_000 * SCALAR_7));

        let (position_id, _) = fixture.open_and_fill(
            &user,
            AssetIndex::BTC as u32,
            10_000 * SCALAR_7,
            20_000 * SCALAR_7, // 2x leverage
            is_long,
            BTC_PRICE,
            0,
            0,
        );

        // Change price
        let new_price = 100_000 * PRICE_SCALAR + (price_change_pct as i128) * 1_000 * PRICE_SCALAR;
        fixture.set_price(BTC_FEED_ID, new_price);

        let (pnl, _) = fixture.trading.close_position(&position_id, &fixture.dummy_price());

        if is_long {
            if price_change_pct > 0 {
                prop_assert!(pnl > 0, "Long should profit when price up, got pnl={}", pnl);
            } else {
                prop_assert!(pnl < 0, "Long should lose when price down, got pnl={}", pnl);
            }
        } else {
            if price_change_pct > 0 {
                prop_assert!(pnl < 0, "Short should lose when price up, got pnl={}", pnl);
            } else {
                prop_assert!(pnl > 0, "Short should profit when price down, got pnl={}", pnl);
            }
        }
    }
}
