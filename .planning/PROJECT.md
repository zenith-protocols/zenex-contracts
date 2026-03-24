# Zenex Contracts — Audit Preparation

## What This Is

Preparing the Zenex perpetual futures trading protocol smart contracts for a security audit. The contracts (trading, strategy-vault, factory, price-verifier) are feature-complete and need threat modeling, comprehensive integration tests, in-code documentation, and technical documentation to give auditors full context and confidence.

## Core Value

Every attack surface is identified, tested, and documented — auditors can verify the protocol's safety without guessing intent.

## Requirements

### Validated

- ✓ Trading contract (positions, fees, funding, borrowing, ADL, liquidation) — existing
- ✓ Strategy vault contract (collateral management, single-strategy pattern) — existing
- ✓ Factory contract (atomic vault+trading deployment via deploy_v2) — existing
- ✓ Price verifier contract (Pyth oracle integration, price freshness) — existing
- ✓ Fee system (funding peer-to-peer, borrowing utilization curve) — existing
- ✓ ADL system (auto-deleveraging) — existing
- ✓ Fixed-point math (SCALAR_7, SCALAR_18, price_scalar) — existing

### Active

- [ ] Threat model following Stellar's threat modeling framework (on-chain + trust boundaries)
- [ ] Integration tests in test-suites covering all critical paths and threat-model-derived cases
- [ ] In-code documentation: function-level docs, invariant annotations, decision rationale
- [ ] Technical documentation (docs/ folder): protocol spec, architecture, API reference, deployment guide
- [ ] Test coverage sufficient for audit confidence

### Out of Scope

- New features or protocol changes — code is frozen, documentation and tests only
- Treasury contract — not in audit scope
- Account contract — separate repository, not in scope
- Off-chain services (keeper, relayer, backend) — only their trust boundaries with contracts are modeled
- Frontend or SDK — not in audit scope

## Context

- **Protocol**: Perpetual futures trading on Stellar/Soroban
- **Contracts in scope**: trading, strategy-vault, factory, price-verifier
- **Current test state**: test-suites integration tests are mostly outdated and out of sync with the current trading API (old oracle pattern)
- **Decimal system**: SCALAR_7 for rates/fees, SCALAR_18 for indices/funding, price_scalar derived from Pyth exponent
- **Key patterns**: Factory deploys vault+trading atomically using `deployed_address()` to resolve circular deps; `__constructor` pattern (no separate initialize); single strategy per vault
- **Trust boundaries**: Contracts interact with Pyth oracle (off-chain price relay), keeper (liquidations), relayer (transaction submission), and admin (config updates)
- **Threat model reference**: https://developers.stellar.org/docs/build/security-docs/threat-modeling

## Constraints

- **Code freeze**: No functional changes to contracts — only docs, tests, and threat model artifacts
- **Tech stack**: Rust/Soroban, `soroban-fixed-point-math`, Pyth oracle
- **Test framework**: Soroban test framework with `#[cfg(any(test, feature = "testutils"))]` gating
- **Threat model format**: Must follow Stellar's threat modeling framework

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| On-chain + trust boundary scope for threat model | Auditors need to understand where contracts trust external systems (keeper, relayer, oracle) | — Pending |
| Rebuild test-suites from scratch vs. update | Tests are mostly outdated against current API | — Pending |
| Both auditor and developer docs | Auditors need invariants/attack surfaces; future devs need architecture/API reference | — Pending |

## Evolution

This document evolves at phase transitions and milestone boundaries.

**After each phase transition** (via `/gsd:transition`):
1. Requirements invalidated? → Move to Out of Scope with reason
2. Requirements validated? → Move to Validated with phase reference
3. New requirements emerged? → Add to Active
4. Decisions to log? → Add to Key Decisions
5. "What This Is" still accurate? → Update if drifted

**After each milestone** (via `/gsd:complete-milestone`):
1. Full review of all sections
2. Core Value check — still the right priority?
3. Audit Out of Scope — reasons still valid?
4. Update Context with current state

---
*Last updated: 2026-03-24 after initialization*
