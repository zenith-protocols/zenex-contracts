"""
Realistic ADL scenario. All fees/rates zeroed except impact=i128::MAX (1 unit).
Vault = 500k. No position is underwater at ADL time.

BTC: longs win (price up 140%). Shorts lose but solvent (moderate leverage).
ETH: both sides profit. Long @$1.5k, short @$2.5k, ADL price $2k.
XLM: no side profitable. Both opened @$0.10, ADL price $0.10.

After ADL: one BTC long (high leverage, ~50x) gets margin-squeezed and is liquidatable.

Run: python3 test-suites/tests/adl_expected_values.py
"""
S7 = 10_000_000
S18 = 10**18
PS = 10**8

BTC_E = 100_000 * PS       # $100k entry
BTC_C = 240_000 * PS       # $240k (+140%)
ETH_LONG_E = 1_500 * PS    # $1.5k entry for longs
ETH_SHORT_E = 2_500 * PS   # $2.5k entry for shorts
ETH_C = 2_000 * PS         # $2k at ADL time (both sides profit)
XLM_E = 10_000_000          # $0.10 entry and ADL price
XLM_C = XLM_E

MARGIN = 100_000  # 1% in S7
LIQ_FEE = 50_000  # 0.5% in S7

def fmf(a, b, s): return a * b // s
def fmc(a, b, s): return (a * b + s - 1) // s
def fdf(a, b, s): return a * s // b
def fdc(a, b, s): return (a * s + b - 1) // b

# ============================================================
# Position design
# ============================================================
# BTC longs: 3 positions. One at high leverage (~50x) for post-ADL liquidation.
#   alice: 200k notional, 5k col (~40x) — will be checked for liquidation post-ADL
#   bob:   150k notional, 4k col (~37x) — safe
#   carol: 100k notional, 3k col (~33x) — safe
# BTC short: dave 100k notional, 10k col (~10x) — loses but solvent at +140%
#   dave's loss = 100k × 1.4 = 140k. col=10k - 1 (impact). equity = 10k - 140k < 0.
#   That's underwater! Need more collateral for dave.
#   At 140% loss: need col > 140k for solvency. Use 150k col? That's 0.67x leverage.
#   More realistic: dave shorts with 15k col, 100k notional (6.7x).
#   loss = 100k × 1.4 = 140k. equity = 15k - 140k = -125k. Still underwater.
#
#   The issue: at +140% BTC, ANY short position loses 140% of its notional.
#   Short PnL = notional × (entry - current) / entry = notional × (100k - 240k)/100k = -1.4 × notional.
#   For equity > 0: col > 1.4 × notional. So col must be > 140% of notional.
#   That's < 1x leverage. Not realistic for a perp.
#
#   Solution: dave enters the short at a HIGHER price — say $200k.
#   Then at $240k: loss = notional × (200k - 240k)/200k = -0.2 × notional.
#   With 100k notional and 25k col: equity = 25k - 20k = 5k. Solvent!

BTC_SHORT_E = 200_000 * PS  # Dave's short entry at $200k

# Re-check: Can we still trigger ADL with a $200k short entry?
# Short contributes negative PnL: -(notional × 0.2) = -20k with 100k notional.
# Net still dominated by longs: 450k × 1.4 + ETH winners - 20k - ETH losers.

print("=== Position Setup ===")

# BTC longs (entry $100k)
btc_longs = [
    ("alice", 200_000, 5_000, BTC_E),   # 40x leverage — ADL liquidation candidate
    ("bob",   150_000, 4_000, BTC_E),   # 37x
    ("carol", 100_000, 3_000, BTC_E),   # 33x
]
btc_l_total = sum(n for _, n, _, _ in btc_longs) * S7  # 450k
btc_l_ew = sum(fdf(n*S7, e, PS) for _, n, _, e in btc_longs)

# BTC short (entry $200k — entered after price already moved up)
btc_short = ("dave", 100_000, 25_000, BTC_SHORT_E)  # 4x leverage, solvent at $240k
btc_s_total = btc_short[1] * S7
btc_s_ew = fdf(btc_s_total, BTC_SHORT_E, PS)

print(f"BTC longs: {[n for _, n, _, _ in btc_longs]} = {btc_l_total//S7}k total, ew={btc_l_ew}")
print(f"BTC short: {btc_short[1]}k @ ${btc_short[3]//PS}k, col={btc_short[2]}k, ew={btc_s_ew}")

