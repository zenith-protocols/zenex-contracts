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

/// Realistic multi-market ADL scenario with 1-5% reductions per round.
///
/// 500k vault, all fees/rates zeroed (impact=i128::MAX → 0 fee).
/// 9 positions across BTC/ETH/XLM with varied entry prices.
///
/// Flow:
///   1. Open positions, jump past MIN_OPEN_TIME
///   2. BTC $183k → OnIce (net ~98.6% vault, no ADL)
///   3. BTC $186k → ADL #1 (~1.07% reduction, all non-XLM sides are winners)
///   4. BTC $192k → ADL #2 (~4.68% additional, compounds to ~5.69% total)
///   5. BTC $107k → Active restored (net < 90% vault)
///   6. Liquidate carol's BTC long (equity < threshold, but not underwater)
///   7. Close all remaining at $107k BTC, $2k ETH, $0.10 XLM
///   8. Verify zero notionals and new-position ADL index snapshot
///
/// All expected values from adl_expected_values.py (independent Python math).
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
    // Phase 1 — Open 9 positions at varied entry prices
    // ========================================================
    // BTC longs: alice 200k @$90k, bob 150k @$100k, carol 100k @$110k  (total 450k)
    // BTC short: dave 100k @$200k
    // ETH shorts: alice 200k @$2.5k, bob 150k @$2.2k  (total 350k)
    // ETH long: carol 100k @$1.5k
    // XLM: dave 50k long @$0.10, alice 50k short @$0.10

    let btc_long_alice = fixture.open_long(&alice, FEED_BTC, 5_000, 200_000, 90_000 * PRICE_SCALAR as i64);
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
    // Phase 2 — BTC $183k: OnIce (no ADL)
    // ========================================================
    // net_pnl ~$492,999.99 (~98.6% of $500k vault)
    // net_pnl = 4_929_999_943_000
    // >= 95% vault ($475k = 4_750_000_000_000) → OnIce
    // <= vault ($500k = 5_000_000_000_000) → no ADL
    fixture.trading.update_status(
        &price_update_all(&fixture, 183_000 * PRICE_SCALAR as i64, 200_000_000_000, 10_000_000),
    );

    assert_eq!(fixture.trading.get_status(), 1); // OnIce
    assert_eq!(fixture.trading.get_market_data(&FEED_BTC).l_adl_idx, SCALAR_18);
    assert_eq!(fixture.trading.get_market_data(&FEED_BTC).s_adl_idx, SCALAR_18);
    assert_eq!(fixture.trading.get_market_data(&FEED_ETH).l_adl_idx, SCALAR_18);
    assert_eq!(fixture.trading.get_market_data(&FEED_ETH).s_adl_idx, SCALAR_18);
    assert_eq!(fixture.trading.get_market_data(&FEED_XLM).l_adl_idx, SCALAR_18);
    assert_eq!(fixture.trading.get_market_data(&FEED_XLM).s_adl_idx, SCALAR_18);

    // ========================================================
    // Phase 3 — BTC $186k: ADL #1 (~1.07% reduction)
    // ========================================================
    // All non-XLM sides profitable (BTC short @$200k profits at $186k).
    //
    // net_pnl ~$505,393.93 > vault $500k → ADL triggered
    // net_pnl = 5_053_939_336_000
    // deficit ~$5,393.93 = 53_939_336_000
    // winner_pnl = net_pnl (no losers) = 5_053_939_336_000
    // reduction ~1.067% = floor(53_939_336_000 × 10^18 / 5_053_939_336_000) = 10_672_731_193_226_178
    // factor ~0.9893 = 10^18 - 10_672_731_193_226_178 = 989_327_268_806_773_822
    fixture.trading.update_status(
        &price_update_all(&fixture, 186_000 * PRICE_SCALAR as i64, 200_000_000_000, 10_000_000),
    );

    // Status stays OnIce (ADL runs but doesn't change status in OnIce state)
    assert_eq!(fixture.trading.get_status(), 1);

    // BTC longs: $450k × 0.9893 ~= $445,197.27
    // floor(4_500_000_000_000 × 989_327_268_806_773_822 / 10^18) = 4_451_972_709_630
    let btc = fixture.trading.get_market_data(&FEED_BTC);
    assert_eq!(btc.l_notional, 4_451_972_709_630);
    // idx: 1.0 × factor ~= 0.9893
    // floor(10^18 × 989_327_268_806_773_822 / 10^18) = 989_327_268_806_773_822
    assert_eq!(btc.l_adl_idx, 989_327_268_806_773_822);

    // BTC shorts: $100k × 0.9893 ~= $98,932.73
    // floor(1_000_000_000_000 × 989_327_268_806_773_822 / 10^18) = 989_327_268_806
    assert_eq!(btc.s_notional, 989_327_268_806);
    // idx: 1.0 × factor ~= 0.9893
    // floor(10^18 × 989_327_268_806_773_822 / 10^18) = 989_327_268_806_773_822
    assert_eq!(btc.s_adl_idx, 989_327_268_806_773_822);

    // ETH long: $100k × 0.9893 ~= $98,932.73
    // floor(1_000_000_000_000 × 989_327_268_806_773_822 / 10^18) = 989_327_268_806
    let eth = fixture.trading.get_market_data(&FEED_ETH);
    assert_eq!(eth.l_notional, 989_327_268_806);
    // idx: 1.0 × factor ~= 0.9893
    // floor(10^18 × 989_327_268_806_773_822 / 10^18) = 989_327_268_806_773_822
    assert_eq!(eth.l_adl_idx, 989_327_268_806_773_822);

    // ETH shorts: $350k × 0.9893 ~= $346,264.54
    // floor(3_500_000_000_000 × 989_327_268_806_773_822 / 10^18) = 3_462_645_440_823
    assert_eq!(eth.s_notional, 3_462_645_440_823);
    // idx: 1.0 × factor ~= 0.9893
    // floor(10^18 × 989_327_268_806_773_822 / 10^18) = 989_327_268_806_773_822
    assert_eq!(eth.s_adl_idx, 989_327_268_806_773_822);

    // XLM: zero PnL → untouched
    assert_eq!(fixture.trading.get_market_data(&FEED_XLM).l_adl_idx, SCALAR_18);
    assert_eq!(fixture.trading.get_market_data(&FEED_XLM).s_adl_idx, SCALAR_18);

    // ========================================================
    // Phase 4 — BTC $192k: ADL #2 (compounds to ~5.69% total)
    // ========================================================
    // Uses post-ADL #1 state.
    //
    // net_pnl ~$524,523.32 > vault $500k → ADL #2
    // net_pnl = 5_245_233_231_193
    // deficit ~$24,523.32 = 245_233_231_193
    // winner_pnl = net_pnl = 5_245_233_231_193
    // reduction₂ ~4.675% = floor(245_233_231_193 × 10^18 / 5_245_233_231_193) = 46_753_541_812_901_048
    // factor₂ ~0.9532 = 10^18 - 46_753_541_812_901_048 = 953_246_458_187_098_952
    // compound ~0.9431 = floor(0.9893 × 10^18 × 0.9532 × 10^18 / 10^18) = 943_072_714_977_973_127
    fixture.trading.update_status(
        &price_update_all(&fixture, 192_000 * PRICE_SCALAR as i64, 200_000_000_000, 10_000_000),
    );

    // BTC longs: $445,197.27 × 0.9532 ~= $424,382.72
    // floor(4_451_972_709_630 × 953_246_458_187_098_952 / 10^18) = 4_243_827_217_400
    let btc2 = fixture.trading.get_market_data(&FEED_BTC);
    assert_eq!(btc2.l_notional, 4_243_827_217_400);
    // compound idx: 0.9893 × 0.9532 ~= 0.9431
    // floor(989_327_268_806_773_822 × 953_246_458_187_098_952 / 10^18) = 943_072_714_977_973_127
    assert_eq!(btc2.l_adl_idx, 943_072_714_977_973_127);

    // BTC shorts: $98,932.73 × 0.9532 ~= $94,307.27
    // floor(989_327_268_806 × 953_246_458_187_098_952 / 10^18) = 943_072_714_977
    assert_eq!(btc2.s_notional, 943_072_714_977);
    // compound idx: 0.9893 × 0.9532 ~= 0.9431
    // floor(989_327_268_806_773_822 × 953_246_458_187_098_952 / 10^18) = 943_072_714_977_973_127
    assert_eq!(btc2.s_adl_idx, 943_072_714_977_973_127);

    // ETH long: $98,932.73 × 0.9532 ~= $94,307.27
    // floor(989_327_268_806 × 953_246_458_187_098_952 / 10^18) = 943_072_714_977
    let eth2 = fixture.trading.get_market_data(&FEED_ETH);
    assert_eq!(eth2.l_notional, 943_072_714_977);
    // compound idx: 0.9893 × 0.9532 ~= 0.9431
    // floor(989_327_268_806_773_822 × 953_246_458_187_098_952 / 10^18) = 943_072_714_977_973_127
    assert_eq!(eth2.l_adl_idx, 943_072_714_977_973_127);

    // ETH shorts: $346,264.54 × 0.9532 ~= $330,075.45
    // floor(3_462_645_440_823 × 953_246_458_187_098_952 / 10^18) = 3_300_754_502_422
    assert_eq!(eth2.s_notional, 3_300_754_502_422);
    // compound idx: 0.9893 × 0.9532 ~= 0.9431
    // floor(989_327_268_806_773_822 × 953_246_458_187_098_952 / 10^18) = 943_072_714_977_973_127
    assert_eq!(eth2.s_adl_idx, 943_072_714_977_973_127);

    // XLM still untouched
    assert_eq!(fixture.trading.get_market_data(&FEED_XLM).l_adl_idx, SCALAR_18);
    assert_eq!(fixture.trading.get_market_data(&FEED_XLM).s_adl_idx, SCALAR_18);

    // ========================================================
    // Phase 5 — BTC $107k: Active restored
    // ========================================================
    // net_pnl ~$168,829.06 < 90% vault ($450k) → Active
    // net_pnl = 1_688_290_581_022 < active_line = 4_500_000_000_000
    fixture.trading.update_status(
        &price_update_all(&fixture, 107_000 * PRICE_SCALAR as i64, 200_000_000_000, 10_000_000),
    );
    assert_eq!(fixture.trading.get_status(), 0); // Active

    // ========================================================
    // Phase 6 — Liquidate carol's BTC long at $107k
    // ========================================================
    // Carol: $100k notional @$110k, $3k col, pos.adl_idx = 1.0 (opened before ADL).
    //
    // ADL-adjusted: eff_not = $100k × 0.9431 ~= $94,307.27
    // floor(1_000_000_000_000 × 943_072_714_977_973_127 / 10^18) = 943_072_714_977
    //
    // PnL at $107k: ($107k - $110k) / $110k ~= -2.73%
    // ratio = trunc(-$3k × 10^8 / $110k) = -2_727_272
    // pnl ~= -$2,572.02 = trunc(943_072_714_977 × -2_727_272 / 10^8) = -25_720_158_095
    //
    // equity = $3k + (-$2,572.02) ~= $427.98
    // col + pnl = 30_000_000_000 + (-25_720_158_095) = 4_279_841_905
    //
    // liq_threshold = eff_not × 0.5% ~= $471.54
    // floor(943_072_714_977 × 50_000 / 10_000_000) = 4_715_363_574
    //
    // equity $427.98 < threshold $471.54 → liquidatable (but equity > 0 → NOT underwater)
    let liq_price = fixture.price_for_feed(FEED_BTC, 107_000 * PRICE_SCALAR as i64);
    fixture.trading.execute(&keeper, &svec![&fixture.env, btc_long_carol], &liq_price);
    assert!(!fixture.position_exists(btc_long_carol));

    // ========================================================
    // Phase 7 — Close all remaining at $107k BTC, $2k ETH, $0.10 XLM
    // ========================================================

    let btc_107k = fixture.price_for_feed(FEED_BTC, 107_000 * PRICE_SCALAR as i64);
    let eth_2k = fixture.price_for_feed(FEED_ETH, 200_000_000_000);
    let xlm_010 = fixture.price_for_feed(FEED_XLM, 10_000_000);

    // Alice BTC long: $200k @$90k, $5k col, compound ADL idx 0.9431.
    // eff_not = $200k × 0.9431 ~= $188,614.54
    // floor(2_000_000_000_000 × 943_072_714_977_973_127 / 10^18) = 1_886_145_429_955
    // PnL: ($107k - $90k) / $90k ~= +18.89%
    // ratio = floor($17k × 10^8 / $90k) = 18_888_888
    // pnl ~= $35,627.19 = floor(1_886_145_429_955 × 18_888_888 / 10^8) = 356_271_897_781
    // payout = $5k + $35,627.19 ~= $40,627.19
    // 50_000_000_000 + 356_271_897_781 = 406_271_897_781
    let pay_alice_btc = fixture.trading.close_position(&btc_long_alice, &btc_107k);
    assert_eq!(pay_alice_btc, 406_271_897_781);

    // Bob BTC long: $150k @$100k, $4k col, compound ADL idx 0.9431.
    // eff_not = $150k × 0.9431 ~= $141,460.91
    // floor(1_500_000_000_000 × 943_072_714_977_973_127 / 10^18) = 1_414_609_072_466
    // PnL: ($107k - $100k) / $100k = +7%
    // ratio = floor($7k × 10^8 / $100k) = 7_000_000
    // pnl ~= $9,902.26 = floor(1_414_609_072_466 × 7_000_000 / 10^8) = 99_022_635_072
    // payout = $4k + $9,902.26 ~= $13,902.26
    // 40_000_000_000 + 99_022_635_072 = 139_022_635_072
    let pay_bob_btc = fixture.trading.close_position(&btc_long_bob, &btc_107k);
    assert_eq!(pay_bob_btc, 139_022_635_072);

    // Dave BTC short: $100k @$200k, $25k col, compound ADL idx 0.9431.
    // eff_not = $100k × 0.9431 ~= $94,307.27
    // floor(1_000_000_000_000 × 943_072_714_977_973_127 / 10^18) = 943_072_714_977
    // PnL: ($200k - $107k) / $200k = +46.5%
    // ratio = floor($93k × 10^8 / $200k) = 46_500_000
    // pnl ~= $43,852.88 = floor(943_072_714_977 × 46_500_000 / 10^8) = 438_528_812_464
    // payout = $25k + $43,852.88 ~= $68,852.88
    // 250_000_000_000 + 438_528_812_464 = 688_528_812_464
    let pay_dave_btc = fixture.trading.close_position(&btc_short_dave, &btc_107k);
    assert_eq!(pay_dave_btc, 688_528_812_464);

    // Alice ETH short: $200k @$2.5k, $25k col, compound ADL idx 0.9431.
    // eff_not = $200k × 0.9431 ~= $188,614.54
    // floor(2_000_000_000_000 × 943_072_714_977_973_127 / 10^18) = 1_886_145_429_955
    // PnL: ($2.5k - $2k) / $2.5k = +20%
    // ratio = floor($500 × 10^8 / $2.5k) = 20_000_000
    // pnl ~= $37,722.91 = floor(1_886_145_429_955 × 20_000_000 / 10^8) = 377_229_085_991
    // payout = $25k + $37,722.91 ~= $62,722.91
    // 250_000_000_000 + 377_229_085_991 = 627_229_085_991
    let pay_alice_eth = fixture.trading.close_position(&eth_short_alice, &eth_2k);
    assert_eq!(pay_alice_eth, 627_229_085_991);

    // Bob ETH short: $150k @$2.2k, $20k col, compound ADL idx 0.9431.
    // eff_not = $150k × 0.9431 ~= $141,460.91
    // floor(1_500_000_000_000 × 943_072_714_977_973_127 / 10^18) = 1_414_609_072_466
    // PnL: ($2.2k - $2k) / $2.2k ~= +9.09%
    // ratio = floor($200 × 10^8 / $2.2k) = 9_090_909
    // pnl ~= $12,860.08 = floor(1_414_609_072_466 × 9_090_909 / 10^8) = 128_600_823_483
    // payout = $20k + $12,860.08 ~= $32,860.08
    // 200_000_000_000 + 128_600_823_483 = 328_600_823_483
    let pay_bob_eth = fixture.trading.close_position(&eth_short_bob, &eth_2k);
    assert_eq!(pay_bob_eth, 328_600_823_483);

    // Carol ETH long: $100k @$1.5k, $15k col, compound ADL idx 0.9431.
    // eff_not = $100k × 0.9431 ~= $94,307.27
    // floor(1_000_000_000_000 × 943_072_714_977_973_127 / 10^18) = 943_072_714_977
    // PnL: ($2k - $1.5k) / $1.5k ~= +33.33%
    // ratio = floor($500 × 10^8 / $1.5k) = 33_333_333
    // pnl ~= $31,435.76 = floor(943_072_714_977 × 33_333_333 / 10^8) = 314_357_568_515
    // payout = $15k + $31,435.76 ~= $46,435.76
    // 150_000_000_000 + 314_357_568_515 = 464_357_568_515
    let pay_carol_eth = fixture.trading.close_position(&eth_long_carol, &eth_2k);
    assert_eq!(pay_carol_eth, 464_357_568_515);

    // XLM: no ADL, $0.10 entry = $0.10 close, pnl = $0, no fees.
    // payout = col = $1k = 10_000_000_000
    let pay_xlm_dave = fixture.trading.close_position(&xlm_long_dave, &xlm_010);
    assert_eq!(pay_xlm_dave, 10_000_000_000);
    let pay_xlm_alice = fixture.trading.close_position(&xlm_short_alice, &xlm_010);
    assert_eq!(pay_xlm_alice, 10_000_000_000);

    // ========================================================
    // Phase 8 — Verify zero notionals, new positions snapshot compound index
    // ========================================================

    // Two ADL rounds of floor division can leave up to 2 units of rounding dust
    // per market side (1 unit per round × 2 rounds).
    let btc_f = fixture.trading.get_market_data(&FEED_BTC);
    assert!(btc_f.l_notional <= 2, "btc_l remaining: {}", btc_f.l_notional);
    assert_eq!(btc_f.s_notional, 0);
    let eth_f = fixture.trading.get_market_data(&FEED_ETH);
    assert_eq!(eth_f.l_notional, 0);
    assert!(eth_f.s_notional <= 2, "eth_s remaining: {}", eth_f.s_notional);
    let xlm_f = fixture.trading.get_market_data(&FEED_XLM);
    assert_eq!(xlm_f.l_notional, 0);
    assert_eq!(xlm_f.s_notional, 0);

    // New position on BTC snapshots the compound ADL index (~0.9431)
    let new_btc = fixture.open_long(&alice, FEED_BTC, 1_000, 10_000, 107_000 * PRICE_SCALAR as i64);
    assert_eq!(fixture.trading.get_position(&new_btc).adl_idx, 943_072_714_977_973_127);

    // New position on XLM snapshots SCALAR_18 (no ADL occurred on XLM)
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
