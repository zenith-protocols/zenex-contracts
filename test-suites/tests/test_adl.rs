use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec as svec, Address};
use test_suites::test_fixture::TestFixture;
use test_suites::pyth_helper;
use test_suites::constants::{SCALAR_7, SCALAR_18};
use trading::testutils::{default_market, FEED_BTC, FEED_ETH, FEED_XLM, PRICE_SCALAR};

fn price_update_all(fixture: &TestFixture, btc: i64, eth: i64, xlm: i64) -> soroban_sdk::Bytes {
    let ts = fixture.env.ledger().timestamp();
    pyth_helper::build_price_update(
        &fixture.env,
        &fixture.signing_key,
        &[
            pyth_helper::FeedInput { feed_id: 1, price: btc, exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: 2, price: eth, exponent: -8, confidence: 0 },
            pyth_helper::FeedInput { feed_id: 3, price: xlm, exponent: -8, confidence: 0 },
        ],
        ts,
    )
}

#[test]
fn test_adl_multi_market_scenario() {
    let fixture = TestFixture::create();
    fixture.token.mint(&fixture.owner, &(500_000 * SCALAR_7));
    fixture.vault.deposit(&(500_000 * SCALAR_7), &fixture.owner, &fixture.owner, &fixture.owner);

    let mut config = fixture.trading.get_config();
    config.r_funding = 0; config.r_base = 0; config.r_var = 0;
    config.fee_dom = 0; config.fee_non_dom = 0;
    fixture.trading.set_config(&config);

    let mut mc = default_market(&fixture.env);
    mc.r_var_market = 0; mc.impact = i128::MAX;
    fixture.create_market(FEED_BTC, &mc);
    fixture.create_market(FEED_ETH, &mc);
    fixture.create_market(FEED_XLM, &mc);

    let alice = Address::generate(&fixture.env);
    let bob = Address::generate(&fixture.env);
    let carol = Address::generate(&fixture.env);
    let dave = Address::generate(&fixture.env);
    let keeper = Address::generate(&fixture.env);

    for u in [&alice, &bob, &carol, &dave] {
        fixture.token.mint(u, &(200_000 * SCALAR_7));
    }

    // ========================================================
    // Phase 1 — Open positions at varied entry prices
    // ========================================================

    let _btc_long_alice = fixture.open_long(&alice, FEED_BTC, 5_000, 200_000, 90_000 * PRICE_SCALAR as i64);
    let btc_long_bob = fixture.open_long(&bob, FEED_BTC, 4_000, 150_000, 100_000 * PRICE_SCALAR as i64);
    let btc_long_carol = fixture.open_long(&carol, FEED_BTC, 3_000, 100_000, 110_000 * PRICE_SCALAR as i64);
    let btc_short_dave = fixture.open_short(&dave, FEED_BTC, 25_000, 100_000, 200_000 * PRICE_SCALAR as i64);

    let eth_short_alice = fixture.open_short(&alice, FEED_ETH, 25_000, 200_000, 250_000_000_000);
    let eth_short_bob = fixture.open_short(&bob, FEED_ETH, 20_000, 150_000, 220_000_000_000);
    let eth_long_carol = fixture.open_long(&carol, FEED_ETH, 15_000, 100_000, 150_000_000_000);

    let xlm_long_dave = fixture.open_long(&dave, FEED_XLM, 1_000, 50_000, 10_000_000);
    let xlm_short_alice = fixture.open_short(&alice, FEED_XLM, 1_000, 50_000, 10_000_000);

    fixture.jump(31);

    // ========================================================
    // Phase 2 — ADL #1: BTC $240k (+140%), ETH $2k, XLM $0.10
    // ========================================================
    // vault = 5_000_000_000_009 (500k tokens + 9 impact fees from opens)
    //
    // BTC long PnL (entry_wt = 22222222 + 15000000 + 9090909 = 46313131):
    //   pnl = $240k × 46313131 / 100_000_000 - 4_500_000_000_000 = 6_615_151_440_000
    // BTC short PnL (entry_wt = 5000000, entry $200k):
    //   pnl = 1_000_000_000_000 - $240k × 5000000 / 100_000_000 = -200_000_000_000
    // ETH long PnL (entry_wt = 666666666, entry $1.5k):
    //   pnl = $2k × 666666666 / 100_000_000 - 1_000_000_000_000 = 333_333_332_000
    // ETH short PnL (entry_wt = 800000000 + 681818181 = 1481818181):
    //   pnl = 3_500_000_000_000 - $2k × 1481818181 / 100_000_000 = 536_363_638_000
    //
    // net_pnl = 6_615_151_440_000 + (-200_000_000_000) + 333_333_332_000 + 536_363_638_000
    //         = 7_284_848_410_000
    // winner_pnl = 6_615_151_440_000 + 333_333_332_000 + 536_363_638_000 = 7_484_848_410_000
    // deficit = 7_284_848_410_000 - 5_000_000_000_000 = 2_284_848_410_000
    // reduction = floor(2_284_848_410_000 × 1_000_000_000_000_000_000 / 7_484_848_410_000)
    //           = 305_263_150_947_368_351
    // factor = 1_000_000_000_000_000_000 - 305_263_150_947_368_351 = 694_736_849_052_631_649
    fixture.trading.update_status(
        &price_update_all(&fixture, 240_000 * PRICE_SCALAR as i64, 200_000_000_000, 10_000_000),
    );

    assert_eq!(fixture.trading.get_status(), 1);

    // BTC longs reduced: floor(4_500_000_000_000 × 694_736_849_052_631_649 / 1_000_000_000_000_000_000)
    //                   = 3_126_315_820_736
    let btc = fixture.trading.get_market_data(&FEED_BTC);
    assert_eq!(btc.l_notional, 3_126_315_820_736);
    assert_eq!(btc.l_adl_idx, 694_736_849_052_631_649);
    assert_eq!(btc.s_notional, 100_000 * SCALAR_7);
    assert_eq!(btc.s_adl_idx, SCALAR_18);

    // ETH long reduced: floor(1_000_000_000_000 × 694_736_849_052_631_649 / 1e18) = 694_736_849_052
    // ETH short reduced: floor(3_500_000_000_000 × factor / 1e18) = 2_431_578_971_684
    let eth = fixture.trading.get_market_data(&FEED_ETH);
    assert_eq!(eth.l_notional, 694_736_849_052);
    assert_eq!(eth.s_notional, 2_431_578_971_684);
    assert_eq!(eth.l_adl_idx, 694_736_849_052_631_649);
    assert_eq!(eth.s_adl_idx, 694_736_849_052_631_649);

    assert_eq!(fixture.trading.get_market_data(&FEED_XLM).l_adl_idx, SCALAR_18);
    assert_eq!(fixture.trading.get_market_data(&FEED_XLM).s_adl_idx, SCALAR_18);

    // ========================================================
    // Phase 3 — Partial close (OnIce allows closes)
    // ========================================================

    let btc_close_240k = fixture.price_for_feed(FEED_BTC, 240_000 * PRICE_SCALAR as i64);

    // Alice BTC long: 200_000 notional @$90k, col = 50_000_000_000 - 1 (impact).
    // ADL-adjusted notional = floor(2_000_000_000_000 × 694_736_849_052_631_649 / 1_000_000_000_000_000_000)
    //                       = 1_389_473_698_107
    // pnl ratio = floor(($240k - $90k) × 100_000_000 / ($90k × 100_000_000))
    //           = floor(150_000 × 100_000_000 / 90_000) = 166_666_666
    // pnl = floor(1_389_473_698_107 × 166_666_666 / 100_000_000) = 2_315_789_495_580
    // payout = (50_000_000_000 - 1) + 2_315_789_495_580 - 1 = 2_365_789_487_578
    let pay_alice_btc = fixture.trading.close_position(&0u32, &btc_close_240k);
    assert_eq!(pay_alice_btc, 2_365_789_487_578);

    // Dave BTC short: 100_000 notional @$200k, not ADL'd. col = 250_000_000_000 - 1.
    // pnl ratio = floor(($200k - $240k) × 100_000_000 / ($200k × 100_000_000))
    //           = floor(-40_000 × 100_000_000 / 200_000) = -20_000_000
    // pnl = floor(1_000_000_000_000 × (-20_000_000) / 100_000_000) = -200_000_000_000
    // payout = max((250_000_000_000 - 1) + (-200_000_000_000) - 1, 0) = 50_000_000_000
    let pay_dave_btc = fixture.trading.close_position(&btc_short_dave, &btc_close_240k);
    assert_eq!(pay_dave_btc, 50_000_000_000);

    let btc_mid = fixture.trading.get_market_data(&FEED_BTC);
    assert_eq!(btc_mid.l_notional, 1_736_842_122_631);
    assert_eq!(btc_mid.s_notional, 0);

    assert_eq!(fixture.vault.total_assets(), 2_884_210_512_422);

    // ========================================================
    // Phase 4 — ADL #2: BTC $300k (still OnIce, continued rally)
    // ========================================================
    // Uses post-close market state (btc_l_not=1_736_842_122_631, btc_l_ew=16_736_842).
    // vault = 2_884_210_512_422.
    //
    // BTC long pnl = $300k × 16_736_842 / 100_000_000 - 1_736_842_122_631 = 3_283_210_477_365
    // ETH long pnl = $2k × 463_157_898 / 100_000_000 - 694_736_849_052 = 231_578_947_947
    // ETH short pnl = 2_431_578_971_684 - $2k × 1_029_473_693 / 100_000_000 = 372_990_084_088
    //
    // net = 3_887_779_509_400. winner = 3_887_779_509_400.
    // deficit = 3_887_779_509_400 - 2_884_210_512_422 = 1_003_568_996_970
    // reduction = floor(1_003_568_996_970 × 1e18 / 3_887_779_509_400) = 258_256_627_815_618_144
    // factor₂ = 741_743_372_184_381_856
    // compound = floor(694_736_849_052_631_649 × 741_743_372_184_381_856 / 1e18)
    //          = 515_316_453_195_489_003
    fixture.trading.update_status(
        &price_update_all(&fixture, 300_000 * PRICE_SCALAR as i64, 200_000_000_000, 10_000_000),
    );

    // BTC long: floor(1_736_842_122_631 × 741_743_372_184_381_856 / 1e18) = 1_288_291_132_988
    let btc2 = fixture.trading.get_market_data(&FEED_BTC);
    assert_eq!(btc2.l_notional, 1_288_291_132_988);
    assert_eq!(btc2.l_adl_idx, 515_316_453_195_489_003);

    // ETH long: floor(694_736_849_052 × factor₂ / 1e18) = 515_316_453_195
    // ETH short: floor(2_431_578_971_684 × factor₂ / 1e18) = 1_803_607_586_184
    let eth2 = fixture.trading.get_market_data(&FEED_ETH);
    assert_eq!(eth2.l_notional, 515_316_453_195);
    assert_eq!(eth2.l_adl_idx, 515_316_453_195_489_003);
    assert_eq!(eth2.s_notional, 1_803_607_586_184);
    assert_eq!(eth2.s_adl_idx, 515_316_453_195_489_003);

    assert_eq!(fixture.trading.get_market_data(&FEED_XLM).l_adl_idx, SCALAR_18);

    // ========================================================
    // Phase 5 — Price reversal to $104k → Active restored, carol liquidatable
    // ========================================================
    // net_pnl at $104k ≈ 450_972_330_000 < 90% × vault (2_595_789_461_187) → Active.
    //
    // Carol's BTC long: 100_000 notional @$110k, pos.adl_idx = 1e18 (opened before any ADL).
    // market adl_idx = 515_316_453_195_489_003.
    // effective_not = floor(1_000_000_000_000 × 515_316_453_195_489_003 / 1e18) = 515_316_453_195
    // pnl ratio = floor(($104k - $110k) × 100_000_000 / $110k) = -5_454_545
    // pnl = floor(515_316_453_195 × (-5_454_545) / 100_000_000) = -28_108_338_810
    // equity = (30_000_000_000 - 1) + (-28_108_338_810) - 1 = 1_891_661_188
    // liq_threshold = floor(515_316_453_195 × 50_000 / 10_000_000) = 2_576_582_265
    // equity (1_891_661_188) < threshold (2_576_582_265) → liquidatable
    fixture.trading.update_status(
        &price_update_all(&fixture, 104_000 * PRICE_SCALAR as i64, 200_000_000_000, 10_000_000),
    );
    assert_eq!(fixture.trading.get_status(), 0);

    let liq_price = fixture.price_for_feed(FEED_BTC, 104_000 * PRICE_SCALAR as i64);
    fixture.trading.execute(&keeper, &svec![&fixture.env, btc_long_carol], &liq_price);
    assert!(!fixture.position_exists(btc_long_carol));

    // ========================================================
    // Phase 6 — Close remaining at current market prices
    // ========================================================

    let btc_104k = fixture.price_for_feed(FEED_BTC, 104_000 * PRICE_SCALAR as i64);
    let eth_2k = fixture.price_for_feed(FEED_ETH, 200_000_000_000);
    let xlm_010 = fixture.price_for_feed(FEED_XLM, 10_000_000);

    // Bob BTC long: 150_000 @$100k, compound idx 515_316_453_195_489_003.
    // eff_not = floor(1_500_000_000_000 × 515_316_453_195_489_003 / 1e18) = 772_974_679_796
    // pnl ratio = floor(($104k - $100k) × 100_000_000 / $100k) = 4_000_000
    // pnl = floor(772_974_679_796 × 4_000_000 / 100_000_000) = 30_918_987_191
    // payout = (40_000_000_000 - 1) + 30_918_987_191 - 1 = 70_918_987_191
    let pay_bob_btc = fixture.trading.close_position(&btc_long_bob, &btc_104k);
    assert_eq!(pay_bob_btc, 70_918_987_191);

    // Alice ETH short: 200_000 @$2.5k, compound idx.
    // eff_not = floor(2_000_000_000_000 × 515_316_453_195_489_003 / 1e18) = 1_030_632_906_395
    // pnl ratio = floor(($2.5k - $2k) × 100_000_000 / $2.5k) = 20_000_000
    // pnl = floor(1_030_632_906_395 × 20_000_000 / 100_000_000) = 206_126_581_279
    // payout = (250_000_000_000 - 1) + 206_126_581_279 - 1 = 456_126_581_278
    let pay_eth_alice = fixture.trading.close_position(&eth_short_alice, &eth_2k);
    assert_eq!(pay_eth_alice, 456_126_581_278);

    // Bob ETH short: 150_000 @$2.2k.
    // eff_not = floor(1_500_000_000_000 × 515_316_453_195_489_003 / 1e18) = 772_974_679_796
    // pnl ratio = floor(($2.2k - $2k) × 100_000_000 / $2.2k) = 9_090_909
    // pnl = floor(772_974_679_796 × 9_090_909 / 100_000_000) = 70_270_424_733
    // payout = (200_000_000_000 - 1) + 70_270_424_733 - 1 = 270_270_424_733
    let pay_eth_bob = fixture.trading.close_position(&eth_short_bob, &eth_2k);
    assert_eq!(pay_eth_bob, 270_270_424_733);

    // Carol ETH long: 100_000 @$1.5k.
    // eff_not = floor(1_000_000_000_000 × 515_316_453_195_489_003 / 1e18) = 515_316_453_195
    // pnl ratio = floor(($2k - $1.5k) × 100_000_000 / $1.5k) = 33_333_333
    // pnl = floor(515_316_453_195 × 33_333_333 / 100_000_000) = 171_772_149_347
    // payout = (150_000_000_000 - 1) + 171_772_149_347 - 1 = 321_772_149_347
    let pay_eth_carol = fixture.trading.close_position(&eth_long_carol, &eth_2k);
    assert_eq!(pay_eth_carol, 321_772_149_347);

    // XLM: no ADL, $0.10 entry = $0.10 close, pnl = 0.
    // payout = 10_000_000_000 - 1 (open impact) - 1 (close impact) = 10_000_000_000
    let pay_xlm_dave = fixture.trading.close_position(&xlm_long_dave, &xlm_010);
    assert_eq!(pay_xlm_dave, 10_000_000_000);
    let pay_xlm_alice = fixture.trading.close_position(&xlm_short_alice, &xlm_010);
    assert_eq!(pay_xlm_alice, 10_000_000_000);

    // ========================================================
    // Phase 7 — Verify zero notionals, new position snapshots compound index
    // ========================================================

    // Two ADL rounds of floor division can leave up to 2 units of rounding dust
    // per market side (1 unit per round × 2 rounds).
    let btc_f = fixture.trading.get_market_data(&FEED_BTC);
    assert!(btc_f.l_notional <= 2);
    assert_eq!(btc_f.s_notional, 0);
    let eth_f = fixture.trading.get_market_data(&FEED_ETH);
    assert_eq!(eth_f.l_notional, 0);
    assert!(eth_f.s_notional <= 1);
    let xlm_f = fixture.trading.get_market_data(&FEED_XLM);
    assert_eq!(xlm_f.l_notional, 0);
    assert_eq!(xlm_f.s_notional, 0);

    let new_btc = fixture.open_long(&alice, FEED_BTC, 1_000, 10_000, 104_000 * PRICE_SCALAR as i64);
    assert_eq!(fixture.trading.get_position(&new_btc).adl_idx, 515_316_453_195_489_003);

    let new_xlm = fixture.open_long(&alice, FEED_XLM, 1_000, 10_000, 10_000_000);
    assert_eq!(fixture.trading.get_position(&new_xlm).adl_idx, SCALAR_18);
}

