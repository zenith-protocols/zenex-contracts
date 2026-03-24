# Project Research Summary

**Project:** Zenex Contracts -- Audit Preparation
**Domain:** DeFi perpetual futures protocol security audit preparation (Soroban/Stellar)
**Researched:** 2026-03-24
**Confidence:** HIGH

## Executive Summary

Zenex is a perpetual futures trading protocol built on Stellar's Soroban smart contract platform. The project is at code freeze and preparing for a formal security audit, likely through the Stellar Audit Bank program. Expert consensus from OpenZeppelin, Hacken, CoinFabrik (a Stellar Audit Bank firm), and Stellar's own documentation converges on a clear preparation methodology: threat model first (using Stellar's mandatory STRIDE template), then testing infrastructure (integration tests, fuzz testing, static analysis), then documentation (protocol spec, invariant spec, architecture docs). The project already has working per-crate unit tests and a partially completed STRIDE threat model in `security-v2/`, but its integration test suite (`test-suites/`) is completely non-functional due to API drift, which is the single largest gap.

The recommended approach is a four-phase preparation: (1) pre-audit code cleanup to fix known bugs and eliminate mechanical audit findings, (2) integration test suite rebuild as the highest-effort and highest-value deliverable, (3) documentation and threat model completion in parallel with testing, and (4) static analysis, fuzz testing, and mutation testing as the quality verification pass. Research identified 6 critical bugs/pitfalls that must be fixed before audit submission -- collateral going negative after fee deduction, MIN_OPEN_TIME blocking emergency liquidation, token decimal assumption unenforced, missing authorization negative tests, unsafe unwrap calls in production paths, and market config validation bypassing the factory.

Key risks center on three areas. First, the broken integration test suite -- auditors may refuse to engage or will significantly mark down confidence without working cross-contract tests. Second, the six critical code-level issues that will each generate HIGH/CRITICAL audit findings if not fixed pre-submission. Third, incomplete documentation -- the STRIDE threat model is ~70% complete and missing its visual DFD and validation section, and no formal invariant specification exists. All three risks are addressable within the preparation timeline if work is sequenced correctly: fix bugs first, then rebuild tests, then document.

## Key Findings

### Recommended Stack

The tooling stack for audit preparation is mature and well-documented for Soroban. Every recommended tool is either Stellar-endorsed or the standard Rust ecosystem choice. No custom tooling is needed.

**Core technologies:**
- **CoinFabrik Scout (`cargo-scout-audit`)**: Soroban-specific static analyzer with 24 detectors -- the only purpose-built vulnerability scanner for Soroban, built by an Audit Bank firm
- **`cargo-llvm-cov`**: LLVM-based coverage reporting -- already configured in the project Makefile, Stellar-recommended
- **`cargo-mutants`**: Mutation testing to verify test quality beyond line coverage -- Stellar-recommended, zero-config for Soroban
- **`cargo-fuzz` + `proptest`**: Fuzzing and property-based testing -- existing infrastructure in `test-suites/fuzz/` needs rebuilding
- **Stellar STRIDE Template**: Mandatory threat model format for Audit Bank applications -- non-negotiable
- **mdBook**: Protocol specification and architecture documentation -- Rust-native, integrates with `cargo doc`
- **Certora Sunbeam** (optional, Phase 2): Formal verification of Soroban WASM -- used by Blend v2 and Aquarius but MEDIUM confidence due to tooling maturity

**Critical version requirements:** Soroban SDK 25.3.0 (verify no newer security patches exist before submission). Nightly Rust toolchain required for `cargo-fuzz`.

### Expected Features

**Must have (table stakes -- auditors expect all of these):**
- Protocol specification document with testable correctness statements
- Architecture overview with visual data flow diagrams
- Access control matrix (function-level permissions for all 6 contracts)
- Invariant specification (vault solvency, fee conservation, notional bookkeeping)
- Code freeze with tagged commit and locked `Cargo.lock`
- Clean codebase: zero warnings, zero TODOs, zero dead code
- Unit tests passing at 80%+ line coverage across in-scope contracts
- Working integration test suite covering all critical cross-contract paths
- Complete STRIDE threat model (all 6 categories, DFD, mitigations, validation)

**Should have (differentiators that reduce audit cost and findings):**
- Function-level `///` doc comments on all business logic
- Inline invariant annotations at critical code points
- Decision rationale docs (why no reentrancy guards, why keeper has no auth, etc.)
- Fuzz testing with invariant assertions on fee/rate/settlement math
- Property-based tests (proptest) for settlement conservation, funding zero-sum
- Negative/adversarial test cases for every public entry point
- Known issues / accepted risks document
- Attack scenario test cases derived from STRIDE threats
- Static analysis clean bill (Scout + Clippy + `cargo audit`)
- Deployment guide with parameter security bounds

**Defer to post-audit or v2:**
- Formal verification (Certora Sunbeam) -- valuable but secondary to core package
- Mutation testing results -- nice-to-have, not required
- Economic attack analysis documentation -- document basic vault drain/funding manipulation scenarios but deep analysis can follow
- Emergency procedure documentation
- Separate testnet deployment for auditors (they review code, not deployments)

### Architecture Approach

The STRIDE analysis identified 39 distinct threats across 7 trust boundaries, with 9 rated CRITICAL. The architecture maps to Stellar's four-question threat model framework. The most attack-prone boundaries are: oracle interface (stale/manipulated prices via Pyth Lazer), keeper interface (selective execution, front-running), and vault interface (unbounded `strategy_withdraw` with no withdrawal cap). The perpetual futures domain adds domain-specific threats not found in simpler DeFi protocols: funding rate manipulation, ADL index gaming, fixed-point arithmetic dust accumulation, and liquidation cascade timing.

**Major components and their threat surface:**
1. **Trading Contract** -- Position lifecycle, fee computation, settlement math. Highest threat density (touches all boundaries). 39 storage items in instance + persistent storage.
2. **Strategy Vault** -- LP deposits/withdrawals, `total_assets()` used for utilization calculations. Critical trust boundary: only `strategy` (trading contract) can call `strategy_withdraw`, but there is no withdrawal cap.
3. **Price Verifier** -- Pyth Lazer ed25519 signature verification, staleness, confidence. Single oracle dependency with no fallback.
4. **Governance** -- Timelock-gated config updates. `set_status` is immediate (emergency power). Queued updates use temporary storage with TTL expiration risk.
5. **Factory** -- Deploys vault+trading atomically. Contains validation logic (`require_valid_market_config`) that the trading contract itself does not enforce.
6. **Treasury** -- Fee routing with configurable rate. Rate has no explicit cap beyond SCALAR_7 (100%).

### Critical Pitfalls

The top 6 issues that will generate HIGH/CRITICAL audit findings if not addressed:

1. **Collateral can go negative after fee deduction** (Pitfall 4) -- `position.col -= base_fee + impact_fee` has no post-deduction non-negativity check. Fix: add check after fee deduction or validate against post-fee collateral. This is a vault drain vector.
2. **MIN_OPEN_TIME blocks emergency liquidation** (Pitfall 5) -- `require_closable()` enforces 30s hold time for ALL close types including liquidations. During flash crashes, positions become unliquidatable. Fix: exempt liquidation from MIN_OPEN_TIME.
3. **Zero authorization negative tests** (Pitfall 1) -- 100% of tests use `mock_all_auths()`. No test anywhere verifies that unauthorized callers are rejected. Fix: add negative auth tests for every `require_auth` call.
4. **Integration test suite non-functional** (Pitfall 3) -- `test-suites/` uses the old oracle pattern and cannot compile. Zero working cross-contract tests exist. Fix: rebuild from scratch.
5. **Unsafe unwrap calls in production paths** (Pitfall 2) -- `.unwrap()` in `contract.rs:25`, `adl.rs:31`, `adl.rs:86`, `price-verifier/lib.rs:42`, `price-verifier/pyth.rs:51`. Fix: replace with `panic_with_error!` or `.ok_or()`.
6. **Token decimal assumption unenforced** (Pitfall 6) -- Constructor does not verify collateral token has 7 decimals. Wrong decimals silently break all math. Fix: add `assert!(token.decimals() == 7)` in constructor.

Additional moderate pitfalls: market config validation only in factory not trading contract (Pitfall 11), index accumulation without overflow guards (Pitfall 9), position TTL too short for perpetual positions (Pitfall 10, 14/21 day TTL), dependency pinning on git refs without commit hashes (Pitfall 13).

## Implications for Roadmap

Based on combined research, the audit preparation should be structured in 4 phases with clear dependencies. The critical path is: code fixes -> integration tests -> documentation. Code fixes must come first because they are technically code freeze exceptions (bug fixes, not features) and the tests need to be written against correct code.

### Phase 1: Pre-Audit Code Cleanup
**Rationale:** Fix the 6 critical code-level issues and run static analysis to generate a complete mechanical fix list. These are bug fixes, not feature changes, so they are valid code freeze exceptions. Fixing them first means integration tests are written against correct behavior.
**Delivers:** Clean codebase with zero CRITICAL Scout findings, all known bugs fixed, dependencies pinned, warnings eliminated.
**Addresses (from FEATURES.md):** Clean codebase, code freeze with tagged commit, consistent code style (rustfmt + clippy clean), no dead code/TODOs.
**Avoids (from PITFALLS.md):** Pitfalls 2, 4, 5, 6, 8, 11, 13, 14.
**Stack (from STACK.md):** Scout Soroban, Clippy, `cargo audit`, `cargo deny`.

**Tasks in order:**
1. Run `cargo scout-audit` to get full finding list
2. Fix collateral negativity (Pitfall 4)
3. Split `require_closable` for liquidation exemption (Pitfall 5)
4. Add token decimal assertion in constructor (Pitfall 6)
5. Add market config validation in trading contract (Pitfall 11)
6. Replace all production `.unwrap()` with `panic_with_error!` (Pitfall 2)
7. Pin git dependencies to specific commits (Pitfall 13)
8. Run `cargo clippy -- -D warnings` and fix all warnings
9. Remove dead code, TODOs, debug artifacts
10. Tag commit as audit baseline

### Phase 2: Integration Test Suite Rebuild
**Rationale:** This is the single highest-effort and highest-value deliverable. Auditors use integration tests as their primary confidence signal. The threat-to-test mapping from ARCHITECTURE.md drives what to test. Must be built against the cleaned-up code from Phase 1.
**Delivers:** Working `test-suites/` with cross-contract tests covering all critical paths, authorization negative tests, adversarial edge cases.
**Addresses (from FEATURES.md):** Integration test suite, unit tests at 80%+ coverage, negative/adversarial test cases, test coverage report.
**Avoids (from PITFALLS.md):** Pitfalls 1 (auth tests), 3 (broken test-suites), 12 (no fuzz testing).
**Stack (from STACK.md):** `cargo-llvm-cov`, `cargo-fuzz`, `proptest`.

**Tasks in order:**
1. Fix/update `TestFixture` to work with current contract APIs
2. Write integration tests for full position lifecycle (open -> fill -> close with profit/loss)
3. Write liquidation flow tests (including immediate liquidation within MIN_OPEN_TIME)
4. Write ADL triggering and recovery tests
5. Write funding accrual and conservation tests (long paid = short received)
6. Write multi-market interaction tests
7. Add authorization negative tests for ALL `require_auth` calls (no `mock_all_auths`)
8. Add boundary tests for every numeric limit (max leverage, max util, margins)
9. Rebuild fuzz targets for fee/rate/settlement calculations
10. Add proptest properties for settlement conservation and funding zero-sum
11. Generate coverage report, target 80%+ line coverage

### Phase 3: Documentation and Threat Model Completion
**Rationale:** Can partially overlap with Phase 2. The STRIDE threat model is ~70% done and drives both auditor understanding and test prioritization. Documentation includes protocol spec, invariant spec, and function-level doc comments. These are writing tasks that do not depend on code changes.
**Delivers:** Complete STRIDE threat model in Stellar template format, protocol specification, invariant specification, access control matrix, function-level documentation, known issues document.
**Addresses (from FEATURES.md):** Protocol specification, architecture overview with DFD, access control matrix, invariant specification, STRIDE threat model, function-level doc comments, decision rationale docs, known issues document, deployment guide.
**Avoids (from PITFALLS.md):** Pitfall 7 (incomplete threat model), Pitfall 10 (undocumented TTL behavior), Pitfall 15 (undocumented storage choices), Pitfall 16 (no error code docs).
**Stack (from STACK.md):** mdBook, `cargo doc`, Mermaid/draw.io for DFDs.

**Tasks in order:**
1. Complete STRIDE threat model: add visual DFDs, finish mitigation table, complete validation section
2. Archive `security/` and make `security-v2/` the single source of truth
3. Write protocol specification (position lifecycle, fee system, ADL, vault mechanics)
4. Write formal invariant specifications (vault solvency, fee conservation, notional consistency, index monotonicity)
5. Create access control matrix (every entry point x actor type x auth mechanism)
6. Add `///` doc comments to all public functions in trading, vault, price-verifier
7. Add inline invariant annotations at critical code points
8. Write known issues / accepted risks document
9. Create error code reference table
10. Write deployment guide with parameter security bounds

### Phase 4: Quality Verification and Audit Package Assembly
**Rationale:** Final pass to verify test quality, run extended analysis, and assemble the complete audit submission package. Depends on both tests (Phase 2) and documentation (Phase 3) being complete.
**Delivers:** Mutation testing results, static analysis clean bill, complete audit package with all artifacts.
**Addresses (from FEATURES.md):** Mutation testing results, static analysis clean bill, dependency audit report, test coverage report, attack scenario tests derived from STRIDE.
**Stack (from STACK.md):** `cargo-mutants`, Scout (final run), `cargo audit`, `cargo deny`, `cargo-llvm-cov`.

**Tasks in order:**
1. Run `cargo-mutants` on trading, vault, factory, price-verifier -- fix MISSED mutations
2. Run final Scout + Clippy pass (zero findings required)
3. Run `cargo audit` + `cargo deny` -- fix any advisories
4. Generate final coverage report (HTML + lcov)
5. Create threat-to-test mapping document (THREAT-TEST-MAP.md)
6. Build mdBook documentation site
7. Generate `cargo doc` API reference
8. Assemble audit submission package (README, SCOPE.md, all artifacts)
9. Tag final audit commit

### Phase Ordering Rationale

- **Phase 1 before Phase 2:** Tests must be written against correct code. Fixing the collateral negativity bug, liquidation timing bug, and other issues first means integration tests validate the correct behavior, not the buggy behavior.
- **Phase 2 is the bottleneck:** Rebuilding `test-suites/` from scratch is the highest-effort item. Starting it as early as possible (immediately after Phase 1) is critical for timeline.
- **Phase 3 overlaps Phase 2:** Documentation is a writing task that can proceed in parallel with test development. The STRIDE model does not depend on tests, and doc comments can be added while integration tests are being built.
- **Phase 4 depends on Phase 2 + 3:** Mutation testing requires a complete test suite. Audit package assembly requires both tests and docs to be done.
- **This ordering avoids the top pitfall pattern:** Teams that write docs first and tests last run out of time for tests. Tests are prioritized because auditors weight them more heavily than documentation.

### Research Flags

Phases likely needing deeper research during planning:
- **Phase 2 (Integration Tests):** Soroban resource limits for batch execute (DoS.7) and apply_funding with many markets (DoS.8) need empirical measurement. The exact `SorobanArbitrary::Prototype` setup for fuzz targets may need experimentation.
- **Phase 3 (Documentation):** Auditor expectations for protocol specification depth vary by firm. If the specific audit firm is known, review their sample audit reports to calibrate spec detail level.

Phases with standard patterns (skip research-phase):
- **Phase 1 (Code Cleanup):** Entirely mechanical. Run Scout, fix findings, pin deps. Well-documented patterns.
- **Phase 4 (Quality Verification):** Standard tool execution. `cargo-mutants` and final analysis passes are straightforward.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All tools are Stellar-endorsed or standard Rust ecosystem. Scout is built by an Audit Bank firm. No exotic or unproven tools. |
| Features | HIGH | Cross-referenced OpenZeppelin, Hacken, SCSFG, and Stellar audit preparation guides. Strong consensus on required artifacts. |
| Architecture | HIGH | 39 STRIDE threats derived from direct codebase analysis, not theoretical. Trust boundaries verified against actual contract code. |
| Pitfalls | HIGH | 6 critical pitfalls verified by code inspection and cross-referenced with CONCERNS.md. Historical DeFi exploits provide evidence for severity ratings. |

**Overall confidence:** HIGH

### Gaps to Address

- **Soroban transaction resource limits:** DoS threats (DoS.7, DoS.8) need empirical testing to determine actual CPU/memory budget limits for batch execute and apply_funding. Plan to measure during Phase 2.
- **Optimal max_staleness value:** Requires analysis of Stellar ledger close times (~5-7s) vs. Pyth Lazer update frequency. Affects Spoof.2 severity assessment.
- **Multi-sig for admin operations:** Not currently implemented. Design decision needed: should trading contract owner be a multi-sig, the governance contract, or both? Affects EoP.1, EoP.2 severity.
- **Vault lock_time and ADL interaction:** Can locked LP shares be affected by ADL-triggered vault balance changes? Needs verification during Phase 2 testing.
- **Post-audit monitoring and incident response:** Out of scope for audit preparation but should be planned for post-deployment. Emergency procedure documentation is a differentiator, not table stakes.
- **Audit firm selection:** The specific firm's expectations may refine Phase 3 priorities. If applying through Stellar Audit Bank, CoinFabrik, Halborn, OtterSec, Trail of Bits, Quantstamp, or OpenZeppelin are the participating firms -- each has different depth expectations.

## Sources

### Primary (HIGH confidence)
- [Stellar STRIDE Threat Model Template](https://developers.stellar.org/docs/build/security-docs/threat-modeling/STRIDE-template)
- [Stellar Threat Modeling How-To Guide](https://developers.stellar.org/docs/build/security-docs/threat-modeling/threat-modeling-how-to)
- [Stellar Code Coverage Guide](https://developers.stellar.org/docs/build/guides/testing/code-coverage)
- [Stellar Fuzzing Guide](https://developers.stellar.org/docs/build/smart-contracts/example-contracts/fuzzing)
- [Stellar Mutation Testing Guide](https://developers.stellar.org/docs/build/guides/testing/mutation-testing)
- [Stellar Definitive Guide to Testing](https://stellar.org/blog/developers/the-definitive-guide-to-testing-smart-contracts-on-stellar)
- [CoinFabrik Scout Soroban (GitHub)](https://github.com/CoinFabrik/scout-soroban)
- [Veridise Soroban Security Checklist](https://veridise.com/blog/audit-insights/building-on-stellar-soroban-grab-this-security-checklist-to-avoid-vulnerabilities/)
- [OpenZeppelin Audit Readiness Guide](https://learn.openzeppelin.com/security-audits/readiness-guide)
- [Hacken: How to Prepare for a Smart Contract Audit](https://hacken.io/discover/smart-contract-audit-process/)
- [SCSFG: Audit Preparation](https://scsfg.io/developers/audit-preparation/)
- [Soroban Audit Bank Program](https://stellar.org/grants-and-funding/soroban-audit-bank)
- Direct codebase analysis of all 6 in-scope contracts

### Secondary (MEDIUM confidence)
- [Certora Sunbeam Documentation](https://docs.certora.com/en/latest/docs/sunbeam/index.html)
- [QuillAudits: Perpetual Derivatives Protocols](https://www.quillaudits.com/blog/web3-security/perpetual-derivatives-protocols)
- [Composable Security: Threat Modeling for Smart Contracts](https://composable-security.com/blog/threat-modeling-for-smart-contracts-best-step-by-step-guide/)
- [Sherlock: Smart Contract Audit Process](https://sherlock.xyz/post/smart-contract-audit-the-complete-process-from-scoping-to-secure-deployment)
- [Hacken: Perpetual DEX Security Evolution](https://hacken.io/discover/perpetual-dex-security-evolution/)

---
*Research completed: 2026-03-24*
*Ready for roadmap: yes*
