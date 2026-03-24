# Codebase Concerns

**Analysis Date:** 2026-03-24

## Out-of-Sync Tests

**Integration Test Suite (test-suites) Incompatibility:**
- Issue: `test-suites/tests/` uses old oracle pattern and mismatched trading API
- Files:
  - `test-suites/tests/test_trading_liquidations.rs`
  - `test-suites/tests/test_trading_adl.rs`
  - `test-suites/tests/test_trading_pnl.rs`
  - `test-suites/tests/test_trading_position.rs`
  - `test-suites/tests/test_trading_proptest.rs`
  - `test-suites/tests/test_cost_profile.rs`
- Blocks: Cannot run integration tests reliably; hidden failures in position settlement, ADL logic, and fee calculations
- Risk: Changes to trading API or fee logic could break unexpectedly
- Test coverage: Integration tests do not reflect current contract state
- Recommendation: Migrate tests to use `PriceData` struct via price verifier; update fixtures to match current trading interface

## Unsafe Unwrap Calls

**Unchecked Vector Access:**
- Issue: Two critical `.unwrap()` calls on vector access with no error handling
- Files:
  - `trading/src/contract.rs` line 24: `.get(0).unwrap()` in `verify_price()`
  - `trading/src/trading/adl.rs` line 31: `.find(...).unwrap()` in market PnL calculation
  - `trading/src/trading/adl.rs` line 86: `.get(i).unwrap()` in ADL loop
- Cause: Assumes price data always present; assumes market prices always in feed list
- Impact: Contract panics if price feed data missing or feed_id not in price list during ADL execution
- Fix approach: Replace with proper error handling—return `TradingError::InvalidPrice` if price data missing or feed not found
- Priority: High (production-blocking bug)

**In contract.rs verify_price():**
```rust
// Current (panics if empty)
let price = prices.get(0).unwrap();

// Should be
let price = prices.get(0).ok_or(TradingError::InvalidPrice)?;
```

**In adl.rs market lookup:**
```rust
// Current (panics if feed not found)
.find(|f| f.feed_id == feed_id)
.unwrap();

// Should be
.find(|f| f.feed_id == feed_id)
.ok_or(TradingError::InvalidPrice)?;
```

## Price Verifier Assumptions

**No Fallback on Empty Price Batch:**
- Issue: `verify_price()` and `verify_prices()` assume Pyth response always contains price for requested feed
- Files: `price-verifier/src/lib.rs` lines 39-55
- Trigger: Oracle unavailability or delayed Pyth Lazer update
- Workaround: None—contract must be paused by guardian until oracle recovers
- Recommendation: Add fallback mechanism (cached price or price oracle fallback). Document that keeper cannot execute until Pyth recovers.

**Max Confidence Not Enforced in Trading:**
- Issue: Price verifier checks `max_confidence_bps` but trading contract ignores confidence metadata
- Files: `price-verifier/src/pyth.rs` (enforced), `trading/src/contract.rs` (not used)
- Impact: Trading could proceed with low-confidence prices even if verifier rejects them
- Fix: Ensure price verifier is sole gatekeeper; trading never calls Pyth directly

## Forced Trade-Off: Validator Index Access

**ADL and Fee Calculation Loop Panics:**
- Issue: ADL loop uses `.get(i).unwrap()` instead of iterator
- Files: `trading/src/trading/adl.rs` lines 85-86
- Why: Soroban Vec doesn't support index iteration without unsafe unwrap
- Risk: If cached Vec is modified during loop (unlikely but possible in future refactoring), panics
- Mitigation: Add comment explaining this is safe because Vec is not modified during loop

## Integer Overflow Risks

**Fee/Rate Accumulation Without Overflow Protection:**
- Issue: Borrowing and funding rate indices accumulate via `+=` without checked arithmetic
- Files: `trading/src/trading/market.rs` lines 183-191 (borrowing index), lines 199-208 (funding index)
- Trigger: Very long-lived positions (>100 years) with high rates
- Likelihood: Negligible in practice (perpetual futures typically wind down)
- Mitigation: None needed for testnet; document that max position lifetime should be enforced by governance

**Notional Addition to Total:**
- Issue: `total_notional += position.notional` has no overflow check
- Files: `trading/src/trading/market.rs` line 98
- Trigger: Many concurrent positions at max notional
- Impact: Integer wrap (unlikely given i128 range ~10^38, market cap constraints)
- Mitigation: Utilization checks (require within vault balance) implicitly cap total notional

## Configuration Safety