#[test]
#[should_panic(expected = "Error(Contract, #750)")]
fn test_adl_cannot_retrigger_with_small_positions() {
    let fixture = TestFixture::create();
    fixture.token.mint(&fixture.owner, &(500_000 * SCALAR_7));
    fixture.vault.deposit(&(500_000 * SCALAR_7), &fixture.owner, &fixture.owner, &fixture.owner);

    let mut config = fixture.trading.get_config();
    config.r_funding = 0; config.r_base = 0; config.r_var = 0;
    config.fee_dom = 0; config.fee_non_dom = 0;
    fixture.trading.set_config(&config);

    let mut mc = default_market(&fixture.env);
    mc.r_var_market = 0; mc.impact = i128::MAX;
    fixture.create_market(FEED_BTC, &mc);
    fixture.create_market(FEED_ETH, &mc);
    fixture.create_market(FEED_XLM, &mc);

    let alice = Address::generate(&fixture.env);
    fixture.token.mint(&alice, &(100_000 * SCALAR_7));

    fixture.open_long(&alice, FEED_BTC, 500, 1_000, 100_000 * PRICE_SCALAR as i64);
    fixture.open_long(&alice, FEED_XLM, 500, 1_000, 10_000_000);
    fixture.jump(31);

    // 1_000 × 10_000_000 notional at 100× BTC: PnL = 99_000 × 10_000_000 < vault 500_000 × 10_000_000
    fixture.trading.update_status(
        &price_update_all(&fixture, 10_000_000 * PRICE_SCALAR as i64, 200_000_000_000, 10_000_000),
    );
}
