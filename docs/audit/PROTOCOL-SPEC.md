# Zenex Protocol Specification

**Version:** 1.0
**Status:** Audit Preparation
**Contracts in scope:** Trading, Strategy Vault, Factory, Price Verifier, Governance (Timelock)

---

## 1. Overview

Zenex is a decentralized perpetual futures trading protocol on the Stellar blockchain (Soroban). Traders open leveraged long and short positions on price feeds (e.g., BTC/USD), backed by a shared liquidity vault (ERC-4626 compliant) that serves as the counterparty. The protocol uses a single collateral token per pool, a single oracle provider (Pyth Lazer), and all settlement happens on-chain with no off-chain matching engine.

Key properties:
- **Leveraged perpetual positions** with configurable margin requirements
- **Peer-to-peer funding fees** -- the dominant side pays the non-dominant side; the protocol takes zero cut
- **Utilization-based borrowing fees** -- only the dominant side pays, using a `util^5` curve
- **Auto-deleveraging (ADL)** -- a permissionless circuit breaker that reduces winning positions when the vault cannot cover liabilities
- **ERC-4626 vault** -- LPs deposit collateral and receive share tokens; the trading contract withdraws from the vault to pay winning traders
- **Factory deployment** -- atomic vault+trading deployment with deterministic addresses to resolve circular dependencies
- **Timelock governance** -- admin config changes are queued with a configurable delay before execution

---

## 2. How to Audit This Codebase

**Suggested audit order:**

1. **Read this protocol spec** -- understand the math, position lifecycle, fee system, and trust assumptions. Every formula references its source file so you can verify it matches the implementation.

2. **Review the threat model** -- see `docs/audit/THREAT-MODEL.md` for STRIDE analysis, trust boundaries, and threat catalog with severity ratings.

3. **Check the threat-test matrix** -- see `docs/audit/THREAT-TEST-MATRIX.md` for traceability from each threat to its corresponding test case.

4. **Run the test suite** -- from the repo root:
   ```bash
   cargo test --workspace
   ```
   Integration tests live in `test-suites/tests/`. Unit tests are inline in each crate.

5. **Review the code** -- start with the contract entry points (`trading/src/contract.rs`) and trace into business logic (`trading/src/trading/`).

**Key files per concern:**

| Concern | Primary File | Supporting Files |
|---------|-------------|-----------------|
| Fee math | `trading/src/trading/rates.rs` | `trading/src/trading/market.rs` (accrual) |
| Position lifecycle | `trading/src/trading/actions.rs` | `trading/src/trading/execute.rs` (keeper triggers) |
| Settlement | `trading/src/trading/position.rs` | `trading/src/trading/market.rs` (close) |
| ADL / Circuit breaker | `trading/src/trading/adl.rs` | `trading/src/constants.rs` (thresholds) |
| Vault | `strategy-vault/src/contract.rs` | `strategy-vault/src/strategy.rs` |
| Price verification | `price-verifier/src/pyth.rs` | `price-verifier/src/lib.rs` |
| Factory deployment | `factory/src/lib.rs` | `factory/src/storage.rs` |
| Governance / Timelock | `governance/src/lib.rs` | -- |
| Validation | `trading/src/validation.rs` | `trading/src/constants.rs` (caps) |
| Error codes | `trading/src/errors.rs` | -- |

---

## 3. Decimal System

The protocol uses three fixed-point precision scales. All arithmetic uses the `SorobanFixedPoint` trait from `soroban-fixed-point-math`.

### 3.1 Precision Scales

| Scale | Value | Usage | Defined In |
|-------|-------|-------|-----------|
| `SCALAR_7` | `10_000_000` (10^7) | All rates, fees, ratios, weights, leverage, utilization | `trading/src/constants.rs` |
| `SCALAR_18` | `10^18` | Interest indices (funding, borrowing, ADL), funding rates | `trading/src/constants.rs` |
| `price_scalar` | `10^(-exponent)` | Price operations; derived from Pyth oracle exponent at verify time | Computed in `trading/src/dependencies.rs` via `scalar_from_exponent()` |

### 3.2 Naming Conventions

| Suffix | Meaning | Example |
|--------|---------|---------|
| No suffix | Token units (collateral, notional in `token_decimals`) | `col`, `notional` |
| `_scalar` | Divisor for fixed-point operations | `price_scalar`, `SCALAR_7` |
| `_idx` | Cumulative index value (always SCALAR_18) | `fund_idx`, `borr_idx`, `adl_idx` |
| `_rate` | Rate value (SCALAR_7 for weights, SCALAR_18 for interest) | `fund_rate`, `borr_rate` |
| `_fee` | Fee amount (token_decimals unless explicitly scaled) | `base_fee`, `impact_fee` |

