# Feature Landscape: Smart Contract Audit Preparation Package

**Domain:** DeFi perpetual futures protocol audit preparation (Soroban/Stellar)
**Researched:** 2026-03-24

## Table Stakes

Features auditors expect. Missing any of these means the audit takes longer, costs more, or the auditor refuses to proceed.

### Documentation Deliverables

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Protocol specification document | Auditors need a single source of truth describing intended behavior, not guessing intent from code. Hacken calls these "functional requirements" and says they are the baseline for evaluating correctness. | Medium | Describe all position lifecycle flows (open, close, modify, liquidate, ADL), fee calculations (funding, borrowing, base, impact), vault mechanics, and oracle integration. Must be testable statements, not marketing prose. |
| Architecture overview with data flow diagrams | Auditors need to understand component boundaries, cross-contract calls, and data flow before reading code. OpenZeppelin requires "external documentation covering system architecture." | Medium | Already partially in `ARCHITECTURE.md`. Needs visual DFD showing trading <-> vault <-> treasury <-> price-verifier <-> governance call graph with data types on each edge. STRIDE report notes "TODO: Visual diagram to be added." |
| Access control matrix | Auditors check every privileged function against the documented permission model. Misconfigured access controls caused >$200M in losses in 2024-2025. Hacken specifically requires "a permissions matrix identifying operator roles, privileged accounts." | Low | Document every entry point, who can call it (user/keeper/owner/anyone), and what auth check enforces it. Zenex has 4 actor types (trader, keeper, owner, anyone) across 6 contracts. Existing actor inventory in STRIDE report is a good start but needs function-level granularity. |
| Invariant specification | Auditors validate correctness against invariants, not just "does it compile." Formal verification literature describes invariants as "logical assertions about execution that must remain true under every possible circumstance." | High | Critical for a perp protocol: vault solvency (total_assets >= sum of all position payouts), index monotonicity, notional bookkeeping consistency, fee conservation. These must be written as precise mathematical statements auditors can verify against code. |
| Code freeze with tagged commit | Any change during audit invalidates findings. Every audit firm requires a stable, reproducible baseline. SCSFG: "Any changes made to the code can invalidate the audit findings." | Low | Tag a commit hash, lock `Cargo.lock`, document exact Soroban SDK version. Already planned per PROJECT.md constraints. |
| Clean, compiling codebase with one-command setup | Auditors bill by the hour. If they spend a day getting the project to compile, that is wasted audit budget. Hacken requires "a minimal, preferably one-command setup." | Low | `cargo build --target wasm32v1-none --release` must succeed cleanly. Remove TODOs, dead code, commented-out logic. Resolve all compiler warnings. Run `cargo clippy`. |
| Unit tests passing with reasonable coverage | Test suites are "a good proxy for the overall quality of a project" (OpenZeppelin). Auditors frequently discover vulnerabilities in untested code paths. 75-90% coverage is the industry norm. | Medium | Per-crate unit tests in trading, vault, price-verifier, factory, governance, treasury. Current state: each contract has `test.rs` but trading's are in-crate tests. Coverage target: 80%+ line coverage across all in-scope contracts. |
| Integration test suite covering critical paths | Cross-contract interaction bugs are where the money is lost. Integration tests prove the system works as a whole. | High | `test-suites` crate exists but is "mostly outdated and out of sync with current trading API" (PROJECT.md). Must be rebuilt covering: full position lifecycle, liquidation flow, ADL trigger, funding accrual, vault deposit/redeem, governance timelock execution. This is the highest-effort table stakes item. |
| STRIDE threat model (complete) | Stellar's Audit Bank program specifically requires STRIDE-format threat models. This is non-negotiable for the Stellar ecosystem. The framework requires system overview, DFD, threats per STRIDE category, mitigations, and validation. | Medium | `security-v2/` has a STRIDE report in progress with sections 1-6+ partially complete. Needs completion: visual DFD, remaining threat categories, full mitigation documentation, and validation section. |

### Code Quality

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Consistent code style (rustfmt + clippy clean) | Inconsistent formatting wastes auditor time on style vs. substance. Industry standard: "Follow official language and framework style guides. Run formatters and linters." | Low | Run `cargo fmt --check` and `cargo clippy -- -D warnings` as CI gates. |
| No dead code, TODOs, or debug artifacts | Dead code confuses scope. TODOs signal incomplete work. Debug code may mask real behavior. | Low | Audit the codebase for `todo!()`, `unimplemented!()`, `dbg!()`, commented-out blocks. |
| Meaningful error types with distinct codes | Auditors trace error paths to verify security invariants. Generic errors obscure what went wrong and why. | Low | Zenex already has `TradingError` enum with distinct codes (702+, 730+, 750+, 780). Verify all error paths use specific variants, not panic!() or generic messages. |

