# Roadmap: Zenex Contracts Audit Preparation

## Overview

Prepare the Zenex perpetual futures contracts for security audit by threat modeling first, then fixing known bugs, then building tests and documentation derived from the threat model. The contracts are feature-complete and code-frozen -- this roadmap covers only threat analysis, bug fixes (code freeze exceptions), tests, and documentation artifacts. The critical path is: threat model first (identifies what to test and where the risk lives), then clean the code (so tests validate correct behavior), then build integration tests driven by threat findings alongside documentation.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [ ] **Phase 1: Threat Model** - STRIDE analysis, data flow diagrams, trust boundary documentation -- produces the threat catalog that drives all test design
- [ ] **Phase 2: Code Quality and Static Analysis** - Fix critical bugs, eliminate unsafe code, pin dependencies, pass all static analysis tools
- [ ] **Phase 3: Integration Tests and Documentation** - Rebuild test-suites from threat model findings, achieve 80%+ coverage, complete protocol spec and architecture docs

## Phase Details

### Phase 1: Threat Model
**Goal**: A complete threat catalog exists that identifies every attack surface, trust boundary, and threat category -- this catalog becomes the blueprint for integration test design in Phase 3
**Depends on**: Nothing (first phase)
**Requirements**: THREAT-01, THREAT-02, THREAT-03, THREAT-04
**Success Criteria** (what must be TRUE):
  1. A STRIDE threat model exists following Stellar's four-section framework (scope+DFDs, threats, mitigations, retrospective) covering trading, vault, factory, and price-verifier contracts (THREAT-01)
  2. Data flow diagrams cover all contract interactions including external system boundaries (keeper, relayer, oracle, LP, governance) with data types and trust levels annotated on each flow (THREAT-02)
  3. All 7 trust boundaries are documented with their attack surfaces, and each boundary has at least one identified threat with severity rating and mitigation status (THREAT-03, THREAT-04)
  4. The threat catalog is structured so each threat can be referenced by ID (e.g., T-SPOOF-01) for traceability into test cases in Phase 3
**Plans**: 2 plans

Plans:
- [x] 01-01-PLAN.md — Scope, DFDs, trust boundaries, and threat catalog (Sections 1-2)
- [x] 01-02-PLAN.md — Mitigations and retrospective (Sections 3-4)

### Phase 2: Code Quality and Static Analysis
**Goal**: The codebase has zero known bugs, zero unsafe unwrap calls, zero static analysis findings, and all dependencies are pinned -- auditors will not find mechanical issues
**Depends on**: Phase 1 (threat model may surface additional code issues to fix)
**Requirements**: QUAL-01, QUAL-02, QUAL-03, QUAL-04, QUAL-05, QUAL-06, QUAL-07, QUAL-08, QUAL-09
**Success Criteria** (what must be TRUE):
  1. Collateral can never go negative after fee deduction in any position operation (QUAL-01 fix verified by existing unit tests passing)
  2. Zero `.unwrap()` calls remain in production code paths across all in-scope contracts (QUAL-02)
  3. ~~Token decimal validation~~ DROPPED: math is decimal-agnostic, deployer sets appropriate config values (QUAL-03 resolved)
  4. `cargo scout-audit` reports zero critical and zero high findings across trading, strategy-vault, factory, price-verifier, and timelock (QUAL-04)
  5. `cargo clippy -- -D warnings` passes clean on all in-scope contracts, `cargo-audit` and `cargo-deny` report no known vulnerabilities, and all dependency versions are pinned to exact commits or versions (QUAL-05, QUAL-06, QUAL-07)
  6. A generic timelock contract replaces governance — supports queue/execute/cancel for any target contract call, with instant bypass for set_status (emergency halt). Trading-specific types removed. (QUAL-08, QUAL-09)
**Plans**: 3 plans

Plans:
- [x] 02-01-PLAN.md — Generic timelock contract replacing governance (QUAL-08, QUAL-09)
- [x] 02-02-PLAN.md — Unwrap fixes, liquidation guard, threat model update (QUAL-01, QUAL-02, QUAL-03)
- [x] 02-03-PLAN.md — Static analysis, dependency pinning, clippy (QUAL-04, QUAL-05, QUAL-06, QUAL-07)

### Phase 3: Integration Tests and Documentation
**Goal**: Auditors can run a complete integration test suite -- derived from the Phase 1 threat catalog -- that exercises every critical contract path, proves authorization enforcement, and demonstrates 80%+ line coverage; alongside a complete protocol spec and architecture documentation
**Depends on**: Phase 1 (threat catalog drives test design), Phase 2 (code must be correct before testing)
**Requirements**: TEST-01, TEST-02, TEST-03, TEST-04, TEST-05, TEST-06, TEST-07, TEST-08, TEST-09, TEST-10, DOC-01, DOC-02, DOC-03, DOC-04, DOC-05
**Success Criteria** (what must be TRUE):
  1. `test-suites` compiles and all integration tests pass against the current trading API with the new price verifier pattern (TEST-01)
  2. A full position lifecycle test exists: open -> accrue fees -> close with profit, close with loss, liquidation, and ADL -- each as a separate test case (TEST-02, TEST-05)
  3. Every `require_auth` call in trading, vault, factory, and price-verifier has a corresponding negative test that proves unauthorized callers are rejected without `mock_all_auths` (TEST-03)
  4. Fee system tests prove funding is zero-sum (long paid equals short received) and borrowing curve matches the formula `r_base * (1 + r_var * util^5)` at multiple utilization points (TEST-04)
  5. `cargo-llvm-cov` reports >= 80% line coverage across in-scope contracts, and `cargo-mutants` confirms tests catch real code changes with an acceptable kill rate (TEST-08, TEST-09)
  6. A threat-to-test traceability matrix exists mapping every threat ID from the Phase 1 catalog to at least one integration test, with no high/critical threats left untested (TEST-10)
  7. A protocol specification document covers the fee system, position lifecycle, ADL mechanism, and decimal system with enough precision that an auditor can independently verify the math (DOC-01)
  8. Architecture documentation with component diagrams, data flow, and a docs/ folder organized for auditor and developer audiences exists, and every public function has a rustdoc comment (DOC-02, DOC-03, DOC-04, DOC-05)
**Plans**: 7 plans

Plans:
- [x] 03-01-PLAN.md — TestFixture rebuild with real PriceVerifier, Pyth helper (with confidence), factory deployment + price verifier integration tests (TEST-01, TEST-06, TEST-07)
- [x] 03-02-PLAN.md — Position lifecycle and liquidation integration tests (TEST-02, TEST-05)
- [ ] 03-03-PLAN.md — ADL integration tests and fee system conservation/accuracy tests (TEST-04, TEST-05)
- [x] 03-04-PLAN.md — Authorization negative tests and timelock integration tests (TEST-03)
- [ ] 03-05-PLAN.md — Coverage measurement, mutation testing, threat-to-test traceability matrix, old test cleanup (TEST-08, TEST-09, TEST-10)
- [x] 03-06-PLAN.md — Protocol specification, architecture docs, deployment docs, docs folder structure (DOC-01, DOC-02, DOC-05)
- [x] 03-07-PLAN.md — Rustdoc comments and inline decision annotations across all contracts (DOC-03, DOC-04)

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Threat Model | 2/2 | Complete | 2026-03-24 |
| 2. Code Quality and Static Analysis | 3/3 | Complete | 2026-03-24 |
| 3. Integration Tests and Documentation | 0/7 | Not started | - |