### 3.3 Rounding Direction

The protocol uses directional rounding to protect the vault:

| Operation | Direction | Method | Rationale |
|-----------|-----------|--------|-----------|
| Fee calculations (base, impact, borrowing) | **Ceil** | `fixed_mul_ceil` | Collect more fees -- protects vault |
| PnL calculations | **Floor** | `fixed_mul_floor` / `fixed_div_floor` | Pay less PnL -- protects vault |
| Utilization checks | **Ceil** | `fixed_div_ceil` | Trigger limits earlier -- protects vault |
| Funding receive delta | **Floor** | `fixed_mul_floor` | Receive less -- ensures funding pot is not over-distributed |
| Entry weight calculation | **Floor** | `fixed_div_floor` | Conservative position tracking |
| ADL reduction | **Floor** | `fixed_mul_floor` | Reduce more aggressively -- protects vault |

Source: rounding directions visible in `trading/src/trading/market.rs`, `trading/src/trading/position.rs`, `trading/src/trading/rates.rs`.

### 3.4 Price Scalar Derivation

The `price_scalar` is NOT stored on-chain. It is recomputed on every price verification:

```
price_scalar = 10^(-exponent)
```

For example, BTC with Pyth exponent `-8` yields `price_scalar = 100_000_000`.

Source: `trading/src/dependencies.rs` -- `scalar_from_exponent()` function.

---

## 4. Position Lifecycle

### 4.1 Market Order (Immediate Fill)

**Entry point:** `Trading::open_market()` in `trading/src/contract.rs`
**Implementation:** `execute_create_market()` in `trading/src/trading/actions.rs`

Steps:
1. `require_active(e)` -- contract must be in Active status
2. `user.require_auth()` -- user signs the transaction
3. `Market::load(e, price_data)` -- load market context, accrue indices to current time
4. `Position::create(e, user, feed_id, is_long, market.price, collateral, notional, sl, tp)` -- create position with ID
5. `market.open(e, &mut position, id)`:
   a. Compute `base_fee` based on dominant/non-dominant side
   b. Compute `impact_fee = notional / impact` (quadratic-like divisor)
   c. Deduct fees from position collateral: `position.col -= base_fee + impact_fee`
   d. `position.validate()` -- check notional bounds, leverage, enabled
   e. `position.fill(e, &data)` -- snapshot `fund_idx`, `borr_idx`, `adl_idx` from current market data
   f. Persist position to storage
   g. Update market stats: `data.update_stats(long, notional, ew_delta)`
   h. `total_notional += notional`
   i. `require_within_util(e)` -- check per-market and global utilization caps
6. Transfer collateral from user to contract
7. Transfer opening fees: vault gets `total_fee - treasury_fee`, treasury gets its cut
8. `market.store(e)` -- persist updated market data
9. Emit `OpenMarket` event

### 4.2 Limit Order (Pending)

**Entry point:** `Trading::place_limit()` in `trading/src/contract.rs`
**Implementation:** `execute_create_limit()` in `trading/src/trading/actions.rs`

Steps:
1. `require_active(e)` -- contract must be Active
2. `user.require_auth()`
3. `Position::create(...)` -- create position with `filled = false`
4. `position.validate()` -- check notional bounds, leverage
5. Persist position (NOT filled yet -- no fee deduction, no index snapshot)
6. Transfer collateral from user to contract
7. Emit `PlaceLimit` event

### 4.3 Limit Order Fill (Keeper)

**Entry point:** `Trading::execute()` with `ExecuteRequestType::Fill`
**Implementation:** `apply_fill()` in `trading/src/trading/execute.rs`

Steps:
1. `require_can_manage(e)` -- contract must not be Frozen
2. Verify position is not already filled (`!position.filled`)
3. Check fill condition: long requires `market.price <= entry_price`, short requires `market.price >= entry_price`
4. Set `position.entry_price = market.price` (fill at current price)
5. `market.open(e, position, id)` -- same as market order step 5
6. Distribute opening fees: vault, treasury, caller (keeper incentive)
7. Emit `FillLimit` event

### 4.4 Close Position (User)

