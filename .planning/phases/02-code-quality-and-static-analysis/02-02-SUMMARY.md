---
phase: 02-code-quality-and-static-analysis
plan: 02
subsystem: trading, price-verifier
tags: [unwrap, panic-safety, liquidation, stale-price, threat-model, soroban]

# Dependency graph
requires:
  - phase: 01-threat-modeling
    provides: "THREAT-MODEL.md with STRIDE threat catalog and T-TAMP-14 as last tampering threat"
provides:
  - "Zero unsafe unwrap/expect in production code of trading and price-verifier"
  - "require_liquidatable guard with publish_time stale-price check on liquidation path"
  - "StalePrice (749) error variant for distinct stale-price errors"
  - "publish_time field threaded through Market struct from PriceData"
  - "T-TAMP-15 same-block price arbitrage threat documented in THREAT-MODEL.md"
  - "Verification that QUAL-01, QUAL-03, D-09 are already resolved"
affects: [03-integration-tests, audit-readiness]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "unwrap_or_else with panic_with_error for unsafe unwrap sites"
    - "unwrap_optimized with SAFETY comments for bounded-index and constructor-set values"
    - "require_liquidatable as separate guard from require_closable (no MIN_OPEN_TIME for liquidations)"

key-files:
  created: []
  modified:
    - "trading/src/contract.rs"
    - "trading/src/trading/adl.rs"
    - "trading/src/trading/position.rs"
    - "trading/src/trading/execute.rs"
    - "trading/src/trading/market.rs"
    - "trading/src/errors.rs"
    - "price-verifier/src/lib.rs"
    - "price-verifier/src/pyth.rs"
    - "price-verifier/src/storage.rs"
    - "docs/THREAT-MODEL.md"

key-decisions:
  - "Used unwrap_or_else with panic_with_error for 3 UNSAFE sites (verify_price, ADL feed lookup) giving clear error codes instead of opaque panics"
  - "Used unwrap_optimized for 6 SAFE sites (bounded loops, constructor-set values) with SAFETY comments documenting invariants"
  - "StalePrice (749) as distinct error from PositionTooNew (748): semantically different -- stale price vs opened too recently"
  - "publish_time threaded through Market struct rather than passed as separate parameter to maintain existing function signatures"

patterns-established:
  - "unwrap_or_else with panic_with_error: pattern for unsafe Option/Result unwraps where error code is needed"
  - "unwrap_optimized with SAFETY comment: pattern for proven-safe unwraps (bounded loops, constructor invariants)"
  - "require_liquidatable vs require_closable: separate guards for liquidation (no MIN_OPEN_TIME, has publish_time check) vs regular close (has MIN_OPEN_TIME, no publish_time check)"

requirements-completed: [QUAL-01, QUAL-02, QUAL-03]

# Metrics
duration: 5min
completed: 2026-03-24
---

# Phase 02 Plan 02: Unwrap Safety and Liquidation Guard Summary

**Zero unsafe unwrap/expect in production code, require_liquidatable guard with StalePrice error for stale-price liquidation attacks, and T-TAMP-15 same-block price arb threat documented**

## Performance

- **Duration:** 5 min
- **Started:** 2026-03-24T21:07:19Z
- **Completed:** 2026-03-24T21:12:42Z
- **Tasks:** 2
- **Files modified:** 10

## Accomplishments
- Eliminated all 9 unsafe `.unwrap()` and `.expect()` calls from production code in trading and price-verifier crates: 3 UNSAFE sites use `unwrap_or_else` with proper error codes (720), 6 SAFE sites use `unwrap_optimized` with SAFETY comments
- Added `require_liquidatable` guard with `publish_time >= position.created_at` check, using distinct `StalePrice` (749) error variant, threaded through Market struct from PriceData
- Added 4 unit tests for `require_liquidatable` covering stale price rejection, valid price acceptance, unfilled position rejection, and immediate liquidation
- Documented T-TAMP-15 (same-block open+close price arbitrage) in THREAT-MODEL.md sections 2.2, 2.7, and 3.1.2
- Verified QUAL-01 (collateral negativity) already mitigated by `validate()` after fee deduction
- Verified QUAL-03 (token decimal validation) not needed -- math is decimal-agnostic with SCALAR_7 ratios
- Verified D-09 (config validation margin > liq_fee) already present in `require_valid_market_config`

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix all unwrap/expect calls in trading and price-verifier production code** - `4de2c54` (fix)
2. **Task 2: Add require_liquidatable guard with publish_time threading, StalePrice error, tests, and THREAT-MODEL update** - `b673f39` (feat)

