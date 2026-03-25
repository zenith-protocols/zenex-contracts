# Requirements: Zenex Contracts Audit Preparation

**Defined:** 2026-03-24
**Core Value:** Every attack surface is identified, tested, and documented — auditors can verify the protocol's safety without guessing intent.

## v1 Requirements

Requirements for audit-ready submission. Each maps to roadmap phases.

### Threat Model

- [x] **THREAT-01**: STRIDE threat model following Stellar's four-section framework (scope+DFDs, threats, mitigations, retrospective)
- [x] **THREAT-02**: Data flow diagrams covering all contract interactions (trading, vault, factory, price-verifier)
- [x] **THREAT-03**: Trust boundary documentation for all 7 boundaries (user-trading, keeper-trading, trading-price-verifier, trading-vault, trading-treasury, governance-trading, LP-vault)
- [x] **THREAT-04**: At least one threat per STRIDE category with severity rating and mitigation status

### Code Quality

- [x] **QUAL-01**: ~~Fix collateral negativity bug~~ — NOT A BUG: `position.validate()` on the next line checks `col <= 0` and reverts. Margin check also catches this. Already mitigated.
- [x] **QUAL-02**: Replace all unsafe `.unwrap()` calls in production code with proper error handling (6 identified)
- [x] **QUAL-03**: ~~Enforce token decimal assumption~~ — NOT NEEDED: all math is decimal-agnostic (rates are SCALAR_7 ratios, notional/col are same denomination). Deployer sets config values appropriate for the token's decimals.
- [x] **QUAL-04**: Scout Soroban static analysis passes with no critical/high findings
- [x] **QUAL-05**: Clippy passes with no warnings on all in-scope contracts
- [x] **QUAL-06**: cargo-audit and cargo-deny report no known vulnerabilities in dependencies
- [x] **QUAL-07**: All dependency versions pinned in Cargo.toml files
- [x] **QUAL-08**: Replace governance contract with generic timelock contract (queue/execute/cancel pattern using env.call(), instant set_status bypass, removes trading-specific coupling)
- [x] **QUAL-09**: Governance/timelock contract included in audit scope with tests

### Testing

- [ ] **TEST-01**: Integration tests in test-suites rebuilt against current trading API (replacing outdated oracle pattern)
- [x] **TEST-02**: Cross-contract tests covering full position lifecycle (open -> accrue -> close/liquidate/ADL)
- [x] **TEST-03**: Authorization negative tests for every privileged function (verify unauthorized callers are rejected)
- [x] **TEST-04**: Fee system tests (funding peer-to-peer conservation, borrowing curve accuracy, fee accrual over time)
- [x] **TEST-05**: Edge case tests for liquidation, ADL triggers, vault utilization limits, and market config boundaries
- [ ] **TEST-06**: Factory deployment tests (deploy_v2 atomic deployment, address precomputation)
- [ ] **TEST-07**: Price verifier tests (freshness, exponent handling, multi-price verification)
- [ ] **TEST-08**: Line coverage >= 80% measured by cargo-llvm-cov
- [ ] **TEST-09**: Mutation testing via cargo-mutants confirms tests catch real code changes
- [ ] **TEST-10**: Threat-to-test traceability matrix mapping every threat ID from the STRIDE catalog to at least one integration test (promoted from v2)

### Documentation

- [ ] **DOC-01**: Protocol specification covering fee system (funding + borrowing), position lifecycle, ADL mechanism, and decimal system
- [ ] **DOC-02**: Architecture documentation with component diagrams and data flow between contracts
- [ ] **DOC-03**: Function-level rustdoc comments on all public functions in trading, strategy-vault, factory, price-verifier
- [ ] **DOC-04**: Inline decision annotations on non-obvious code (why SCALAR_7 vs SCALAR_18, why MIN_OPEN_TIME, etc.)
- [ ] **DOC-05**: Technical docs in docs/ folder organized for both auditor and developer audiences

## v2 Requirements

Deferred to post-audit. Tracked but not in current roadmap.

### Testing Enhancements

- **TEST-V2-01**: Fuzz target updates for current API (cargo-fuzz with SorobanArbitrary)
- **TEST-V2-02**: Property-based tests using proptest for invariant verification
- **TEST-V2-03**: Formal verification via Certora Sunbeam for critical invariants

### Documentation Enhancements

- **DOC-V2-01**: Access control matrix (who can call what)
- **DOC-V2-02**: Known issues document with severity ratings
- **DOC-V2-03**: Developer onboarding guide
- **DOC-V2-04**: Invariant specification document (formal invariants)

### Threat Model Enhancements

- **THREAT-V2-01**: Historical DeFi exploit analysis mapped to Zenex attack surfaces
- **THREAT-V2-02**: Soroban resource limit empirical testing for DoS threats

## Out of Scope

| Feature | Reason |
|---------|--------|
| New contract features | Code is frozen — audit prep only |
| Treasury contract | Not in audit scope |
| Account contract | Separate repository |
| Off-chain services (keeper, relayer, backend) | Only trust boundaries are modeled, not the services themselves |
| Frontend/SDK | Not in audit scope |
| Formal verification (Certora Sunbeam) | High effort, specialized knowledge — defer to v2 |
| Multi-sig admin key design | Operational decision, not contract code |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| THREAT-01 | Phase 1 | Complete |
| THREAT-02 | Phase 1 | Complete |
| THREAT-03 | Phase 1 | Complete |
| THREAT-04 | Phase 1 | Complete |
| QUAL-01 | Phase 2 | Complete |
| QUAL-02 | Phase 2 | Complete |
| QUAL-03 | Phase 2 | Complete |
| QUAL-04 | Phase 2 | Complete |
| QUAL-05 | Phase 2 | Complete |
| QUAL-06 | Phase 2 | Complete |
| QUAL-07 | Phase 2 | Complete |
| QUAL-08 | Phase 2 | Complete |
| QUAL-09 | Phase 2 | Complete |
| TEST-01 | Phase 3 | Pending |
| TEST-02 | Phase 3 | Complete |
| TEST-03 | Phase 3 | Complete |
| TEST-04 | Phase 3 | Complete |
| TEST-05 | Phase 3 | Complete |
| TEST-06 | Phase 3 | Pending |
| TEST-07 | Phase 3 | Pending |
| TEST-08 | Phase 3 | Pending |
| TEST-09 | Phase 3 | Pending |
| TEST-10 | Phase 3 | Pending |
| DOC-01 | Phase 3 | Pending |
| DOC-02 | Phase 3 | Pending |
| DOC-03 | Phase 3 | Pending |
| DOC-04 | Phase 3 | Pending |
| DOC-05 | Phase 3 | Pending |

**Coverage:**
- v1 requirements: 28 total
- Mapped to phases: 28
- Unmapped: 0

---
*Requirements defined: 2026-03-24*
*Last updated: 2026-03-24 after roadmap revision (threat model first)*