# ETH: both sides profitable
# Long @$1.5k → at $2k: pnl = notional × (2000-1500)/1500 = +33%
# Short @$2.5k → at $2k: pnl = notional × (2500-2000)/2500 = +20%
eth_positions = [
    ("alice", False, 200_000, 25_000, ETH_SHORT_E),  # short @$2.5k
    ("bob",   False, 150_000, 20_000, ETH_SHORT_E),  # short @$2.5k
    ("carol", True,  100_000, 15_000, ETH_LONG_E),   # long @$1.5k
]
eth_s_total = sum(n*S7 for _, is_l, n, _, _ in eth_positions if not is_l)
eth_l_total = sum(n*S7 for _, is_l, n, _, _ in eth_positions if is_l)
eth_s_ew = sum(fdf(n*S7, e, PS) for _, is_l, n, _, e in eth_positions if not is_l)
eth_l_ew = sum(fdf(n*S7, e, PS) for _, is_l, n, _, e in eth_positions if is_l)

print(f"\nETH shorts: {eth_s_total//S7}k @ $2.5k, ew={eth_s_ew}")
print(f"ETH long:   {eth_l_total//S7}k @ $1.5k, ew={eth_l_ew}")

# XLM: both sides at same price → 0 PnL
xlm_l = 50_000 * S7;  xlm_s = 50_000 * S7
xlm_l_ew = fdf(xlm_l, XLM_E, PS)
xlm_s_ew = fdf(xlm_s, XLM_E, PS)
print(f"\nXLM: 50k long + 50k short @ $0.10, ew_l={xlm_l_ew}, ew_s={xlm_s_ew}")

# ============================================================
# Vault
# ============================================================
# 12 positions, ~9 contribute 1 impact fee to vault
VAULT = 500_000 * S7 + 9
print(f"\nVault = {VAULT} ({VAULT//S7} tokens + 9)")

# ============================================================
# PnL at ADL prices
# ============================================================
print(f"\n=== PnL at ADL prices ===")

btc_l_pnl = fmf(BTC_C, btc_l_ew, PS) - btc_l_total
btc_s_pnl = btc_s_total - fmf(BTC_C, btc_s_ew, PS)
print(f"BTC long:  {btc_l_pnl} ({btc_l_pnl//S7} tok)")
print(f"BTC short: {btc_s_pnl} ({btc_s_pnl//S7} tok)")

# ETH: long @$1.5k → $2k. pnl = price×ew/PS - notional
eth_l_pnl = fmf(ETH_C, eth_l_ew, PS) - eth_l_total
# ETH: short @$2.5k → $2k. pnl = notional - price×ew/PS
eth_s_pnl = eth_s_total - fmf(ETH_C, eth_s_ew, PS)
print(f"ETH long:  {eth_l_pnl} ({eth_l_pnl//S7} tok)")
print(f"ETH short: {eth_s_pnl} ({eth_s_pnl//S7} tok)")

xlm_l_pnl = fmf(XLM_C, xlm_l_ew, PS) - xlm_l
xlm_s_pnl = xlm_s - fmf(XLM_C, xlm_s_ew, PS)
print(f"XLM long:  {xlm_l_pnl}")
print(f"XLM short: {xlm_s_pnl}")

# Check: BTC short dave solvent?
dave_short_pnl = btc_s_pnl
dave_col = btc_short[2] * S7 - 1  # minus impact
dave_equity = dave_col + dave_short_pnl
print(f"\nDave BTC short equity: col={dave_col//S7} + pnl={dave_short_pnl//S7} = {dave_equity//S7} ({'SOLVENT' if dave_equity > 0 else 'UNDERWATER'})")

# Check: all BTC longs solvent?
for name, notional, col, entry in btc_longs:
    pos_pnl = fmf(BTC_C, fdf(notional*S7, entry, PS), PS) - notional*S7
    pos_eq = col*S7 - 1 + pos_pnl
    print(f"  {name} BTC long equity: col={col}k + pnl={pos_pnl//S7} = {pos_eq//S7} ({'SOLVENT' if pos_eq > 0 else 'UNDERWATER'})")

# Net PnL
all_pnls = [btc_l_pnl, btc_s_pnl, eth_l_pnl, eth_s_pnl, xlm_l_pnl, xlm_s_pnl]
net = sum(all_pnls)
winner = sum(p for p in all_pnls if p > 0)

print(f"\nnet_pnl    = {net} ({net//S7} tok)")
print(f"vault      = {VAULT} ({VAULT//S7} tok)")
print(f"deficit    = {net - VAULT} ({(net-VAULT)//S7} tok)")
print(f"winner_pnl = {winner}")

