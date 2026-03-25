---
phase: 03-integration-tests-and-documentation
plan: 07
subsystem: documentation
tags: [rustdoc, audit, inline-comments, soroban, smart-contracts]

requires:
  - phase: 03-06
    provides: protocol spec and architecture docs for cross-referencing
provides:
  - Comprehensive rustdoc on all public entry points across 6 contracts
  - Inline WHY annotations on non-obvious design decisions
  - Formula documentation on critical math functions (rates, settlement, ADL)
  - Field-level unit documentation on all config/state types
affects: [audit-readiness, developer-onboarding]

tech-stack:
  added: []
  patterns:
    - "WHY annotation pattern for non-obvious design decisions"
    - "Priority-based documentation (entry points > math > types > storage)"

key-files:
  created: []
  modified:
    - trading/src/interface.rs
    - trading/src/contract.rs
    - trading/src/types.rs
    - trading/src/trading/rates.rs
    - trading/src/trading/market.rs
    - trading/src/trading/position.rs
    - trading/src/trading/adl.rs
    - trading/src/trading/execute.rs
    - trading/src/trading/actions.rs
    - trading/src/trading/config.rs
    - trading/src/validation.rs
    - trading/src/errors.rs
    - trading/src/constants.rs
    - trading/src/storage.rs
    - trading/src/events.rs
    - strategy-vault/src/contract.rs
    - factory/src/lib.rs
    - price-verifier/src/lib.rs
    - price-verifier/src/pyth.rs
    - governance/src/lib.rs
    - governance/src/events.rs
    - treasury/src/lib.rs

key-decisions:
  - "Prioritized documentation by audit value: entry points > math > types > ADL > storage/events"
  - "WHY annotations only on non-obvious code (rounding direction, SCALAR choice, fee asymmetry, MIN_OPEN_TIME, peer-to-peer funding)"
  - "Trivial getters get one-line descriptions only per D-12 convention"

patterns-established:
  - "WHY annotation: '// WHY: ...' for inline rationale on non-obvious design decisions"
  - "Rustdoc format: brief, Parameters (with units), Panics (with error codes), auth model"

requirements-completed: [DOC-03, DOC-04]

duration: 17min
completed: 2026-03-25
---

# Phase 03 Plan 07: In-Code Documentation Summary

**Audit-focused rustdoc on all 6 contracts with WHY annotations on rounding, fee asymmetry, index-based settlement, ADL, circuit breaker hysteresis, and peer-to-peer funding**

## Performance

- **Duration:** 17 min
- **Started:** 2026-03-25T12:36:19Z
- **Completed:** 2026-03-25T12:53:19Z
- **Tasks:** 2
- **Files modified:** 22

## Accomplishments

- All public entry points across 6 contracts have rustdoc with description, parameter units (SCALAR_7/SCALAR_18/token_decimals), auth model, and panics/error codes
- Critical math functions documented with formulas: calc_funding_rate, calc_borrowing_rate, Position::settle (index-based settlement), MarketData::accrue (borrowing then funding order)
- 20+ inline WHY annotations covering: rounding direction choices (ceil for fees, floor for PnL), SCALAR_7 vs SCALAR_18 precision tiers, MIN_OPEN_TIME anti-arbitrage, dominant-side-only borrowing, entry-weighted ADL, peer-to-peer funding, margin > liq_fee invariant, circuit breaker hysteresis
- Field-level documentation on TradingConfig, MarketConfig, MarketData, Position, and all error variants
- Price-verifier includes Pyth Lazer binary format specification in module-level doc comment
- Factory includes deterministic address precomputation and front-run protection rationale

## Task Commits

Each task was committed atomically:

1. **Task 1: Trading crate rustdoc and WHY annotations** - `f8d2788` (docs)
2. **Task 2: Vault, factory, price-verifier, governance, treasury rustdoc** - `d299398` (docs)

## Files Created/Modified

### Trading crate (15 files)
- `trading/src/interface.rs` - Full rustdoc on every Trading trait method with params, panics, auth model
- `trading/src/contract.rs` - __constructor docs, verify_price/verify_prices helper docs
- `trading/src/types.rs` - Field-level docs with units on TradingConfig, MarketConfig, MarketData, Position, ContractStatus state machine
- `trading/src/trading/rates.rs` - Formula docs and WHY: peer-to-peer funding, util^5 curve rationale, rounding direction
- `trading/src/trading/market.rs` - Market struct docs, open() fee logic, accrue() order and index mechanics, entry_wt rationale
- `trading/src/trading/position.rs` - Settlement struct field docs, settle() index-based formula, require_closable MIN_OPEN_TIME WHY
- `trading/src/trading/adl.rs` - execute_update_status circuit breaker logic, do_adl entry-weighted O(1) WHY
- `trading/src/trading/execute.rs` - Transfers batching WHY, execute_trigger CEI transfer order
- `trading/src/trading/actions.rs` - All action functions (create_limit, create_market, close, modify_collateral, set_triggers, apply_funding)
- `trading/src/trading/config.rs` - Config/market CRUD function docs
- `trading/src/validation.rs` - Status guards, config validation with fee_dom >= fee_non_dom invariant WHY, margin > liq_fee WHY
- `trading/src/errors.rs` - One-line description for every TradingError variant
- `trading/src/constants.rs` - SCALAR_7 vs SCALAR_18 precision rationale, circuit breaker hysteresis WHY, MIN_OPEN_TIME WHY, all cap constants
- `trading/src/storage.rs` - TTL tier rationale (instance 30d, market 45d, position 14d)
- `trading/src/events.rs` - One-line description for every event struct

### Other contracts (7 files)
- `strategy-vault/src/contract.rs` - Module docs, deposit lock WHY, decimals_offset inflation attack, strategy_withdraw auth model
- `factory/src/lib.rs` - deploy() deterministic address WHY, deployment order, compute_salts front-run protection
- `price-verifier/src/lib.rs` - PriceData field docs, __constructor params, verify_price/verify_prices
- `price-verifier/src/pyth.rs` - Pyth Lazer binary format module-level doc, verify_and_extract 7-step verification, check_staleness abs_diff WHY
- `governance/src/lib.rs` - Timelock trait docs, queue generic invoke WHY, execute CEI WHY, set_status bypass WHY, set_delay PendingDelay WHY
- `governance/src/events.rs` - Event descriptions
- `treasury/src/lib.rs` - get_fee floor rounding WHY, set_rate 50% cap, error docs

## Decisions Made

- **Priority-based documentation**: Documented in order of audit value (entry points first, then math, then types, then storage/events) to ensure the highest-value documentation was completed first
- **WHY-only-where-non-obvious**: Added WHY annotations only where the code's intent cannot be inferred from reading it -- rounding direction choices, scalar precision tiers, fee asymmetry logic, time guards, and architectural patterns like ADL index-based settlement
- **Trivial getter one-liners**: Simple getters (get_position, get_config, etc.) got one-line descriptions only, avoiding verbose format per D-12 convention

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

- Factory tests require compiled WASM binaries (`target/wasm32v1-none/release/*.wasm`) which are not present in the worktree. This is a pre-existing condition and does not affect the documentation changes. All other crates' tests pass.

## Known Stubs

None - documentation-only changes, no functional code or data stubs.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- All 6 contracts now have comprehensive in-code documentation for auditors
- DOC-03 (public function rustdoc) and DOC-04 (inline decision annotations) complete
- Auditors can understand intent, units, auth model, and design rationale without guessing

---
*Phase: 03-integration-tests-and-documentation*
*Completed: 2026-03-25*