**No Validation of Token Decimals:**
- Issue: `min_notional` and `max_notional` use SCALAR_7 but contract doesn't verify token decimals
- Files: `trading/src/validation.rs` line 52
- Risk: If collateral token is not 7 decimals, notional boundaries are wrong
- Example: 8-decimal token with min_notional = 1e7 becomes 0.1 USDT instead of 1 USDT
- Fix approach: In `__constructor`, validate token decimals match assumptions or accept decimals parameter
- Recommendation: Add assertion that token.decimals() == 7 in constructor; document this requirement in CLAUDE.md

**No Upper Bound on Configuration:**
- Issue: `set_market_config()` doesn't validate that `margin > liq_fee` (relies on factory pre-check)
- Files: `trading/src/validation.rs` line 86
- Risk: If owner calls `set_market_config()` directly (not via factory), config can be invalid
- Fix: Add validation to `set_market_config` in contract, don't rely on factory pre-checks

## Storage Access Panics

**Market/Position Lookup Assumes Existence:**
- Issue: All `storage::get_*` calls use `unwrap_optimized()` without fallback
- Files: `trading/src/storage.rs` throughout (lines 81, 94, 104, 117, 130, 143)
- Risk: If instance storage corrupted or market removed, any get_market_config call panics
- Mitigation: Instance storage very stable; market removal validated before deletion
- Recommendation: Add `?` return type to getter functions; let callers handle missing data

## Collateral Validation Gap

**Position Collateral Can Become Negative:**
- Issue: `position.col -= base_fee + impact_fee` not validated after deduction
- Files: `trading/src/trading/market.rs` line 91
- Scenario: If fee exceeds collateral (edge case in very tight markets), col becomes negative
- Fix: Add validation after fee deduction: `if position.col < 0 { panic!(InsufficientCollateral) }`

## Position Closure Timing Lock

**MIN_OPEN_TIME Prevents Emergency Liquidation:**
- Issue: `require_closable()` forbids closing positions opened <30s ago, even if liquidatable
- Files: `trading/src/trading/position.rs` line 101-108
- Scenario: Position quickly becomes underwater; keeper must wait 30s before liquidating
- Impact: User loss extends due to artificial delay; funding costs accrue
- Fix approach: Remove MIN_OPEN_TIME check for liquidations only; allow immediate SL/TP regardless
- Recommendation: Split `require_closable()` into separate checks for regular close vs liquidation

## ADL Determinism

**Price Feed Order Affects ADL Outcome:**
- Issue: ADL uses `.find()` to locate price for each market; order in Vec matters
- Files: `trading/src/trading/adl.rs` line 27-31
- Risk: If keeper submits same prices in different order, different market might trigger ADL first
- Mitigation: ADL applies to all markets with positive PnL, order irrelevant to outcome
- Recommendation: No fix needed; document that ADL is market-order-independent

## Fragile Areas Requiring Careful Testing

**Market Data Accrual State Machine:**
- Files: `trading/src/trading/market.rs` lines 162-210 (MarketData::accrue)
- Why fragile: Complex interaction between borrowing indices, funding rates, ADL indices
- Changes to accrue logic affect all fee calculations retroactively
- Safe modification: Add test cases for every accrue path before changing; verify indices remain ≥0

**Settlement PnL Calculation:**
- Files: `trading/src/trading/position.rs` lines 143-180 (settle)
- Why fragile: Must correctly apply 3 index types (funding, borrowing, ADL) in exact order
- Changes to index application order change user payouts
- Safe modification: Add unit tests for each index type; verify no negative payouts

**Utilization Check Logic:**
- Files: `trading/src/trading/market.rs` lines 57-71 (require_within_util)
- Why fragile: Uses ceiling division; off-by-one errors affect trading ability
- Changes to SCALAR_7 usage affect leverage limits
- Safe modification: Test at boundary: exactly at max_util, just below, just above

## Performance Bottlenecks

**Market Data Load Per Transaction:**
- Issue: Every trading action calls `Market::load()` which:
  1. Fetches all global config
  2. Fetches vault balance (cross-contract call)
  3. Fetches market data
  4. Accrues all indices
- Files: `trading/src/trading/market.rs` lines 30-55
- Trigger: High transaction throughput (>100 TPS)
- Impact: Vault balance fetch is blocking cross-contract call
- Improvement: Cache vault balance per block; update only in apply_funding() and after deposits
- Estimated improvement: 30-40% reduction in cross-contract calls

**Position Query Unbounded:**
- Issue: `get_user_positions()` returns Vec of all user positions with no pagination
- Files: `trading/src/storage.rs` (get_user_positions)
- Risk: User with 50+ positions can create tx that exceeds Soroban gas/size limits
- Fix: Add `offset` and `limit` parameters to getter; document MAX_ENTRIES = 50 per user
- Recommendation: Limit storage to 50 open positions per user; enforce in `Position::create()`

