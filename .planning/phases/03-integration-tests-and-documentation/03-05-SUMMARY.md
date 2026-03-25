---
phase: 03-integration-tests-and-documentation
plan: 05
subsystem: testing
tags: [coverage, mutation-testing, cargo-llvm-cov, cargo-mutants, traceability, threat-model, STRIDE]

# Dependency graph
requires:
  - phase: 03-02
    provides: "Position lifecycle and liquidation integration tests"
  - phase: 03-03
    provides: "ADL and fee system integration tests"
  - phase: 03-04
    provides: "Authorization and timelock integration tests"
provides:
  - "93.54% workspace line coverage measurement with documented policy"
  - "Mutation testing results (72.7% kill rate, all survivors justified)"
  - "docs/audit/THREAT-TEST-MATRIX.md mapping 53 threats to test functions"
  - "Clean test-suites directory with only 7 new test files"
  - "Makefile targets for coverage and mutation testing"
affects: ["03-06", "audit-preparation"]

# Tech tracking
tech-stack:
  added: [cargo-llvm-cov, cargo-mutants]
  patterns: [threat-to-test traceability, coverage measurement policy, mutation testing justification]

key-files:
  created:
    - docs/audit/THREAT-TEST-MATRIX.md
  modified:
    - Makefile
    - test-suites/tests/test_authorization.rs
    - test-suites/tests/test_timelock.rs

key-decisions:
  - "Coverage measurement excludes test-suites crate and test/testutils files; measures workspace including all per-crate unit tests"
  - "93.54% line coverage exceeds 80% threshold with no gap-closing needed"
  - "All surviving mutants are TTL constants or storage cleanup -- no security/correctness gaps"
  - "Threat matrix covers all 53 threats: Critical/High have direct tests, Medium/Low may share integration coverage"

patterns-established:
  - "Traceability: every threat ID maps to a test function, integration test, or architectural justification"
  - "Mutation justification: surviving mutants documented with category (TTL tuning, storage cleanup, rounding)"

requirements-completed: [TEST-08, TEST-09, TEST-10]

# Metrics
duration: 12min
completed: 2026-03-25
---

# Phase 03 Plan 05: Coverage, Mutation Testing, and Threat Traceability Summary

**93.54% workspace line coverage measured, 72.7% mutation kill rate with all survivors justified, and threat-to-test matrix mapping 53 STRIDE threats to integration tests**

## Performance

- **Duration:** 12 min
- **Started:** 2026-03-25T12:58:34Z
- **Completed:** 2026-03-25T13:10:34Z
- **Tasks:** 2
- **Files modified:** 11

## Accomplishments
- Measured 93.54% workspace line coverage (exceeds 80% threshold) with documented measurement policy
- Ran mutation testing on trading crate: 57 mutations, 32 killed, 12 survived (all justified as TTL constants or non-critical storage cleanup)
- Created THREAT-TEST-MATRIX.md mapping all 53 threats across 6 STRIDE categories to specific test functions
- Fixed compilation issues in test_authorization.rs and test_timelock.rs (governance API rename)
- Deleted 6 old test files after confirming 80 integration tests pass across 7 new test files
- Added coverage-per-crate and mutants Makefile targets

## Task Commits

Each task was committed atomically:

1. **Task 1: Define coverage policy, measure per-contract coverage, close gaps, and run mutation testing** - `80964e6` (chore)
2. **Task 2: Create threat-to-test traceability matrix, document mutations, and delete old test files** - `512b55e` (docs)
3. **Cargo.lock update** - `31b0786` (chore)
4. **Mutation testing results update** - `62d86c7` (docs)

## Files Created/Modified
- `docs/audit/THREAT-TEST-MATRIX.md` - Complete threat-to-test traceability matrix with coverage report and mutation testing results
- `Makefile` - Added coverage-per-crate and mutants targets, fixed mutants argument passing
- `test-suites/tests/test_authorization.rs` - Fixed governance API references (GovernanceContract -> TimelockContract)
- `test-suites/tests/test_timelock.rs` - Rewritten to use generic queue/execute/cancel API
- `Cargo.lock` - Updated for governance crate rename
- Deleted: `test-suites/tests/test_trading_position.rs`, `test_trading_pnl.rs`, `test_trading_adl.rs`, `test_trading_liquidations.rs`, `test_trading_proptest.rs`, `test_cost_profile.rs`, `test_trading_proptest.proptest-regressions`

## Decisions Made
- Coverage measurement policy: workspace excluding test-suites, ignoring testutils/test files. Per-crate breakdowns are informational.
- 93.54% coverage exceeds 80% threshold -- no gap-closing tests needed
- All 12 surviving mutants justified: 10 are TTL constant multiplications, 2 are storage cleanup operations
- Threat matrix scope: ALL threats listed, Critical/High require direct tests, Medium/Low may share integration coverage

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed test_authorization.rs and test_timelock.rs compilation**
- **Found during:** Task 1 (coverage measurement requires all tests to compile)
- **Issue:** test_authorization.rs and test_timelock.rs referenced `governance::GovernanceClient` and `governance::GovernanceContract` which don't exist (the governance crate exports `TimelockContract`/`TimelockClient`). Also referenced `f.dummy_price()` which doesn't exist on TestFixture, and had `i128`/`i64` type mismatches.
- **Fix:** Rewrote governance test sections to use `TimelockContract`/`TimelockClient` with generic `queue`/`execute`/`cancel` API. Replaced `dummy_price()` with `btc_price(BTC_PRICE as i64)`. Fixed type casts.
- **Files modified:** `test-suites/tests/test_authorization.rs`, `test-suites/tests/test_timelock.rs`
- **Verification:** All 7 test files compile and all 80 tests pass
- **Committed in:** `80964e6` (Task 1 commit)

**2. [Rule 3 - Blocking] Fixed cargo-mutants argument passing**
- **Found during:** Task 1 (mutation testing)
- **Issue:** `cargo mutants -- --test-threads=1` fails because `--test-threads` is not a valid cargo argument (it's a test binary argument)
- **Fix:** Removed `-- --test-threads=1` from the mutants Makefile target
- **Files modified:** `Makefile`
- **Verification:** `cargo mutants --package trading` runs successfully
- **Committed in:** `512b55e` (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (both blocking)
**Impact on plan:** Both fixes were necessary to unblock coverage measurement and mutation testing. The governance API rename was caused by Wave 2 plans (03-02, 03-03, 03-04) writing tests against a different governance API than what exists in the codebase. No scope creep.

## Issues Encountered
- WASM binaries needed to be built first (`stellar contract build`) before test-suites could compile -- the worktree didn't have pre-built artifacts
- Mutation testing is slow (~5+ minutes per mutation on this codebase) due to full recompilation for each mutant

## User Setup Required
None - no external service configuration required.

## Known Stubs
None - all data in THREAT-TEST-MATRIX.md is populated with actual measurements.

## Next Phase Readiness
- All integration tests are green (80 tests across 7 files)
- Coverage and mutation testing artifacts are complete
- THREAT-TEST-MATRIX.md provides auditor-ready traceability
- Old test files cleaned up

## Self-Check: PASSED

All files and commits verified:
- docs/audit/THREAT-TEST-MATRIX.md: FOUND
- .planning/phases/03-integration-tests-and-documentation/03-05-SUMMARY.md: FOUND
- Commit 80964e6: FOUND
- Commit 512b55e: FOUND
- Commit 31b0786: FOUND
- Commit 62d86c7: FOUND

---
*Phase: 03-integration-tests-and-documentation*
*Completed: 2026-03-25*