**Entry point:** `Trading::close_position()` in `trading/src/contract.rs`
**Implementation:** `execute_close_position()` in `trading/src/trading/actions.rs`

Steps:
1. `require_can_manage(e)` -- contract must not be Frozen
2. `position.user.require_auth()`
3. `position.require_closable(e)` -- must be filled AND past `MIN_OPEN_TIME` (30 seconds)
4. Verify `price_data.feed_id == position.feed`
5. `Market::load()` and `market.close()`:
   a. `position.settle(e, market)` -- compute PnL and all fees
   b. Update market stats with negative notional
   c. `total_notional -= position.notional`
   d. Remove position from storage
6. Compute payouts:
   - `user_payout = settlement.equity(col).max(0)` -- user cannot receive less than 0
   - `treasury_fee = treasury.get_fee(protocol_fee)`
   - `vault_transfer = col - user_payout - treasury_fee`
7. Execute transfers (vault pays via `strategy_withdraw` if losing, contract pays vault if winning)
8. Emit `ClosePosition` event

### 4.5 Keeper Triggers (Stop Loss, Take Profit, Liquidation)

**Entry point:** `Trading::execute()` in `trading/src/contract.rs`
**Implementation:** `execute_trigger()` and `apply_*()` functions in `trading/src/trading/execute.rs`

**Stop Loss** (`ExecuteRequestType::StopLoss`):
1. Check `position.check_stop_loss(market.price)` -- long: price <= SL, short: price >= SL
2. `handle_close()` -- same close flow as user close
3. Keeper receives `caller_rate` of trading fees
4. Emit `StopLoss` event

**Take Profit** (`ExecuteRequestType::TakeProfit`):
1. Check `position.check_take_profit(market.price)` -- long: price >= TP, short: price <= TP
2. `handle_close()` -- same close flow
3. Keeper receives `caller_rate` of trading fees
4. Emit `TakeProfit` event

**Liquidation** (`ExecuteRequestType::Liquidate`):
1. Must be filled position
2. `market.close()` -- compute settlement
3. Check `equity < liq_threshold` where `liq_threshold = notional * liq_fee`
4. User gets nothing -- remaining equity goes as liquidation bonus
5. Revenue = `protocol_fee + liq_fee`, capped at collateral
6. Vault receives `col - treasury_fee - caller_fee`
7. Emit `Liquidation` event

### 4.6 Cancel Limit Order

**Entry point:** `Trading::cancel_limit()` in `trading/src/contract.rs`
**Implementation:** `execute_cancel_limit()` in `trading/src/trading/actions.rs`

Steps:
1. `require_can_manage(e)` -- contract must not be Frozen
2. `position.user.require_auth()`
3. Verify `!position.filled` -- can only cancel pending orders
4. Return full collateral to user (no fees charged for unfilled limits)
5. Remove position from storage
6. Emit `CancelLimit` event

### 4.7 Modify Collateral

**Entry point:** `Trading::modify_collateral()` in `trading/src/contract.rs`
**Implementation:** `execute_modify_collateral()` in `trading/src/trading/actions.rs`

Steps:
1. `require_can_manage(e)` -- contract must not be Frozen
2. `position.user.require_auth()`
3. Position must be filled
4. Compute `collateral_diff = new_collateral - position.col`; panic if zero
5. **Adding collateral** (`diff > 0`): transfer from user to contract
6. **Withdrawing collateral** (`diff < 0`): settle position, verify equity remains above margin requirement (`equity >= notional * margin`), then transfer to user
7. Persist updated position
8. Emit `ModifyCollateral` event

### 4.8 Set Triggers (Update TP/SL)

**Entry point:** `Trading::set_triggers()` in `trading/src/contract.rs`
**Implementation:** `execute_set_triggers()` in `trading/src/trading/actions.rs`

Steps:
1. `require_can_manage(e)`
2. `position.user.require_auth()`
3. Update `position.tp` and `position.sl`
4. `position.validate_triggers(e)` -- TP must be above entry for longs / below for shorts; SL opposite
5. Persist position
6. Emit `SetTriggers` event

---

## 5. Fee System

### 5.1 Base Fee (Opening and Closing)

**Source:** `trading/src/trading/market.rs` lines 84-88 (open), `trading/src/trading/position.rs` lines 168-172 (close/settle)

**Opening:**
```
if position is on the dominant side (after adding its notional):
    base_fee = notional * fee_dom / SCALAR_7       (ceil)
else:
    base_fee = notional * fee_non_dom / SCALAR_7   (ceil)
```