## Testing Coverage Gaps

**No Tests for Negative Fee Scenarios:**
- What's not tested: Positions closed at loss where borrowing fee > remaining equity
- Files: `trading/src/trading/position.rs` (settle)
- Risk: Could break silently if fee logic changes
- Priority: High

**No Tests for Max Leverage Boundary:**
- What's not tested: Positions exactly at max leverage; positions 1 wei over limit
- Files: `trading/src/trading/position.rs` line 96
- Risk: Precision loss in fixed-point math could allow over-leverage
- Priority: Medium

**ADL Triggering Untested:**
- What's not tested: Actual ADL triggering when net_pnl >= 95% of vault
- Files: `trading/src/trading/adl.rs` (do_adl)
- Risk: ADL logic could be broken in production
- Priority: Critical

**No Fuzz Tests for Fee Calculations:**
- What's not tested: Random combinations of rates, leverages, and hold times
- Files: `trading/src/trading/rates.rs`
- Risk: Overflow or precision bugs in rate curve
- Priority: High

## Scaling Limits

**Per-User Position Limit:**
- Current: MAX_ENTRIES = 50 (defined in `constants.rs`)
- Capacity: Can store 50 positions per user in persistent storage
- Scaling path: If exceeds 50, keeper must liquidate or close to make room
- Recommendation: Enforce at creation; return MaxPositionsReached error

**Market Count Limit:**
- Current: No hard limit; validation only checks if market already exists
- Risk: If 1000 markets added, `get_markets()` returns huge Vec
- Recommendation: Add MAX_MARKETS constant; enforce in `set_market()`

**Total Notional Ceiling:**
- Current: Global max_util enforced per transaction
- Scaling: Total notional capped at `vault_balance * max_util`
- If vault = 10M, max_util = 1000%, total notional can be 100M
- Safe: Governed by utilization checks; no overflow risk

## Dependency Risk

**soroban-fixed-point-math Crate Risk:**
- Used for: All fixed-point math (fees, rates, indices)
- Risk: If crate has precision bugs, all calculations affected
- Mitigation: Thoroughly tested; used in Blend protocol
- Recommendation: Pin to specific version; don't auto-update

**stellar_access/ownable Pattern:**
- Used for: Owner authorization
- Risk: If ownable trait has bug, contract not ownable
- Mitigation: Standard Stellar pattern; well-audited
- Recommendation: No action needed

## Error Message Leakage

**Contract Status Not Exposed in Errors:**
- Issue: Error enum doesn't distinguish between OnIce and Frozen states in user-facing errors
- Files: `trading/src/errors.rs` lines 41-44
- Risk: Frontend cannot determine if contract is recoverable (OnIce) or not (Frozen)
- Fix: Add separate error types: `ContractOnIce` vs `ContractFrozen` (already done—no action needed)

## Documentation Gaps

**MIN_LEVERAGE Undocumented:**
- Issue: Code uses `MIN_LEVERAGE = 2` but not documented what this means
- Files: `trading/src/constants.rs` (not present; inferred from code)
- Actually: Code uses `margin` field (inverse of leverage); MIN_LEVERAGE not directly used
- Recommendation: Document that `margin` is 1 / max_leverage; 50% margin = 2x max leverage

**Factory Deploy Order Critical But Not Documented:**
- Issue: Factory deploys vault first, trading second (order matters for address precomputation)
- Files: `factory/src/lib.rs` lines 76-86
- If changed: Contract will break because trading_address is passed to vault constructor
- Recommendation: Add inline comment: "Vault must be deployed before trading (vault_address passed to trading constructor)"

**No Architecture Doc for Multi-Index System:**
- Issue: Three indices (funding, borrowing, ADL) accumulate simultaneously; interaction not documented
- Risk: Future refactoring could break index ordering
- Recommendation: Add ARCHITECTURE.md explaining index lifecycle

## Known Workarounds and Limitations

**Tunnel URL Changes on Restart:**
- Issue: `cloudflared` tunnel URL changes every restart
- Workaround: Delete + recreate Mercury webhook after each tunnel restart
- Documentation: Documented in CLAUDE.md; no code fix possible
- Impact: Dev environment setup is manual and error-prone

**Contract Pause Cannot Be Partial:**
- Issue: `set_status()` sets status to Frozen globally
- Limitation: Cannot pause specific markets or users
- Workaround: Set market.enabled = false to disable trades on specific market
- Recommendation: Document workaround in interface

---

*Concerns audit: 2026-03-24*
