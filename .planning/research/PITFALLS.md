# Domain Pitfalls: DeFi/Soroban Audit Preparation

**Domain:** Perpetual futures protocol audit preparation (Soroban/Stellar)
**Researched:** 2026-03-24
**Overall confidence:** HIGH (grounded in codebase analysis + Soroban-specific audit literature + DeFi exploit history)

---

## Critical Pitfalls

Mistakes that cause audit failure, extended timelines, or critical/high findings that block deployment.

---

### Pitfall 1: Authorization Tests Are Completely Absent

**What goes wrong:** Every test in the codebase uses `mock_all_auths()` or `mock_all_auths_allowing_non_root_auth()`. Zero tests verify that unauthorized callers are actually rejected. Auditors will flag this as a systemic gap because it means the `require_auth()` calls on all 6 user-facing functions, the `#[only_owner]` guards on admin functions, and the `strategy.require_auth()` on vault withdrawals have never been tested in an adversarial context.

**Why it happens:** During development, `mock_all_auths()` is the fastest way to get tests passing. Teams intend to add auth-negative tests later but never do. The contracts "work" in tests, creating false confidence.

**Consequences:**
- Auditor finding: HIGH severity -- "Authorization not tested; cannot verify access control." This alone can delay sign-off.
- A missing `require_auth()` on `execute()` has already been identified in the STRIDE report (Spoof.3) but remains unfixed. Without auth tests, more gaps could exist.
- The `caller` parameter in `execute()` accepts any address without `require_auth()` -- if future logic uses `caller` for anything beyond fee routing, this becomes exploitable.

**Prevention:**
1. For every function that calls `require_auth()`, add a negative test that calls the function from an unauthorized address and asserts it panics with the expected error.
2. Remove `mock_all_auths()` from at least one integration test per critical flow and use explicit auth entries instead.
3. Test the specific Spoof.3 scenario: call `execute()` with a different `caller` than the transaction submitter and verify behavior is as intended.
4. Use Soroban's `AuthorizedInvocation` testing helpers to verify the exact auth tree for multi-contract calls (trading -> vault -> token).

**Detection:** Search for `mock_all_auths` -- if 100% of tests use it, auth is untested. Search for `should_panic` tests that assert authorization errors -- if none exist, this pitfall is present.

**Phase:** Integration testing phase (rebuild test-suites). Must be addressed before audit submission.

**Confidence:** HIGH -- verified by grep: 18 occurrences of `mock_all_auths`, 0 occurrences of `verify_auths` or auth assertion patterns.

---

### Pitfall 2: Unsafe Unwrap Calls in Production Paths

**What goes wrong:** The trading contract has `.unwrap()` calls in production code paths that will cause unrecoverable panics (not `panic_with_error!`) if inputs are unexpected. Auditors classify bare `unwrap()` in production Soroban code as MEDIUM-HIGH because the panic produces an opaque error rather than a descriptive contract error, and it indicates unvalidated assumptions.

**Why it happens:** Developers assume certain data will always be present (e.g., price feed always returns data, market feed_id always exists in the price list). These assumptions hold in happy-path testing but fail under adversarial or degraded conditions.

**Specific instances in Zenex:**
- `contract.rs:25` -- `verify_price()` calls `.get(0).unwrap()` on the price verifier response. If the verifier returns an empty Vec (e.g., oracle failure), the contract panics with no error context.
- `adl.rs:31` -- `.find(|f| f.feed_id == feed_id).unwrap()` in ADL. If a keeper submits prices missing one market's feed, the contract panics.
- `adl.rs:86` -- `.get(i).unwrap()` in the ADL loop. Safe in current logic but flagged by Scout as `unsafe-unwrap`.
- `price-verifier/lib.rs:42` -- `prices.get(0).unwrap()` in `verify_price()`. Same issue at the verifier layer.
- `price-verifier/pyth.rs:51` -- `update_data.get(i as u32).unwrap()` during byte extraction.
- All `storage.rs` getters using `unwrap_optimized()` -- these are less risky because they follow Soroban convention for instance storage, but auditors will still flag them.