**Closing:**
```
if position side would still be dominant after removing its notional:
    base_fee = notional * fee_non_dom / SCALAR_7   (ceil)  -- rebalances, lower fee
else:
    base_fee = notional * fee_dom / SCALAR_7       (ceil)  -- worsens imbalance, higher fee
```

Note the inversion: closing from the dominant side gets the *lower* fee because it rebalances the market.

Variables:
- `fee_dom`: dominant-side fee rate (SCALAR_7), from `TradingConfig`; cap: `MAX_FEE_RATE = 100_000` (1%)
- `fee_non_dom`: non-dominant-side fee rate (SCALAR_7), from `TradingConfig`; cap: `MAX_FEE_RATE = 100_000` (1%)
- Constraint: `fee_dom >= fee_non_dom` (enforced in `validation.rs`)

### 5.2 Impact Fee

**Source:** `trading/src/trading/market.rs` line 89 (open), `trading/src/trading/position.rs` line 173 (close/settle)

```
impact_fee = notional / impact * SCALAR_7   (ceil division)
```

Implemented as:
```rust
let impact_fee = position.notional.fixed_div_ceil(e, &market.config.impact, &SCALAR_7);
```

Variables:
- `impact`: price impact divisor (SCALAR_7) from `MarketConfig`; minimum: `MIN_IMPACT = 100_000_000` (10 * SCALAR_7)
- At minimum impact, maximum fee is `notional / 10` = 10% of notional

### 5.3 Funding Fee (Peer-to-Peer)

**Source:** `trading/src/trading/rates.rs` -- `calc_funding_rate()`

The funding rate is a signed value indicating market imbalance:

```
funding_rate = r_funding * |L - S| / (L + S)
```

Where:
- `r_funding`: base hourly funding rate (SCALAR_18) from `TradingConfig`
- `L`: total long notional
- `S`: total short notional
- Result is positive when longs dominate (longs pay), negative when shorts dominate (shorts pay)

**Edge cases** (source: `rates.rs` lines 20-43):
- No positions on either side: rate = 0
- Only longs exist: rate = `+r_funding` (full base rate)
- Only shorts exist: rate = `-r_funding` (full base rate, negative)
- Both sides equal: rate = 0
- Naturally bounded in `[-r_funding, +r_funding]`

**Index accrual** (source: `trading/src/trading/market.rs` -- `MarketData::accrue()`, lines 194-221):

```
pay_delta = |fund_rate| * seconds / ONE_HOUR_SECONDS   (ceil)
recv_delta = pay_delta * pay_notional / recv_notional   (floor)
```

If `fund_rate > 0` (longs pay):
```
l_fund_idx += pay_delta     (longs pay more)
s_fund_idx -= recv_delta    (shorts receive -- negative index = credit)
```

If `fund_rate < 0` (shorts pay):
```
s_fund_idx += pay_delta     (shorts pay more)
l_fund_idx -= recv_delta    (longs receive)
```

Funding accrual is skipped if either side has zero notional (no counterparty to receive).

**Per-position funding fee at settlement** (source: `trading/src/trading/position.rs` line 175):
```
funding = notional * (current_fund_idx - position.fund_idx) / SCALAR_18   (floor)
```

**Key property:** Funding is pure peer-to-peer. The protocol takes zero cut. The treasury fee is only applied to `protocol_fee()` which includes base + impact + borrowing fees, but NOT funding.

### 5.4 Borrowing Fee (Dominant Side Only)

**Source:** `trading/src/trading/rates.rs` -- `calc_borrowing_rate()`

```
borrowing_rate = r_base * (1 + r_var * util^5) * r_borrow
```

Where:
- `r_base`: base hourly borrowing rate (SCALAR_18) from `TradingConfig`; cap: `MAX_RATE_HOURLY = 100_000_000_000_000` (0.01%/hr)
- `r_var`: variable multiplier at full utilization (SCALAR_7) from `TradingConfig`; cap: `MAX_R_VAR = 100_000_000` (10x)
- `util`: market utilization = `(l_notional + s_notional) / vault_balance` (SCALAR_7, clamped to SCALAR_7)
- `r_borrow`: per-market borrowing weight (SCALAR_7) from `MarketConfig`; 1e7 = 1x, 2e7 = 2x