if net <= VAULT:
    print("NO ADL — need more notional or smaller vault")
    exit(1)

# ============================================================
# ADL
# ============================================================
deficit = net - VAULT
reduction = min(fdf(deficit, winner, S18), S18)
factor = S18 - reduction
idx = fmf(S18, factor, S18)

print(f"\n=== ADL Factor ===")
print(f"reduction = {reduction}")
print(f"factor    = {factor} ({factor/S18:.6f})")
print(f"adl_idx   = {idx}")

# Winners: BTC longs, ETH longs, ETH shorts (all positive PnL)
# Losers (untouched): BTC short, XLM both
print(f"\nWinning sides: BTC longs ({btc_l_pnl>0}), ETH longs ({eth_l_pnl>0}), ETH shorts ({eth_s_pnl>0})")
print(f"XLM long ({xlm_l_pnl>0}), XLM short ({xlm_s_pnl>0})")

# Post-ADL state
btc_l_new = fmf(btc_l_total, factor, S18)
btc_l_ew_new = fmf(btc_l_ew, factor, S18)
eth_s_new = fmf(eth_s_total, factor, S18)
eth_s_ew_new = fmf(eth_s_ew, factor, S18)
eth_l_new = fmf(eth_l_total, factor, S18)
eth_l_ew_new = fmf(eth_l_ew, factor, S18)

print(f"\n=== Post-ADL State ===")
print(f"btc_l_notional = {btc_l_new}")
print(f"btc_l_ew       = {btc_l_ew_new}")
print(f"btc_l_adl_idx  = {idx}")
print(f"btc_s (unchanged) = {btc_s_total}, adl_idx = {S18}")
print(f"eth_l_notional = {eth_l_new}")
print(f"eth_l_ew       = {eth_l_ew_new}")
print(f"eth_l_adl_idx  = {idx}")
print(f"eth_s_notional = {eth_s_new}")
print(f"eth_s_ew       = {eth_s_ew_new}")
print(f"eth_s_adl_idx  = {idx}")
print(f"xlm unchanged, adl_idx = {S18}")

# ============================================================
# Post-ADL liquidation check
# ============================================================
print(f"\n=== Post-ADL Liquidation Check ===")
# Alice's BTC long: 200k notional at 40x leverage.
# After ADL, her effective notional is reduced.
# At close: notional_eff = 200k × S7 × adl_idx / S18
alice_btc_not_eff = fmf(200_000 * S7, idx, S18)
# Her PnL at $240k close: notional_eff × (240k - 100k) / 100k = notional_eff × 1.4
alice_btc_pnl = fmf(alice_btc_not_eff, fdf(BTC_C - BTC_E, BTC_E, PS), PS)
alice_btc_col = 5_000 * S7 - 1  # minus open impact
alice_btc_equity = alice_btc_col + alice_btc_pnl - 1  # minus close impact
# Liquidation threshold = notional_eff × (margin + liq_fee)
alice_liq_threshold = fmc(alice_btc_not_eff, MARGIN + LIQ_FEE, S7)

print(f"Alice BTC long post-ADL:")
print(f"  notional_eff = {alice_btc_not_eff}")
print(f"  pnl = {alice_btc_pnl} ({alice_btc_pnl//S7})")
print(f"  col = {alice_btc_col}")
print(f"  equity = {alice_btc_equity}")
print(f"  liq_threshold = {alice_liq_threshold}")
print(f"  liquidatable? {alice_btc_equity < alice_liq_threshold}")
# Alice is profitable (price went up for her long), so she won't be liquidatable.
#
# For liquidation: we need a position that LOSES value and whose equity drops
# below the threshold after ADL. But ADL only reduces winning sides.
# A losing position's notional is unchanged. So ADL doesn't make losers more liquidatable.
#
# Wait — that's right. ADL reduces WINNERS, not losers. The losing positions
# (BTC short, XLM) are untouched. ADL can't cause a liquidation because:
# 1. Winners have their notional reduced → less exposure, more solvent.
# 2. Losers are untouched → same equity as before ADL.
#
# The only way ADL leads to liquidation is if:
# - Price moves further after ADL (market stays volatile)
# - An ADL'd winner opened at high leverage, and a SECOND price reversal
#   causes their now-reduced position to flip from profitable to unprofitable.
#
# Example: Alice's BTC long was profitable at $240k. ADL reduces her notional.
# Then BTC drops to $95k. Her reduced notional means less exposure, but she's
# still leveraged. If the drop is severe enough relative to her reduced position,
# she could be liquidatable.