**Consequences:**
- CoinFabrik Scout will flag every instance (it has a dedicated `unsafe-unwrap` detector).
- Auditors will write MEDIUM findings for each production `unwrap()`, consuming audit bandwidth on mechanical issues instead of deep logic review.
- Under oracle degradation, the contract becomes unusable with no meaningful error message.

**Prevention:**
1. Replace all `.unwrap()` in non-test code with `.ok_or()` or `unwrap_or_else(|| panic_with_error!(...))`.
2. Run `cargo-scout-audit` (CoinFabrik Scout for Soroban) before submission -- it will catch these automatically.
3. For `unwrap_optimized()` on instance storage, add an inline comment explaining why the panic is acceptable (data set in constructor, never removed). Auditors accept documented `unwrap_optimized()` for immutable instance storage.

**Detection:** `grep -rn "\.unwrap()" --include="*.rs" | grep -v test | grep -v "#\[cfg(test)\]"` -- any hits in `src/` (excluding `testutils.rs`) are audit findings.

**Phase:** Pre-audit cleanup (code freeze exception for error handling improvements). This is mechanical, does not change logic, and should be done before audit engagement begins.

**Confidence:** HIGH -- verified by direct code inspection.

---

### Pitfall 3: Integration Test Suite Is Non-Functional

**What goes wrong:** The `test-suites/` integration tests are out of sync with the current trading API. They use the old oracle pattern and cannot compile against the current contract interfaces. This means there are zero working integration tests covering multi-contract flows (trading + vault + price verifier + factory). Auditors will refuse to begin or will significantly discount their confidence if the project has no working integration tests.

**Why it happens:** The trading contract underwent API changes (new oracle pattern with `PriceData` struct, new fee system) but the integration tests were not updated in parallel. Development focused on unit tests within individual crates.

**Consequences:**
- **Audit delay:** Many firms require a passing test suite before starting. OpenZeppelin's readiness guide states "at least 90% code coverage." A non-functional integration suite means coverage is artificially low and untested multi-contract interactions.
- **Missed cross-contract bugs:** The individual unit tests mock external contracts. Real integration issues (vault balance discrepancy, price scalar mismatch between verifier and trading, factory deployment order) are only caught by integration tests.
- **Regression risk:** Without integration tests, the code freeze has no regression safety net. Documentation changes that accidentally touch logic have no backstop.
- **Fuzz tests also out of sync:** `test-suites/fuzz/` targets are similarly broken, meaning no property-based testing exists.

**Prevention:**
1. Rebuild `test-suites/` from scratch using the current API. Start with the `TestFixture` (already partially updated in `test_fixture.rs`) and write tests for: open -> fill -> close (profit/loss), liquidation, ADL triggering, funding accrual, multi-market scenarios.
2. Prioritize coverage of the CONCERNS.md items: negative fee scenarios, max leverage boundary, ADL triggering, collateral-goes-negative edge case.
3. Add at least one happy-path and one adversarial test per public contract function.
4. Ensure `make test` passes with zero failures before engaging auditors.

**Detection:** Run `cargo test --workspace 2>&1 | grep -E "FAILED|error"` -- any compilation errors or test failures in `test-suites` indicate this pitfall.

**Phase:** Integration testing phase. This is the single highest-priority item for audit readiness.

**Confidence:** HIGH -- explicitly documented in PROJECT.md and CONCERNS.md.

---

### Pitfall 4: Collateral Can Go Negative After Fee Deduction

**What goes wrong:** In `market.rs` line 91, `position.col -= base_fee + impact_fee` is applied without checking whether collateral remains non-negative after deduction. If opening fees exceed deposited collateral (possible with high-leverage, small-collateral positions in volatile markets), the collateral becomes negative. The position then has more debt than equity from the moment it opens.