**Implementation detail** (source: `rates.rs` lines 60-76):
```
util^5 is computed as: u2 = util * util; u4 = u2 * u2; u5 = u4 * util  (all ceil)
multiplier = SCALAR_7 + r_var * u5                                     (in SCALAR_7)
global_rate = r_base * multiplier                                      (ceil)
final_rate = global_rate * r_borrow                                    (ceil)
```

**Who pays** (source: `market.rs` lines 173-191):
- If `l_notional > s_notional`: only `l_borr_idx` advances (longs pay)
- If `s_notional > l_notional`: only `s_borr_idx` advances (shorts pay)
- If `l_notional == s_notional` and both > 0: both indices advance (both pay)

**Index accrual:**
```
borrow_delta = borr_rate * seconds / ONE_HOUR_SECONDS   (ceil)
dominant_borr_idx += borrow_delta
```

**Per-position borrowing fee at settlement** (source: `position.rs` line 176):
```
borrowing_fee = notional * (current_borr_idx - position.borr_idx) / SCALAR_18   (ceil)
```

### 5.5 Protocol Fee (Treasury)

**Source:** `treasury/src/lib.rs` -- `Treasury::get_fee()`

```
treasury_fee = protocol_fee * treasury_rate / SCALAR_7   (floor)
```

Where:
- `protocol_fee = base_fee + impact_fee + borrowing_fee` (excludes funding -- it's peer-to-peer)
- `treasury_rate`: set by treasury owner; max `SCALAR_7 / 2` = 50%

Source: `trading/src/trading/position.rs` -- `Settlement::protocol_fee()` (line 39-42).

### 5.6 Keeper Fee (Caller Rate)

**Source:** `trading/src/trading/execute.rs` lines 116-117, 155-156

```
caller_fee = trading_fee * caller_rate / SCALAR_7   (floor)
```

Where:
- `trading_fee = base_fee + impact_fee` (excludes funding and borrowing)
- `caller_rate`: from `TradingConfig`; cap: `MAX_CALLER_RATE = 5_000_000` (50%)

---

## 6. Index-Based Accrual

The protocol uses cumulative index tracking to avoid updating every position on every block.

### 6.1 How Indices Work

Each `MarketData` stores six cumulative indices:
- `l_fund_idx` / `s_fund_idx` -- funding indices for longs/shorts (SCALAR_18)
- `l_borr_idx` / `s_borr_idx` -- borrowing indices for longs/shorts (SCALAR_18)
- `l_adl_idx` / `s_adl_idx` -- ADL reduction indices for longs/shorts (SCALAR_18, start at 1.0)

Each `Position` snapshots three indices at fill time:
- `fund_idx` -- funding index at position fill
- `borr_idx` -- borrowing index at position fill
- `adl_idx` -- ADL index at position fill (starts at SCALAR_18)

### 6.2 Settlement Calculation

At close/settle (source: `position.rs` -- `Position::settle()`):

```
funding    = notional * (current_fund_idx - position.fund_idx) / SCALAR_18   (floor)
borrowing  = notional * (current_borr_idx - position.borr_idx) / SCALAR_18   (ceil)
```

The diff-based approach means no position state needs updating when rates accrue -- only the global market indices advance.

### 6.3 Accrual Trigger

Indices accrue on every market operation via `Market::load()` which calls `MarketData::accrue()`. This ensures indices are always current before any position operation.

The `apply_funding` action accrues indices AND recalculates the funding rate based on current open interest. It can only be called once per hour (enforced by `ONE_HOUR_SECONDS` check).

---

## 7. Settlement Math

### 7.1 PnL Calculation

**Source:** `trading/src/trading/position.rs` -- `Position::settle()`, lines 154-164

```
price_diff = (market.price - entry_price)     for longs
             (entry_price - market.price)     for shorts

pnl = notional * price_diff / entry_price / price_scalar   (floor)
```

Implemented as:
```rust
let ratio = price_diff.fixed_div_floor(e, &self.entry_price, &market.price_scalar);
let pnl = self.notional.fixed_mul_floor(e, &ratio, &market.price_scalar);
```

### 7.2 ADL Adjustment (Before PnL)

Before PnL is calculated, the position's notional is adjusted for any ADL that occurred since the position was opened:

```
if position.adl_idx != current_adl_idx:
    position.notional = notional * current_adl_idx / position.adl_idx   (floor)
    position.adl_idx = current_adl_idx
```

This proportionally reduces the notional of winning positions to match the vault's capacity.

Source: `position.rs` lines 148-151.

### 7.3 Equity Formula

**Source:** `trading/src/trading/position.rs` -- `Settlement::equity()`, line 20-22

```
equity = collateral + pnl - total_fee
total_fee = base_fee + impact_fee + funding + borrowing_fee
```

### 7.4 Net PnL (Clamped)

**Source:** `position.rs` -- `Settlement::net_pnl()`, lines 29-31

```
net_pnl = (pnl - total_fee).max(-collateral)
```

A trader cannot lose more than their collateral. The vault absorbs the difference.

### 7.5 Payout Calculation

```
user_payout = equity.max(0)       -- never negative
vault_transfer = col - user_payout - treasury_fee
```

If `vault_transfer < 0`: vault pays via `strategy_withdraw()` (trader profited beyond their collateral).
If `vault_transfer > 0`: contract pays vault (trader lost some or all collateral).

### 7.6 Liquidation Threshold

```
liq_threshold = notional * liq_fee / SCALAR_7   (floor)
```

Position is liquidatable when `equity < liq_threshold`. The liquidation fee (`liq_fee`) from `MarketConfig` serves as both the threshold and the reward pool.

Source: `execute.rs` lines 246-251.

---

## 8. Auto-Deleveraging (ADL)

### 8.1 Circuit Breaker Thresholds

**Source:** `trading/src/constants.rs`

```
UTIL_ONICE  = 9_500_000  (95% in SCALAR_7) -- enter OnIce when net PnL >= 95% of vault
UTIL_ACTIVE = 9_000_000  (90% in SCALAR_7) -- restore Active when net PnL < 90% of vault
```

The 5% gap (hysteresis) prevents flapping between Active and OnIce states.

### 8.2 Status Transitions

**Source:** `trading/src/trading/adl.rs` -- `execute_update_status()`

```
Active + net_pnl >= onice_line  -->  ADL triggered, status -> OnIce
OnIce  + net_pnl <  active_line -->  status -> Active (recovery)
OnIce  + net_pnl >= active_line -->  ADL triggered again, stay OnIce
```

Where:
- `onice_line = vault_balance * UTIL_ONICE / SCALAR_7`
- `active_line = vault_balance * UTIL_ACTIVE / SCALAR_7`

`update_status` is permissionless -- anyone can call it with current price data.

### 8.3 ADL Mechanism

**Source:** `trading/src/trading/adl.rs` -- `do_adl()`, lines 66-124

When ADL is triggered:

1. **Compute per-side PnL for each market:**
   ```
   long_pnl  = price * l_entry_wt / price_scalar - l_notional
   short_pnl = s_notional - price * s_entry_wt / price_scalar
   ```

2. **Compute aggregate PnL:**
   ```
   net_pnl = sum of (long_pnl + short_pnl) across all markets
   total_winner_pnl = sum of positive-side PnLs only
   ```

3. **Compute reduction factor:**
   ```
   deficit = net_pnl - vault_balance
   reduction_pct = deficit / total_winner_pnl   (floor, capped at SCALAR_18)
   factor = SCALAR_18 - reduction_pct
   ```

4. **Apply reduction to winning sides:**
   For each market where a side has positive PnL:
   ```
   side_notional = side_notional * factor / SCALAR_18   (floor)
   side_entry_wt = side_entry_wt * factor / SCALAR_18   (floor)
   side_adl_idx  = side_adl_idx  * factor / SCALAR_18   (floor)
   ```

5. **Update total notional and set status to OnIce**

### 8.4 How ADL Affects Individual Positions

When a position is settled after an ADL event, the position's notional is scaled down proportionally:

```
position.notional = notional * current_adl_idx / position.adl_idx   (floor)
```

This means the position's effective size is reduced, which reduces both its PnL and its fee obligations.

Source: `position.rs` lines 148-151.

---

## 9. Vault (ERC-4626)

### 9.1 Overview

**Source:** `strategy-vault/src/contract.rs`

The vault implements OpenZeppelin's `FungibleVault` trait (ERC-4626 compliant) with:
- A single `strategy` address (the trading contract) that can withdraw via `strategy_withdraw()`
- A `lock_time` that prevents depositors from transferring/redeeming shares for a configurable period after deposit

### 9.2 Deposit / Withdraw

- `deposit(assets, receiver, from, operator)` -- deposit assets, receive shares; records deposit timestamp
- `mint(shares, receiver, from, operator)` -- mint specific share amount; records deposit timestamp
- `withdraw(assets, receiver, owner, operator)` -- withdraw assets; requires lock expired
- `redeem(shares, receiver, owner, operator)` -- redeem shares; requires lock expired
- Share transfers also require lock expired (`transfer`, `transfer_from` overridden)

### 9.3 Strategy Withdraw

```rust
pub fn strategy_withdraw(e: Env, strategy: Address, amount: i128) {
    strategy.require_auth();
    StrategyVault::withdraw(&e, &strategy, amount);
}
```

Only the authorized strategy (trading contract) can call this. It decreases `total_assets` and share price.

### 9.4 Inflation Attack Mitigation

The vault uses `decimals_offset` (set at construction, range 0-10) following the OpenZeppelin virtual shares pattern. This makes the vault shares more granular than the underlying asset, making inflation attacks economically infeasible.

Source: constructor parameter in `strategy-vault/src/contract.rs`; referenced in threat model as T-ELEV-11.

---

## 10. Circuit Breaker

### 10.1 Contract Status States

**Source:** `trading/src/types.rs` -- `ContractStatus` enum

| Status | Value | Who Sets | Allowed Operations |
|--------|-------|----------|-------------------|
| `Active` | 0 | Constructor, `update_status` (recovery), admin | All operations |
| `OnIce` | 1 | `update_status` (ADL trigger) | Close, modify, cancel, trigger, execute -- NO new opens |
| `AdminOnIce` | 2 | Admin (`set_status`) | Close, modify, cancel, trigger, execute -- NO new opens |
| `Frozen` | 3 | Admin (`set_status`) | Nothing allowed |

### 10.2 Status Guard Functions

**Source:** `trading/src/validation.rs`

- `require_active(e)` -- only `Active` passes; used for `place_limit`, `open_market`
- `require_can_manage(e)` -- `Active`, `OnIce`, `AdminOnIce` pass; used for close, modify, cancel, execute, set_triggers

### 10.3 Admin vs Permissionless Status

- Admin can set `Active`, `AdminOnIce`, `Frozen` via `set_status()` (owner only)
- Admin CANNOT set `OnIce` directly (enforced in `config.rs` -- `execute_set_status`)
- `update_status()` is permissionless -- anyone can trigger it with price data
- `update_status()` can only transition between `Active` and `OnIce` based on PnL thresholds

---

## 11. Configuration Parameters

### 11.1 TradingConfig (Global)

**Source:** `trading/src/types.rs`, `trading/src/validation.rs`, `trading/src/constants.rs`

| Field | Unit | Valid Range | Purpose |
|-------|------|------------|---------|
| `caller_rate` | SCALAR_7 | `[0, 5_000_000]` (0-50%) | Keeper's share of trading fees |
| `min_notional` | token_decimals | `> 0`, must be `< max_notional` | Minimum position notional |
| `max_notional` | token_decimals | `> min_notional` | Maximum position notional |
| `fee_dom` | SCALAR_7 | `[0, 100_000]` (0-1%) | Fee rate for dominant side |
| `fee_non_dom` | SCALAR_7 | `[0, 100_000]` (0-1%) | Fee rate for non-dominant side; must be `<= fee_dom` |
| `max_util` | SCALAR_7 | `(0, 100_000_000]` (0-1000%) | Global notional / vault cap |
| `r_funding` | SCALAR_18 | `[0, MAX_RATE_HOURLY]` | Base hourly funding rate |
| `r_base` | SCALAR_18 | `[0, MAX_RATE_HOURLY]` | Base hourly borrowing rate |
| `r_var` | SCALAR_7 | `[0, 100_000_000]` (0-10x) | Borrowing multiplier at full utilization |

### 11.2 MarketConfig (Per-Market)

| Field | Unit | Valid Range | Purpose |
|-------|------|------------|---------|
| `enabled` | bool | true/false | Whether market accepts new positions |
| `max_util` | SCALAR_7 | `(0, 100_000_000]` (0-1000%) | Per-market notional / vault cap |
| `r_borrow` | SCALAR_7 | `[0, 100_000_000]` (0-10x) | Per-market borrowing weight multiplier |
| `margin` | SCALAR_7 | `(0, 5_000_000]` (0-50%) | Initial margin; max leverage = 1/margin |
| `liq_fee` | SCALAR_7 | `(0, 2_500_000]` (0-25%) | Liquidation threshold + fee; must be `< margin` |
| `impact` | SCALAR_7 | `>= 100_000_000` (>= 10) | Price impact divisor; higher = lower impact fee |

### 11.3 Validation Invariants

From `trading/src/validation.rs`:
- `fee_dom >= fee_non_dom` -- ensures close-from-dominant-side discount is always real
- `margin > liq_fee` -- ensures positions cannot be liquidated at max leverage immediately
- All rate fields are non-negative
- `max_util > 0` (both global and per-market)

### 11.4 Constants

| Constant | Value | Unit | Purpose |
|----------|-------|------|---------|
| `SCALAR_7` | `10_000_000` | -- | 7-decimal fixed-point precision |
| `SCALAR_18` | `10^18` | -- | 18-decimal fixed-point precision |
| `MAX_ENTRIES` | `50` | count | Maximum markets or positions per user |
| `UTIL_ONICE` | `9_500_000` | SCALAR_7 | 95% -- enter OnIce threshold |
| `UTIL_ACTIVE` | `9_000_000` | SCALAR_7 | 90% -- restore Active threshold |
| `ONE_HOUR_SECONDS` | `3600` | seconds | Funding rate update interval |
| `MIN_OPEN_TIME` | `30` | seconds | Minimum time before a position can be closed |
| `MAX_FEE_RATE` | `100_000` | SCALAR_7 | 1% max fee rate |
| `MAX_CALLER_RATE` | `5_000_000` | SCALAR_7 | 50% max keeper share |
| `MAX_RATE_HOURLY` | `10^14` | SCALAR_18 | 0.01%/hr max rate |
| `MAX_R_VAR` | `100_000_000` | SCALAR_7 | 10x max borrowing multiplier |
| `MAX_UTIL` | `100_000_000` | SCALAR_7 | 1000% max utilization cap |
| `MIN_IMPACT` | `100_000_000` | SCALAR_7 | 10x minimum impact divisor |
| `MAX_MARGIN` | `5_000_000` | SCALAR_7 | 50% max margin (2x min leverage) |
| `MAX_LIQ_FEE` | `2_500_000` | SCALAR_7 | 25% max liquidation fee |
| `MAX_R_BORROW` | `100_000_000` | SCALAR_7 | 10x max borrowing weight |

---

## 12. Known Trust Assumptions

### 12.1 Oracle Signer Trust

**What:** The protocol trusts the Pyth Lazer oracle signer to provide accurate, timely price data.

**Mitigation:**
- `ed25519_verify()` in `price-verifier/src/pyth.rs` -- every price update is signature-verified against a `trusted_signer` public key stored on-chain
- `max_staleness` check -- prices older than threshold are rejected
- `max_confidence_bps` check -- prices with excessive confidence intervals are rejected
- The trusted signer can be rotated by the price verifier owner (`update_trusted_signer`)

**Residual risk:** If the Pyth Lazer signer is compromised, false prices could be submitted. Mitigation is limited to confidence and staleness bounds. See T-SPOOF-03 in threat model.

### 12.2 Keeper Liveness

**What:** The protocol relies on off-chain keepers for: liquidation execution, stop loss/take profit triggering, limit order fills, and `apply_funding` calls.

**Mitigation:**
- Keeper actions are permissionless -- anyone can run a keeper
- Keeper incentives via `caller_rate` (share of trading fees)
- `apply_funding` is also permissionless (no keeper authorization needed)

**Residual risk:** If no keepers are active, positions may not be liquidated in time, funding rates may not update, and triggers may not fire. The vault absorbs the loss from delayed liquidations. See T-DOS-02 in threat model.

### 12.3 Admin / Timelock Trust

**What:** The admin (via governance timelock) can modify trading configuration, market parameters, and contract status.

**Mitigation:**
- Governance contract enforces a configurable time delay before queued changes take effect
- Status changes are immediate but constrained: admin cannot set `OnIce` (only the permissionless `update_status` can)
- Config validation bounds prevent extreme parameters (all caps in `constants.rs`)

**Residual risk:** Admin can freeze the contract, preventing all operations. Admin can set unfavorable parameters (within bounds). Timelock delay gives users time to react. See T-ELEV-05 in threat model.

### 12.4 Out-of-Scope Services

The following services interact with the contracts but are NOT verified on-chain:

- **Relayer** (transaction submission) -- trusted to relay transactions faithfully; cannot modify transaction content (Stellar signatures)
- **Backend** (API) -- reads on-chain state; does not have privileged access
- **Indexer** (event processing) -- reads events; does not affect contract state
- **Frontend** (UI) -- constructs transactions for user signing; users verify in wallet

These services can fail (denial of service) but cannot corrupt contract state.