print(f"\n=== Scenario: Price reversal after ADL ===")
# After ADL at $240k, BTC drops to $95k.
BTC_DROP = 95_000 * PS
# Alice's position: entry $100k, ADL'd notional.
# PnL = notional_eff × (95k - 100k) / 100k = notional_eff × (-0.05)
alice_drop_ratio = fdf(BTC_DROP - BTC_E, BTC_E, PS)
alice_drop_pnl = fmf(alice_btc_not_eff, alice_drop_ratio, PS)
alice_drop_equity = alice_btc_col + alice_drop_pnl
alice_drop_liq = fmc(alice_btc_not_eff, MARGIN + LIQ_FEE, S7)
print(f"Alice BTC long after drop to $95k:")
print(f"  pnl = {alice_drop_pnl} ({alice_drop_pnl//S7})")
print(f"  equity = {alice_drop_equity}")
print(f"  liq_threshold = {alice_drop_liq}")
print(f"  liquidatable? {alice_drop_equity < alice_drop_liq}")

# With 200k notional × factor ≈ 169k effective, at -5%: loss = 8.5k.
# col = 5k - 1. equity = 5k - 8.5k = -3.5k < threshold. Liquidatable!
# But wait, that's underwater, not just below threshold. Need a milder drop.

BTC_MILD = 97_000 * PS
alice_mild_ratio = fdf(BTC_MILD - BTC_E, BTC_E, PS)
alice_mild_pnl = fmf(alice_btc_not_eff, alice_mild_ratio, PS)
alice_mild_equity = alice_btc_col + alice_mild_pnl
print(f"\nAlice BTC long after drop to $97k:")
print(f"  pnl = {alice_mild_pnl} ({alice_mild_pnl//S7})")
print(f"  equity = {alice_mild_equity}")
print(f"  liq_threshold = {alice_drop_liq}")
print(f"  liquidatable? {alice_mild_equity < alice_drop_liq}")

# ============================================================
# Close payouts (at ADL price $240k, no fees, no funding)
# ============================================================
print(f"\n=== Close Payouts (at ADL prices) ===")

# Alice BTC long (200k, 5k col)
a_btc_not = fmf(200_000*S7, idx, S18)
a_btc_pnl = fmf(a_btc_not, fdf(BTC_C - BTC_E, BTC_E, PS), PS)
a_btc_col = 5_000*S7 - 1
a_btc_pay = max(a_btc_col + a_btc_pnl - 1, 0)
print(f"btc_long_alice:  pay={a_btc_pay}")

# Dave BTC short (100k @$200k, 25k col) — loses 20% at $240k
d_btc_not = 100_000*S7  # not ADL'd
d_btc_pnl = fmf(d_btc_not, fdf(BTC_SHORT_E - BTC_C, BTC_SHORT_E, PS), PS)
d_btc_col = 25_000*S7 - 1
d_btc_pay = max(d_btc_col + d_btc_pnl - 1, 0)
print(f"btc_short_dave:  pnl={d_btc_pnl}, pay={d_btc_pay}")

# ETH short alice (200k @$2.5k, 25k col) — ADL'd, profits 20%
a_eth_not = fmf(200_000*S7, idx, S18)
a_eth_pnl = fmf(a_eth_not, fdf(ETH_SHORT_E - ETH_C, ETH_SHORT_E, PS), PS)
a_eth_col = 25_000*S7 - 1
a_eth_pay = max(a_eth_col + a_eth_pnl - 1, 0)
print(f"eth_short_alice: pay={a_eth_pay}")

# ETH long carol (100k @$1.5k, 15k col) — ADL'd, profits 33%
c_eth_not = fmf(100_000*S7, idx, S18)
c_eth_pnl = fmf(c_eth_not, fdf(ETH_C - ETH_LONG_E, ETH_LONG_E, PS), PS)
c_eth_col = 15_000*S7 - 1
c_eth_pay = max(c_eth_col + c_eth_pnl - 1, 0)
print(f"eth_long_carol:  pay={c_eth_pay}")

# XLM long dave (50k @$0.10, 1k col) — no ADL, no PnL
d_xlm_pay = 1_000*S7 - 1 - 1  # col minus open/close impact
print(f"xlm_long_dave:   pay={d_xlm_pay}")

# XLM short alice (50k @$0.10, 1k col) — no ADL, no PnL
a_xlm_pay = 1_000*S7 - 1 - 1
print(f"xlm_short_alice: pay={a_xlm_pay}")