**Why it happens:** The validation checks (`Position::validate`) verify that collateral meets margin requirements *before* fees are deducted. There is no post-fee validation.

**Consequences:**
- **Critical severity finding:** A position with negative collateral is immediately insolvent. If it gets filled and then closed, the settlement calculation adds PnL to a negative collateral base, potentially paying out the vault when the position should have been rejected.
- **Vault drain vector:** An attacker could intentionally open positions where fees consume all collateral, creating positions that are immediately liquidatable but have negative equity -- the vault absorbs the loss.
- **Cascading with ADL:** Negative-collateral positions distort the net PnL calculation used for ADL triggering.

**Prevention:**
1. Add a post-fee collateral check: `if position.col < min_col_after_fee { panic_with_error!(e, TradingError::InsufficientCollateral) }`.
2. Or: compute fees first, deduct from collateral, then run the existing validation against the reduced collateral.
3. Add a test that opens a position where `base_fee + impact_fee >= collateral` and verify it is rejected.

**Detection:** Review all paths that modify `position.col` and verify a non-negativity check follows.

**Phase:** Pre-audit cleanup (this is a code fix, technically breaks code freeze, but it is a bug fix not a feature).

**Confidence:** HIGH -- identified in CONCERNS.md with specific code location.

---

### Pitfall 5: MIN_OPEN_TIME Blocks Emergency Liquidation

**What goes wrong:** `require_closable()` enforces a 30-second minimum hold time before any position can be closed, including liquidations. If a position becomes deeply underwater within those 30 seconds (flash crash, oracle manipulation), the keeper cannot liquidate it, and losses extend to the vault.

**Why it happens:** `MIN_OPEN_TIME` was designed to prevent sandwich attacks on position creation. But the same check is applied to all close types without distinguishing user-initiated closes from keeper-initiated liquidations.

**Consequences:**
- **High severity finding:** Auditors will flag this as a design flaw in the liquidation mechanism. In a fast-moving market, 30 seconds can mean the difference between a controlled liquidation and a bad-debt event.
- **Vault insolvency risk:** During extreme volatility, multiple positions opened near the same time could all become insolvent simultaneously but be unliquidatable for 30 seconds.
- **Real-world precedent:** The Hyperliquid incident (March 2025, $4M loss) was caused partly by positions that could not be promptly liquidated.

**Prevention:**
1. Split `require_closable()` into two: `require_closable_by_user()` (keeps MIN_OPEN_TIME) and `require_closable_by_keeper()` (skips time check for liquidations).
2. Or: exempt liquidation calls from the MIN_OPEN_TIME check in the execute flow.
3. Add integration tests that open a position and immediately attempt liquidation (within 30s) to verify the keeper can liquidate.

**Detection:** Search for `require_closable` and verify it is not called in liquidation paths, or that liquidation has its own closability check.

**Phase:** Pre-audit cleanup (bug fix). This is a logic fix that should be classified as a bug, not a feature change.

**Confidence:** HIGH -- identified in CONCERNS.md; reinforced by Hyperliquid precedent.

---

### Pitfall 6: Token Decimal Assumption Not Enforced

**What goes wrong:** The entire contract assumes the collateral token has 7 decimals (matching SCALAR_7). The constructor does not validate this. If deployed with a token of different decimals (e.g., USDC with 6 decimals on some chains), all notional bounds, fee calculations, and collateral validation break silently -- positions appear 10x too large or 10x too small.

**Why it happens:** SCALAR_7 is hardcoded as the token scalar. The factory deploys with whatever token address is provided. There is no `token.decimals()` call in the constructor to verify the assumption.

**Consequences:**
- **Critical misconfiguration vulnerability:** Not an exploit per se, but a deployment footgun that auditors will flag as HIGH because it can cause immediate protocol insolvency if deployed with wrong token.
- **Config validation gap:** `require_valid_config` validates rate bounds but not the fundamental decimal assumption.