---

## Differentiators

Features that are not strictly required but significantly speed up the audit, reduce findings, and lower cost.

### Documentation Extras

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Function-level doc comments (Rust `///`) | Auditors read every function. Doc comments explaining intent, invariants maintained, and edge cases handled reduce time spent reverse-engineering logic. Currently ~100 doc comments across 15 files in trading/src -- reasonable start but sparse on business logic. | Medium | Priority: all public entry points, all fee/PnL calculation functions, all state mutation functions. Use `/// # Safety`, `/// # Panics`, `/// # Invariants` sections where applicable. |
| Inline invariant annotations | Comments at critical code points stating "INVARIANT: X holds here because Y" let auditors confirm correctness in-situ rather than cross-referencing separate docs. | Medium | Add at: fee deduction points (collateral >= fees), index update points (index only increases), notional bookkeeping (sum matches), vault balance checks. |
| Decision rationale documentation | Explains WHY a design choice was made, not just WHAT it does. Prevents auditors from flagging intentional behavior as bugs. | Low | Document: why no reentrancy guards (Soroban's execution model prevents it), why keeper execute() has no require_auth on caller, why MIN_OPEN_TIME exists, why ADL uses proportional reduction vs. full liquidation. |
| Deployment guide with parameter explanations | Auditors verify that deployment parameters do not create unsafe states. Explaining valid ranges and their security implications prevents false positives. | Low | Already have `deploy.json`. Add a companion doc explaining each parameter's security bounds: what happens if `max_util` is set to 100%? If `r_funding` is 0? If `liq_fee` exceeds margin? |

### Testing Extras

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Fuzz testing with invariant assertions | Finds bugs that unit tests miss by exploring unexpected input combinations. The Soroban SDK has first-class `cargo-fuzz` support with `SorobanArbitrary` for custom types. "Fuzz testing is necessary for any blockchain project" (industry consensus). | High | `test-suites/fuzz/` already has `fuzz_liquidation.rs` and `fuzz_trading_general.rs` targets. Verify they use `panic_with_error!` (not `panic!`), assert key invariants (vault solvency, notional consistency), and have been run for meaningful duration. Add fuzz targets for: fee calculation edge cases, ADL trigger/recovery, multi-market interactions. |
| Property-based tests (proptest) | Reproducible variant of fuzz testing that runs in standard `cargo test`. Confirms invariants hold across randomized parameter space. | Medium | `test_trading_proptest.rs` exists (201 lines) with regression file. Expand to cover: settlement math (PnL + fees = collateral delta), funding conservation (long funding paid = short funding received), borrowing rate monotonicity. |
| Negative/adversarial test cases | Prove that invalid states are unreachable. Auditors look for what CANNOT happen and want test proof. | Medium | Test: opening position with 0 collateral, setting leverage above max, closing position before MIN_OPEN_TIME, calling admin functions without auth, oracle price at 0 or negative, utilization at 100%. |
| Test coverage report generation | Quantitative proof of coverage. Auditors want numbers, not promises. | Low | Set up `cargo-llvm-cov` and generate HTML report. Include in audit package. Target: 80%+ line coverage. |
| Mutation testing results | Proves tests actually catch bugs, not just execute code paths. Shows test quality beyond coverage numbers. | Low | Run `cargo-mutants` and document results. Fix any MISSED mutations in critical code. |

### Security Artifacts

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Known issues / accepted risks document | Prevents auditors from reporting things you already know about. Saves them time and reduces noise in the final report. Every finding they spend time writing up that you already know about is wasted budget. | Low | Document: Soroban TTL expiry risk for inactive positions, single oracle dependency (Pyth), keeper centralization risk, governance owner key compromise impact. |
| Attack scenario test cases (threat-model-derived) | Tests derived from STRIDE threats prove mitigations work. Industry best practice: "Convert [threat] scenarios to unit tests as prevention/verification mechanism." | High | For each STRIDE threat in `security-v2/`, write a test that attempts the attack and proves it fails. E.g., Spoof.1: test that calling close_position without auth panics. Tampering threats: test that manipulated prices are rejected. |
| Emergency procedure documentation | Auditors check that the protocol can respond to exploits. What happens if a bug is found post-deployment? | Low | Document: how to freeze trading (set_status), how to pause vault withdrawals, governance timelock bypass for emergencies, key rotation procedure for oracle signer. |
| Economic attack analysis | For perps specifically, auditors check economic attacks beyond code bugs: funding rate manipulation, vault drain scenarios, liquidation cascades. SlowMist's perp audit guide specifically checks oracle pricing, order logic, margin/leverage, LP solvency. | Medium | Document scenarios: What if a trader opens max leverage on a low-liquidity market? What if all positions are profitable simultaneously (vault drain)? What is the maximum loss the vault can absorb? How does ADL prevent insolvency? |
| Static analysis clean bill | Running Scout Soroban + Clippy with zero warnings demonstrates code hygiene. | Low | Run, fix findings, include report in audit package. |
| Dependency audit report | Clean `cargo audit` + `cargo deny` showing no known vulnerabilities in dependencies. | Low | Run once, document results. Fix any advisories. |

### Soroban-Specific Extras

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| TTL management audit | Soroban's state rent model is unique. Positions or market data expiring mid-lifecycle is a Soroban-specific risk that EVM auditors will not check by default. Veridise identifies unbounded storage and TTL misuse as Soroban-specific vulnerabilities. | Low | Document and test TTL thresholds for all storage keys. Verify: positions cannot expire while open, market configs bump on access, governance queued items survive their delay period. Already have TTL constants defined. |
| Storage layout documentation | Soroban storage (instance vs. persistent vs. temporary) has different cost and lifecycle characteristics. Auditors need to verify correct storage type usage. | Low | Document which data uses instance storage (hot data: config, status) vs. persistent (positions, market data) vs. temporary (governance queues). Explain the rationale for each choice. |
| Cross-contract call trust boundaries | Soroban cross-contract calls have different trust semantics than EVM. Document what each contract trusts about its callees and callers. | Low | Already in STRIDE TB1-TB8. Formalize: trading trusts vault balance accuracy, trading trusts price-verifier signature verification, vault trusts only strategy address for withdrawals. |
| Vec/Map input validation documentation | Veridise identifies Vec<T> and Map<K,V> type conversion as a Soroban-specific vulnerability. Document where these types cross contract boundaries and how inputs are validated. | Low | Audit all public entry points accepting Vec or Map parameters (e.g., `verify_prices` takes a batch, `execute` takes Vec<ExecuteRequest>). Document validation for each. |

---

## Anti-Features

Things to explicitly NOT include in the audit package. These waste auditor time or actively harm the audit.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| Out-of-scope contracts in the repo | Including backend code, SDK, or off-chain services in the audit repo scope confuses boundaries. Auditors bill for everything they review. | Clearly mark scope in a top-level SCOPE.md or README. Only trading, strategy-vault, factory, price-verifier, governance, and treasury are in scope. |
| Outdated/failing tests | Tests that fail or test old APIs are worse than no tests. They signal the codebase is unstable and force auditors to investigate whether failures are real bugs or stale tests. PROJECT.md already flags test-suites as "mostly outdated." | Either fix all tests or remove them entirely. Never ship an audit package with known-failing tests. |
| Incomplete threat model | A half-done STRIDE report (e.g., with "TODO" sections) suggests the team has not finished security analysis. Auditors will spend time on threats you should have caught internally. | Complete all 6 STRIDE categories, add visual DFD, fill all mitigation entries, complete validation section. The `security-v2/` STRIDE report has a TODO for the DFD and potentially incomplete later sections. |
| Over-documentation of obvious patterns | Documenting that `require_auth()` checks auth is noise. Auditors know Soroban's auth model. | Focus docs on non-obvious decisions: why the keeper has no auth check, why MIN_OPEN_TIME is 30s, why funding is purely P2P with no protocol cut. |
| Separate testnet deployment for auditors | Auditors review code, not deployments. A testnet deployment adds moving parts and maintenance burden without security value. | Provide reproducible local test environment via `cargo test`. If they want to interact with contracts, provide test fixtures that set up a full local environment. |
| Auto-generated documentation without curation | Running `cargo doc` and dumping it produces noise. Type definitions without context do not help auditors understand business logic. | Curate documentation: hand-written protocol spec + architecture doc + curated API reference with business context. Supplement with (but do not replace with) `cargo doc` output. |
| Multiple versions of the same document | Having both `security/` and `security-v2/` with overlapping content creates confusion about which is canonical. | Archive `security/` (or delete it). Make `security-v2/` the single source of truth. Rename to just `security/` or `threat-model/`. |
| New protocol features | Code freeze is in effect. Any functional change invalidates existing analysis and creates a moving target for auditors. | Document feature ideas as future work. Test and document existing behavior only. |
| Custom testing framework | Soroban SDK testutils + cargo test is the standard. Custom frameworks confuse auditors and add maintenance burden. | Use standard Soroban testing patterns. Extend TestFixture for new test scenarios. |

---

## Feature Dependencies

```
Code Freeze (tagged commit)
  |
  +-- Clean Codebase (no TODOs, warnings, dead code)
  |     |
  |     +-- Function-level doc comments
  |     |
  |     +-- Inline invariant annotations
  |     |
  |     +-- Decision rationale docs
  |
  +-- Protocol Specification
  |     |
  |     +-- Invariant Specification (formal statements of spec properties)
  |     |     |
  |     |     +-- Property-based tests (test the invariants)
  |     |     |
  |     |     +-- Fuzz tests with invariant assertions
  |     |
  |     +-- Access Control Matrix (derived from spec roles)
  |     |
  |     +-- Economic Attack Analysis (derived from spec mechanics)
  |
  +-- Architecture Overview + DFD
  |     |
  |     +-- Storage Layout Documentation
  |     |
  |     +-- Cross-contract Trust Boundaries
  |
  +-- STRIDE Threat Model (complete, not partial)
  |     |
  |     +-- Attack Scenario Test Cases (derived from threats)
  |     |
  |     +-- Known Issues / Accepted Risks (threats accepted as risk)
  |
  +-- Unit Tests (per-crate, passing, 80%+ coverage)
  |     |
  |     +-- Integration Tests (cross-contract, rebuilt from scratch)
  |     |     |
  |     |     +-- Negative/Adversarial Tests
  |     |
  |     +-- Test Coverage Report
  |     |
  |     +-- Mutation Testing Results
  |
  +-- Deployment Guide + Parameter Documentation
        |
        +-- Emergency Procedure Documentation
```

**Critical path:** Protocol Spec -> Invariant Spec -> Integration Tests (rebuilding test-suites is the bottleneck).

---

## MVP Recommendation (Minimum Viable Audit Package)

Prioritize in this order:

1. **Clean codebase + code freeze** -- Gate everything else. Remove dead code, fix warnings, tag commit. (Low effort, table stakes)
2. **Rebuild integration test suite** -- The single highest-value deliverable. Covers all critical paths and proves the system works. Current test-suites are outdated and must be rebuilt. (High effort, table stakes)
3. **Complete STRIDE threat model** -- Already ~70% done in `security-v2/`. Finish DFD, validate mitigations, complete all categories. Required for Stellar Audit Bank. (Medium effort, table stakes)
4. **Protocol specification** -- Write the "what should happen" document that auditors verify code against. (Medium effort, table stakes)
5. **Invariant specification** -- Formalize the mathematical properties (vault solvency, fee conservation, notional consistency). (High effort, table stakes)
6. **Access control matrix** -- Function-level permission mapping. Quick to produce from existing code + STRIDE actor inventory. (Low effort, table stakes)
7. **Function-level doc comments** -- Add `///` docs to all business logic functions in trading contract. (Medium effort, differentiator with high impact)
8. **Known issues document** -- List what you already know is risky. Prevents auditors from billing for your known risks. (Low effort, differentiator)
9. **Fuzz test expansion** -- Existing targets exist. Verify they work, expand invariant assertions, run for meaningful duration. (High effort, differentiator)
10. **Attack scenario tests** -- Derive from completed STRIDE model. Prove each mitigation works. (High effort, differentiator)

**Defer:** Formal verification (Certora Sunbeam), mutation testing, economic attack analysis docs, emergency procedure docs. These are valuable but secondary to the core package.

---

## Perpetual Futures-Specific Audit Focus Areas

Based on SlowMist's perp audit guide and QuillAudits' derivatives security research, auditors of this specific protocol type will focus on these areas. The audit package should address each proactively.

| Area | What Auditors Check | Zenex-Specific Concern |
|------|---------------------|----------------------|
| Oracle integrity | Price manipulation resistance, staleness, confidence bounds, fallback mechanisms | Single Pyth Lazer oracle, no fallback. ed25519 sig check + staleness + confidence. Document why no fallback is acceptable. |
| Liquidation correctness | Formula accuracy, fee handling, edge cases (zero collateral, dust positions) | Liquidation in `execute.rs`. Verify margin calculation matches spec. Test edge: position barely above/below liquidation threshold. |
| Funding rate mechanics | Calculation accuracy, manipulation resistance, accrual timing | P2P funding with no protocol cut. Hourly accrual. Test: funding conservation (long paid = short received). |
| Borrowing rate curve | Utilization calculation, rate curve correctness, index accrual | `r_base * (1 + r_var * util^5)` curve. Test: at 0%, 50%, 90%, 95%, 100% utilization. |
| ADL mechanism | Trigger conditions, proportional reduction accuracy, recovery conditions | 95% vault threshold triggers OnIce. Test: ADL reduces positions proportionally, recovery at 90%. |
| Vault solvency | Can the protocol pay all positions? Insurance/reserve adequacy | No insurance fund -- vault is the counterparty. ADL is the safety mechanism. Document explicitly. |
| Position settlement math | PnL + fees = correct token transfer | Settlement struct computes equity, total_fee, protocol_fee. Test: collateral + PnL - fees = actual transfer amount. |
| Fixed-point arithmetic | Overflow, underflow, rounding direction, precision loss | Three scalar systems (SCALAR_7, SCALAR_18, price_scalar). Test boundary values. Verify rounding always favors the protocol (not the trader). |
| Keeper incentive alignment | Can keepers profit by selective execution or timing? | Keeper caller_rate as fee incentive. No require_auth on execute(). Document why this is safe. |
| Governance timelock | Can admin bypass timelock? Can queued changes be frontrun? | Governance contract with mandatory delay. set_status is immediate (emergency). Document attack window. |

---

## Sources

- [OpenZeppelin Audit Readiness Guide](https://learn.openzeppelin.com/security-audits/readiness-guide) -- comprehensive preparation checklist (HIGH confidence)
- [Hacken: How to Prepare for a Smart Contract Audit](https://hacken.io/discover/smart-contract-audit-process/) -- documentation and testing requirements (HIGH confidence)
- [SCSFG: Audit Preparation](https://scsfg.io/developers/audit-preparation/) -- code quality and freeze protocol (HIGH confidence)
- [Stellar: Threat Modeling How-To Guide](https://developers.stellar.org/docs/build/security-docs/threat-modeling/threat-modeling-how-to) -- STRIDE template and requirements (HIGH confidence)
- [Stellar: Threat Modeling Readiness Guide](https://developers.stellar.org/docs/build/security-docs/threat-modeling) -- Audit Bank preparation requirements (HIGH confidence)
- [Veridise: Soroban Security Checklist](https://veridise.com/blog/audit-insights/building-on-stellar-soroban-grab-this-security-checklist-to-avoid-vulnerabilities/) -- Soroban-specific vulnerabilities (HIGH confidence)
- [Stellar: The Definitive Guide to Testing Smart Contracts on Stellar](https://stellar.org/blog/developers/the-definitive-guide-to-testing-smart-contracts-on-stellar) -- testing methodology (HIGH confidence)
- [Stellar: Soroban Fuzzing Example](https://developers.stellar.org/docs/build/smart-contracts/example-contracts/fuzzing) -- cargo-fuzz + proptest setup (HIGH confidence)
- [Stellar: Soroban Security Audit Bank](https://stellar.org/grants-and-funding/soroban-audit-bank) -- program requirements and co-payment structure (HIGH confidence)
- [QuillAudits: Complete Guide to Perpetual Derivatives Protocols](https://www.quillaudits.com/blog/web3-security/perpetual-derivatives-protocols) -- perp-specific attack vectors (MEDIUM confidence)
- [Composable Security: Threat Modeling for Smart Contracts](https://composable-security.com/blog/threat-modeling-for-smart-contracts-best-step-by-step-guide/) -- 14-step threat modeling process (MEDIUM confidence)
- [Sherlock: Smart Contract Audit Process](https://sherlock.xyz/post/smart-contract-audit-the-complete-process-from-scoping-to-secure-deployment) -- audit workflow and deliverables (MEDIUM confidence)
- [Sherlock: Smart Contract Audit Pricing 2026](https://sherlock.xyz/post/smart-contract-audit-pricing-a-market-reference-for-2026) -- cost context (MEDIUM confidence)

---

*Feature research: 2026-03-24*