## Files Created/Modified
- `trading/src/contract.rs` - verify_price uses unwrap_or_else, UpgradeableInternal uses unwrap_optimized
- `trading/src/trading/adl.rs` - Feed lookup uses unwrap_or_else, bounded loop uses unwrap_optimized
- `trading/src/trading/position.rs` - Added require_liquidatable method + 4 unit tests
- `trading/src/trading/execute.rs` - apply_liquidation calls require_liquidatable(e, market.publish_time)
- `trading/src/trading/market.rs` - Added publish_time field to Market struct, populated from PriceData
- `trading/src/errors.rs` - Added StalePrice = 749 error variant
- `price-verifier/src/lib.rs` - verify_price uses unwrap_optimized with SAFETY comment
- `price-verifier/src/pyth.rs` - Bounded loop uses unwrap_optimized with SAFETY comment
- `price-verifier/src/storage.rs` - All 3 getters use unwrap_optimized (set in constructor)
- `docs/THREAT-MODEL.md` - T-TAMP-15 same-block price arb in sections 2.2, 2.7, and 3.1.2

## Decisions Made
- **unwrap_or_else vs unwrap_optimized:** Used `unwrap_or_else(|| panic_with_error!(...))` for 3 sites where the unwrap could genuinely fail at runtime (empty Vec from verifier, missing feed in ADL). Used `unwrap_optimized()` for 6 sites where the value is provably present (constructor invariants, bounded loop indices). Each `unwrap_optimized` has a `// SAFETY:` comment documenting why it's safe.
- **StalePrice (749) as distinct error:** Chose a new error variant rather than reusing `PositionTooNew` (748) because the semantics are different: PositionTooNew means "opened too recently to close normally", StalePrice means "price data predates the position opening."
- **publish_time in Market struct:** Threaded through the existing Market struct rather than adding a separate parameter to `apply_liquidation`, maintaining the existing function signatures.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Verification Results

### QUAL-01 (D-06): Collateral Negativity -- Already Mitigated
After `position.col -= base_fee + impact_fee` in `Market::open()`, the next call is `position.validate()` which checks `self.col <= 0` and panics with `NegativeValueNotAllowed` (735). No code change needed.

### QUAL-03 (D-07): Token Decimal Validation -- Not Needed
The math uses SCALAR_7 ratios for rates/fees and SCALAR_18 for indices. Notional and collateral are in the same token denomination. The math is inherently decimal-agnostic. No code change needed.

### D-09: Config Validation (margin > liq_fee) -- Already Present
`require_valid_market_config()` in `trading/src/validation.rs` lines 85-88 checks `config.margin <= config.liq_fee` and panics with `InvalidConfig` (702). This is called from `execute_set_market` in the config path. No code change needed.

## Next Phase Readiness
- Production code is now panic-safe: zero unhandled unwrap/expect calls
- Liquidation path has stale-price protection without blocking timely liquidations
- T-TAMP-15 threat is fully documented for auditor reference
- All 88 tests pass (81 trading + 7 price-verifier)
- Ready for Phase 3 integration tests to cover the new require_liquidatable guard

## Self-Check: PASSED

All 10 modified files exist on disk. Both task commits (4de2c54, b673f39) verified in git log.

---
*Phase: 02-code-quality-and-static-analysis*
*Completed: 2026-03-24*