**Prevention:**
1. In `__constructor`, call `StellarAssetClient::new(e, &token).decimals()` and assert it equals 7.
2. Or: accept `token_decimals` as a constructor parameter and compute a `token_scalar` used throughout instead of hardcoding SCALAR_7.
3. Document the 7-decimal requirement prominently in the deployment guide and constructor NatSpec.

**Detection:** Search constructor for `decimals` check -- absence confirms this pitfall.

**Phase:** Pre-audit cleanup. One-line assertion fix.

**Confidence:** HIGH -- identified in CONCERNS.md.

---

## Moderate Pitfalls

Issues that result in MEDIUM findings or significant audit time consumption but don't block deployment.

---

### Pitfall 7: Incomplete Threat Model Documentation

**What goes wrong:** While the STRIDE threat model exists (`security-v2/`), auditors expect specific documentation artifacts that may be missing or incomplete:
- **Invariant specifications:** What must always be true? (e.g., "total vault withdrawals never exceed total deposits + net trader losses", "sum of all position notionals equals stored total_notional"). These are not formally documented.
- **Trust boundary diagram:** The STRIDE report describes trust boundaries in text but a visual diagram speeds auditor onboarding by 30%+ (per Hacken's preparation guide).
- **Function-level NatSpec:** Many public functions lack doc comments explaining purpose, preconditions, postconditions, and error conditions. Only `accrue()` and `settle()` have partial documentation.
- **Admin privilege catalog:** What can the owner do? Can they drain the vault? Change the oracle to a malicious one? The STRIDE report covers this but the contracts themselves lack inline documentation of admin capabilities and their risk implications.

**Why it happens:** Teams focus on writing code and tests, treating documentation as an afterthought. The STRIDE model was done separately and may not be cross-referenced in the code.

**Prevention:**
1. Write explicit invariants as doc comments on each contract's `__constructor` (what this contract guarantees).
2. Add `/// # Safety` or `/// # Panics` documentation to every public function.
3. Create a single-page admin privilege matrix showing every `#[only_owner]` function and its worst-case impact.
4. Cross-reference STRIDE findings in code: e.g., `// STRIDE: Spoof.3 -- caller is fee destination, not identity. See security-v2/01-spoofing.md`.

**Detection:** Run `cargo doc --no-deps` and review the generated documentation. If most functions show "No documentation available," this pitfall is present.

**Phase:** Documentation phase. Should run in parallel with integration test rebuilding.

**Confidence:** MEDIUM -- based on audit preparation best practices from OpenZeppelin, Hacken, and Quantstamp readiness guides.

---

### Pitfall 8: No Static Analysis or Linting Pre-Audit

**What goes wrong:** Teams submit code without running available static analysis tools. Auditors then spend the first 2-3 days of an engagement writing up findings that a tool would have caught automatically: unsafe unwraps, division-before-multiplication, assert! usage, outdated SDK version. This wastes audit hours and delays the deep logic review.

**Why it happens:** Teams are unaware of Soroban-specific tools or assume Rust's compiler catches everything.

**Specific tools for Soroban:**
- **CoinFabrik Scout** (`cargo-scout-audit`): 23 Soroban-specific detectors including `unsafe-unwrap`, `divide-before-multiply`, `overflow-check`, `unprotected-mapping-operation`, `dos-unbounded-operation`, `soroban-version`.
- **Clippy** with strict lints: Catches general Rust issues.
- **`cargo audit`**: Checks dependencies for known vulnerabilities.

**Prevention:**
1. Install and run Scout: `cargo install cargo-scout-audit && cargo scout-audit`.
2. Fix all CRITICAL and MEDIUM findings before audit.
3. Run `cargo clippy -- -D warnings` with no suppressions in production code.
4. Run `cargo audit` to check for vulnerable dependencies.
5. Include Scout report in audit submission package as evidence of internal diligence.

**Detection:** Ask "Have we run Scout?" If the answer is no or the team is unaware of it, this pitfall is active.

**Phase:** Pre-audit cleanup. Should be the first task -- it generates a mechanical fix list.

**Confidence:** HIGH -- Scout is the official Stellar/Soroban security tool, endorsed by the Stellar Development Foundation.

---

### Pitfall 9: Index Accumulation Without Overflow Guard

**What goes wrong:** Borrowing and funding indices accumulate via `+=` without checked arithmetic. While `overflow-checks = true` in the release profile catches this at runtime (panicking the transaction), it does so with an opaque error rather than a meaningful contract error. More importantly, if a position remains open for an extremely long period with high rates, index values could theoretically approach `i128::MAX`.

**Why it happens:** The Soroban fixed-point math library (`soroban-fixed-point-math`) returns `None` on overflow for its operations, but the raw `+=` on index accumulators does not use the library -- it uses native Rust `+=` which will panic in release mode (overflow-checks = true) or silently wrap in debug mode.

**Specific locations:**
- `market.rs:185-190` -- `self.l_borr_idx += borrow_delta` and `self.s_borr_idx += borrow_delta`
- `market.rs:215-219` -- `self.l_fund_idx += pay_delta` and `self.s_fund_idx -= recv_delta`
- `market.rs:174` -- `self.l_notional + self.s_notional` (no overflow check)

**Consequences:**
- **MEDIUM finding:** Auditors will note that while `overflow-checks = true` prevents silent wrapping, the panic produces an unrecoverable error with no context. The contract becomes stuck if any index overflows.
- **Practical risk is low:** i128 range is ~1.7 x 10^38. At SCALAR_18 (10^18), this allows indices up to ~10^20 before overflow. Even at 100% hourly rates, it would take millennia. But auditors still flag it because "low probability" is not "impossible."

**Prevention:**
1. Replace `+=` with `checked_add().unwrap_or_else(|| panic_with_error!(e, TradingError::Overflow))`.
2. Or: document explicitly in code comments why overflow is practically impossible and provide the math proof. Auditors accept this if the analysis is rigorous.
3. Consider adding a `MAX_INDEX` constant and capping accumulation.

**Detection:** Search for `+=` and `-=` on index fields in production code.

**Phase:** Pre-audit cleanup (minor code change) or documentation phase (if choosing to document rather than fix).

**Confidence:** MEDIUM -- `overflow-checks = true` mitigates the worst case. The risk is theoretical, but auditors will still report it.

---

### Pitfall 10: Position TTL Archival Could Orphan Positions

**What goes wrong:** Position storage has a 14-day TTL threshold and 21-day bump. If a position is opened and the user does not interact with it for 21+ days, the position's persistent storage entry could be archived. When the user or keeper tries to close or liquidate, the transaction fails because the entry must be restored first. This is Soroban-specific and has no EVM equivalent.

**Why it happens:** Soroban's state archival is unique to Stellar. Developers from EVM backgrounds don't think about TTL management. The storage TTL is set conservatively (21 days) but perpetual positions can theoretically be held much longer.

**Consequences:**
- **MEDIUM finding:** Positions that outlive their TTL become temporarily inaccessible. While Protocol 23 added automatic restoration via simulation, this adds transaction complexity and cost.
- **Keeper disruption:** Liquidation bots that don't handle restoration will fail on aged positions, potentially missing liquidation windows.
- **User confusion:** Users cannot close positions without first restoring them, which requires extra transaction fees and technical knowledge.

**Prevention:**
1. Increase position TTL to match the practical maximum position lifetime. For perps, 90+ days is reasonable: `LEDGER_THRESHOLD_POSITION = ONE_DAY_LEDGERS * 90`.
2. Add a `bump_position_ttl` public function that anyone can call to extend a position's TTL.
3. Ensure the keeper infrastructure handles `RestoreFootprintOp` automatically.
4. Document the TTL behavior in the protocol spec -- auditors need to know the archival implications.

**Detection:** Check `LEDGER_THRESHOLD_POSITION` and `LEDGER_BUMP_POSITION` values. If they are shorter than the expected maximum position lifetime, this pitfall is active. Currently: 14/21 days, which is too short for perpetual positions.

**Phase:** Pre-audit cleanup (constant change) + documentation phase (archival behavior documentation).

**Confidence:** HIGH for the issue existing (verified: 14-day threshold in storage.rs). MEDIUM for it being flagged as critical by auditors (Protocol 23 mitigates somewhat).

---

### Pitfall 11: Market Config Validation Relies on Factory Pre-Check

**What goes wrong:** `set_market_config()` in the trading contract does not validate that `margin > liq_fee` (the invariant that ensures positions cannot be liquidated before reaching max leverage). This validation exists in the factory's `deploy_v2` but if the owner calls `set_market_config()` directly (bypassing the factory), invalid configuration can be set.

**Why it happens:** The validation was implemented in the factory under the assumption that all market configuration goes through the factory. But the trading contract's `set_market_config` is an `#[only_owner]` function that can be called independently.

**Consequences:**
- **HIGH finding:** If `margin <= liq_fee`, positions can be liquidated the moment they are opened (liquidation threshold is met before max leverage is reached), or conversely, positions that should be liquidatable cannot be.
- **Config manipulation through governance:** The governance contract queues and executes config changes on the trading contract directly, not through the factory. This bypass path is the realistic attack vector.

**Prevention:**
1. Move `require_valid_market_config` validation into the trading contract's `set_market_config` implementation. The validation already exists in `validation.rs` -- it just needs to be called.
2. Add integration tests that attempt to set invalid market configs directly on the trading contract and verify they are rejected.

**Detection:** Check whether `require_valid_market_config` is called in the trading contract's `set_market_config` function (not just in the factory).

**Phase:** Pre-audit cleanup (add validation call to trading contract).

**Confidence:** HIGH -- verified in code and documented in CONCERNS.md.

---

### Pitfall 12: No Fuzz Testing for Fixed-Point Arithmetic

**What goes wrong:** Fee calculations, PnL computation, and rate curves use fixed-point math with multiple scalar domains (SCALAR_7, SCALAR_18, price_scalar). Without fuzz testing, edge cases in the math (precision loss, rounding direction disagreement, phantom overflows handled by the library) go undiscovered. The existing fuzz targets in `test-suites/fuzz/` are non-functional (out of sync with current API).

**Why it happens:** Fuzz testing requires additional setup (cargo-fuzz or proptest) and the targets broke when the API changed. Unit tests cover known cases but cannot explore the combinatorial space of leverage * hold_time * rate * price_change.

**Consequences:**
- **MEDIUM finding:** Auditors will note the absence of property-based testing for a math-heavy protocol. They may discover precision bugs that targeted tests missed.
- **Specific risk areas:**
  - `divide-before-multiply` patterns (Scout detector: `divide-before-multiply`) in rate calculations
  - Rounding direction inconsistency: `fixed_mul_ceil` used for fees (correct -- round against user) but `fixed_mul_floor` for PnL (correct -- conservative). Any inconsistency is exploitable.
  - The `soroban-fixed-point-math` library handles phantom overflows by scaling to i128/u128, but the Zenex code sometimes chains multiple fixed-point operations where intermediate results could lose precision.

**Prevention:**
1. Rebuild fuzz targets using the current API. Key properties to test:
   - Settlement equity is never negative (net_pnl clamped to -collateral)
   - Funding is zero-sum between long and short sides
   - Borrowing fee monotonically increases with time and utilization
   - ADL factor is always in [0, SCALAR_18]
2. Use `proptest` for property-based tests that run in `cargo test`.
3. Add boundary tests: position at exactly max leverage, exactly at liquidation threshold, exactly at utilization cap.

**Detection:** Run `cargo fuzz list` or check `test-suites/fuzz/` -- if targets don't compile, fuzz testing is absent.

**Phase:** Integration testing phase (rebuild alongside integration tests).

**Confidence:** HIGH -- existing fuzz targets verified as non-functional; math-heavy protocol without property testing is a known audit gap.

---

## Minor Pitfalls

Issues that result in LOW/INFORMATIONAL findings but indicate professional maturity.

---

### Pitfall 13: Dependency Pinning on Git References

**What goes wrong:** The workspace Cargo.toml pins `stellar-access`, `stellar-contract-utils`, and `stellar-macros` to git references without specifying a commit hash or tag:
```toml
stellar-access = { git = "https://github.com/OpenZeppelin/stellar-contracts" }
```
This means `cargo update` could pull in breaking changes or security regressions from the main branch at any time.

**Prevention:**
1. Pin to a specific commit: `git = "...", rev = "abc123"`.
2. Or pin to a tag: `git = "...", tag = "v0.1.0"`.
3. Document the exact commit/version used for the audit in a `DEPENDENCIES.md`.

**Detection:** Check Cargo.toml for git dependencies without `rev` or `tag`.

**Phase:** Pre-audit cleanup.

**Confidence:** HIGH -- verified in workspace Cargo.toml.

---

### Pitfall 14: Soroban SDK Version May Be Outdated

**What goes wrong:** The workspace uses `soroban-sdk = "25.3.0"`. Soroban SDK versions track Stellar protocol versions and receive security patches. If 25.3.0 has known issues or a newer version has security fixes, the audit will flag it.

**Prevention:**
1. Check the current latest version before audit submission.
2. If updating, re-run all tests to verify compatibility.
3. Document the SDK version choice in the audit submission package.
4. Scout has a `soroban-version` detector that flags outdated versions.

**Detection:** Compare workspace SDK version against latest on crates.io.

**Phase:** Pre-audit cleanup.

**Confidence:** MEDIUM -- the version may or may not be outdated at audit time.

---

### Pitfall 15: Instance Storage Bloat Risk

**What goes wrong:** The trading contract stores 8+ items in instance storage (Status, Vault, Token, PriceVerifier, Config, Treasury, PositionCounter, TotalNotional, LastFundingUpdate). Instance storage is loaded entirely on every contract invocation. If more items are added (e.g., additional config fields), the per-transaction cost increases linearly.

**Why Soroban-specific:** Unlike EVM storage where each slot is loaded independently, Soroban instance storage is all-or-nothing. CoinFabrik Scout's `dos-unbounded-operation` detector flags this pattern.

**Prevention:**
1. Audit what is in instance storage vs. what should be in persistent storage. Items that don't change often (Config, PriceVerifier address) could move to persistent storage with appropriate TTLs.
2. Keep instance storage to only items that genuinely need to be loaded every transaction (Status, TotalNotional, PositionCounter).
3. Document the current instance storage footprint and gas cost.

**Detection:** Count `instance().set()` calls in storage.rs. If more than ~5 items, evaluate whether all need instance-level access.

**Phase:** Documentation phase (document the design decision) or pre-audit cleanup (if refactoring).

**Confidence:** MEDIUM -- current size (8 items) is manageable but auditors will note the pattern.

---

### Pitfall 16: Error Code Documentation Missing

**What goes wrong:** The contract uses numeric error codes (e.g., `Error(Contract, #780)`) but there is no public mapping from error code to meaning. Auditors, keepers, and users cannot interpret error messages without reading source code. The `#[should_panic(expected = "Error(Contract, #421)")]` pattern in tests is fragile and opaque.

**Prevention:**
1. Add doc comments to every variant in `TradingError` enum explaining the error condition.
2. Create an error code reference table in the protocol documentation.
3. Consider using named error variants in test assertions where possible.

**Detection:** Check if `errors.rs` has doc comments on each variant.

**Phase:** Documentation phase.

**Confidence:** HIGH -- standard audit requirement.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| **Pre-audit cleanup** | Pitfalls 2, 4, 5, 6, 8, 11, 13, 14 -- Mechanical fixes that must happen before audit engagement | Run Scout first (P8) to generate full fix list. Fix unsafe unwraps (P2), add collateral check (P4), split closability (P5), add token decimal assertion (P6), add market config validation (P11), pin dependencies (P13) |
| **Integration test rebuild** | Pitfalls 1, 3, 12 -- Test suite is the audit's foundation | Rebuild test-suites first (P3), add auth-negative tests (P1), rebuild fuzz targets (P12). Prioritize: settlement paths, liquidation, ADL, funding accrual |
| **Documentation** | Pitfalls 7, 10, 15, 16 -- Auditors need context to work efficiently | Write invariant specs (P7), document TTL/archival behavior (P10), justify instance storage choices (P15), create error code reference (P16) |
| **Threat model completion** | Pitfall 7 supplement -- STRIDE exists but needs code cross-references | Add inline STRIDE references in code, create visual trust boundary diagram, complete admin privilege matrix |

## Audit Submission Checklist (Derived from Pitfalls)

This checklist synthesizes all pitfalls into a pre-submission verification:

- [ ] `cargo scout-audit` runs with zero CRITICAL findings
- [ ] `cargo test --workspace` passes with zero failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo audit` shows no known vulnerabilities
- [ ] Zero bare `.unwrap()` calls in production code (test code is fine)
- [ ] Authorization negative tests exist for every `require_auth` call
- [ ] Integration tests cover: open, fill, close, modify, cancel, liquidate, ADL, funding
- [ ] Fuzz targets compile and run for fee/rate/settlement calculations
- [ ] Token decimal assertion in constructor
- [ ] Post-fee collateral non-negativity check
- [ ] Liquidation exempt from MIN_OPEN_TIME
- [ ] Market config validation in trading contract (not just factory)
- [ ] Dependencies pinned to specific commits/tags
- [ ] Function-level documentation on all public functions
- [ ] Invariant specifications documented
- [ ] Error code reference table exists
- [ ] Position TTL sufficient for expected position lifetime

## Sources

**Soroban-specific security:**
- [Veridise Soroban Security Checklist](https://veridise.com/blog/audit-insights/building-on-stellar-soroban-grab-this-security-checklist-to-avoid-vulnerabilities/) -- Soroban-specific vulnerability categories
- [CoinFabrik Scout for Soroban](https://github.com/CoinFabrik/scout-soroban) -- 23 Soroban-specific vulnerability detectors
- [Stellar Soroban State Archival](https://developers.stellar.org/docs/learn/fundamentals/contract-development/storage/state-archival) -- TTL and archival mechanics
- [Stellar Authorization Framework](https://developers.stellar.org/docs/learn/fundamentals/contract-development/authorization) -- require_auth security model

**DeFi audit preparation:**
- [Hacken Smart Contract Audit Process](https://hacken.io/discover/smart-contract-audit-process/) -- Pre-audit preparation guide
- [OpenZeppelin Audit Readiness Guide](https://learn.openzeppelin.com/security-audits/readiness-guide) -- Documentation and testing requirements
- [Olympix: Why Audits Fail](https://olympix.security/blog/why-smart-contract-audits-fail) -- Structural audit limitations

**Perpetual futures protocol security:**
- [QuillAudits: Perpetual Derivatives Protocols](https://www.quillaudits.com/blog/web3-security/perpetual-derivatives-protocols) -- Oracle manipulation, business logic attacks, liquidation vulnerabilities
- [Stellar Soroban Audit Bank](https://stellar.org/blog/developers/soroban-security-audit-bank-raising-the-standard-for-smart-contract-security) -- Stellar's audit funding program

**Codebase analysis:**
- Direct code inspection of `zenex-contracts/` (trading, strategy-vault, factory, price-verifier, governance)
- `.planning/codebase/CONCERNS.md` -- Known issues identified during codebase analysis
- `.planning/codebase/TESTING.md` -- Current test state documentation
- `security-v2/` -- Existing STRIDE threat model

---

*Pitfalls audit: 2026-03-24*
