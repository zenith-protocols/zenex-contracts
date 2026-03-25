---
phase: 03-integration-tests-and-documentation
plan: 03
subsystem: testing
tags: [adl, funding, borrowing, fee-system, integration-tests, soroban, pyth-lazer]

# Dependency graph
requires:
  - phase: 03-integration-tests-and-documentation
    plan: 01
    provides: "TestFixture with real PriceVerifier, pyth_helper for Ed25519-signed price payloads"
provides:
  - "9 ADL integration tests: threshold, trigger/reduce, close-after-ADL, short-side, status restoration, compounding, post-ADL, 100% cap, entry-weighted"
  - "7 fee system integration tests: funding zero-sum, dominant-side pays, balanced rate, borrowing curve at 4 utilization points, dominant-only borrowing, time-proportional accrual, token conservation"
affects: [03-04, 03-05]

# Tech tracking
tech-stack:
  added: []
  patterns: [multi-feed-price-updates-for-update-status, borrowing-index-delta-verification, token-conservation-invariant]

key-files:
  created:
    - test-suites/tests/test_adl.rs
    - test-suites/tests/test_fee_accrual.rs
  modified: []

key-decisions:
  - "ADL tests use multi-feed price updates (BTC+ETH+XLM) because update_status requires all market prices"
  - "Borrowing curve verified via borrowing index delta (l_borr_idx) instead of non-existent borr_rate field -- delta over 1 hour equals the hourly rate"
  - "Token conservation tolerance set to 2 units per position to account for fixed-point rounding across multiple contract calls"

patterns-established:
  - "price_update_btc helper: builds multi-feed signed price update with custom BTC and default ETH/XLM prices for ADL/update_status tests"
  - "Borrowing index delta verification: compare l_borr_idx before/after 1 hour accrual against off-chain formula computation"
  - "Token conservation test pattern: sum all balances (users + vault + treasury + trading contract) before and after, assert within tolerance"

requirements-completed: [TEST-04, TEST-05]

# Metrics
duration: 7min
completed: 2026-03-25
---

# Phase 03 Plan 03: ADL and Fee System Integration Tests Summary

**ADL integration tests (9 tests) and fee system conservation/borrowing curve tests (7 tests) with real Ed25519-signed Pyth Lazer price verification**

## Performance

- **Duration:** 7 min
- **Started:** 2026-03-25T12:37:51Z
- **Completed:** 2026-03-25T12:44:42Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- 9 ADL tests covering threshold detection, trigger mechanics, settlement after ADL, short-side ADL, status restoration, compounding, post-ADL positions, 100% reduction cap, and entry-weighted tracking
- 7 fee system tests proving funding zero-sum conservation, dominant-side pays more, balanced-rate zero, borrowing curve formula at 4 utilization points (25%, 50%, 75%, 90%), dominant-only borrowing, time-proportional accrual, and total token conservation
- All 16 tests use real PriceVerifier contract with Ed25519-signed Pyth Lazer payloads -- no mocks
- Borrowing curve verified off-chain against formula r_base * (1 + r_var * util^5) * r_borrow at each utilization point

## Task Commits

Each task was committed atomically:

1. **Task 1: Write ADL integration tests** - `48689b3` (feat)
2. **Task 2: Write fee system conservation and borrowing curve tests** - `3fb9d73` (feat)

## Files Created/Modified
- `test-suites/tests/test_adl.rs` - 9 ADL integration tests: threshold revert, trigger/reduce aggregates, close-after-ADL settlement, short-side ADL, status restoration, compounding, post-ADL unaffected, 100% reduction cap, entry-weighted tracking
- `test-suites/tests/test_fee_accrual.rs` - 7 fee system tests: funding zero-sum token conservation, dominant side pays more, balanced-rate zero, borrowing curve at 4 utilization points, dominant-only borrowing, time-proportional accrual, token conservation after open/close cycle

## Decisions Made
- ADL's `update_status` requires prices for ALL registered markets (BTC, ETH, XLM). Created `price_update_btc` helper that provides custom BTC price with default ETH/XLM prices in a single signed payload.
- Borrowing curve test uses borrowing index delta approach: since `MarketData` has no `borr_rate` field (rate is computed dynamically in `accrue()`), we measure `l_borr_idx` before/after a 1-hour period. The delta equals the hourly rate, which we compare against the off-chain formula computation.
- Token conservation tolerance set at 2 units per position (accounts for fixed-point rounding across multiple contract calls -- open fees, close fees, funding, borrowing all involve separate fixed_mul/fixed_div operations).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Built WASM contracts before test execution**
- **Found during:** Task 1 (ADL tests)
- **Issue:** test-suites depends on compiled WASM for trading, vault, price-verifier, treasury, governance contracts via `include_bytes!`, but worktree had no WASM artifacts
- **Fix:** Ran `stellar contract build` to compile all contract WASMs
- **Files modified:** target/ (build artifacts, not committed)
- **Verification:** All tests compile and pass

**2. [Rule 1 - Bug] Fixed NotionalAboveMaximum in borrowing curve test**
- **Found during:** Task 2 (borrowing curve at utilization points)
- **Issue:** At 25% utilization of 100M vault, target notional = 25M exceeds default max_notional of 10M, causing Error #737
- **Fix:** Added `set_config` to bump max_notional to 1B before opening large positions in the borrowing curve test
- **Files modified:** test-suites/tests/test_fee_accrual.rs
- **Verification:** Borrowing curve test passes at all 4 utilization points
- **Committed in:** 3fb9d73 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes necessary for test execution. The WASM build is a standard prerequisite. The max_notional bump is a test setup concern, not a code change. No scope creep.

## Issues Encountered
- Worktree was on the pre-Plan-03-01 codebase (missing real PriceVerifier fixture). Resolved by fast-forward merging the audit branch, which brought in the updated TestFixture with real contract registration.

## User Setup Required
None - no external service configuration required.

## Known Stubs
None - all test functionality is fully wired.

## Next Phase Readiness
- ADL and fee system integration tests are complete and ready
- Subsequent plans can use the `price_update_btc` multi-feed helper pattern for any tests requiring `update_status`
- Borrowing curve off-chain verification pattern can be extended for additional utilization scenarios
- Old test files (test_trading_adl.rs, test_trading_pnl.rs) still reference old fixture API -- they are NOT broken by this plan but will need updating in future

---
*Phase: 03-integration-tests-and-documentation*
*Completed: 2026-03-25*

## Self-Check: PASSED
- test-suites/tests/test_adl.rs: FOUND (9 test functions)
- test-suites/tests/test_fee_accrual.rs: FOUND (7 test functions)
- .planning/phases/03-integration-tests-and-documentation/03-03-SUMMARY.md: FOUND
- Commit 48689b3 (Task 1): FOUND
- Commit 3fb9d73 (Task 2): FOUND
- No MockPriceVerifier or dummy_price in test_adl.rs: VERIFIED
- All 16 tests pass: VERIFIED
