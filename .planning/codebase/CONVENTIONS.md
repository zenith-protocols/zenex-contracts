# Coding Conventions

**Analysis Date:** 2026-03-24

## Naming Patterns

**Files:**
- Contract modules: `lib.rs` as entry point with conditional compilation gates
- Test files: `test.rs` within the crate (e.g., `trading/src/test.rs`)
- Integration tests: `tests/test_*.rs` pattern in `test-suites/tests/`
- Test utilities: `testutils.rs` gated with `#[cfg(any(test, feature = "testutils"))]`
- Support modules: Descriptive names like `market.rs`, `position.rs`, `rates.rs`

**Functions:**
- Snake case: `calc_borrowing_rate()`, `verify_prices()`, `update_stats()`
- Getter methods: `get_position()`, `get_market_config()`, `total_assets()`
- Setter methods: `set_market_data()`, `set_market_config()`
- Validation: `validate()`, `require_within_util()`, `require_valid_config()`
- Helper methods: Short descriptive names like `indices()`, `is_dominant()`, `settlement()`

**Variables:**
- Snake case: `long_notional`, `short_notional`, `base_fee`, `impact_fee`
- Constants: UPPER_SNAKE_CASE with explicit scalar suffixes when applicable
- Loop variables: Single letter `i` is acceptable (see `testutils.rs` line 76-77)
- Temporary results: Named results like `Settlement`, `ExecuteRequest`, `MarketData`

**Types & Structs:**
- PascalCase: `TradingConfig`, `MarketConfig`, `MarketData`, `Position`, `Settlement`
- Enum variants: PascalCase: `ExecuteRequestType::Fill`, `ContractStatus::Active`
- Error enum: `TradingError` with u32 discriminants (e.g., `Unauthorized = 1`)

## Code Style

**Formatting:**
- Rustfmt default configuration enforced
- Line length: No hard limit specified, but typical Rust practice (~100 chars)
- Indentation: 4 spaces (Rust standard)
- Blank lines: Used between logical sections (see `position.rs` line 8-9, 46-47)

**Linting:**
- Clippy warnings allowed with attributes where documented
- Example: `#[allow(clippy::too_many_arguments)]` in `position.rs` line 51 for position creation

## Import Organization

**Order:**
1. Crate-relative imports (same crate): `use crate::constants::{...}`
2. Soroban SDK: `use soroban_sdk::{...}`
3. Soroban utilities: `use soroban_fixed_point_math::SorobanFixedPoint`
4. Stellar utilities: `use stellar-access::{...}` (workspace deps)

**Path Aliases:**
- None explicitly used; full paths preferred for clarity
- Workspace dependencies imported directly from `soroban_sdk`, `stellar-*` crates

**Visibility:**
- Public modules: `pub mod constants`, `pub mod interface`
- Private modules: `mod dependencies`, `mod errors`
- Conditional public exports: `#[cfg(...)] pub use contract::*`
- Private-within-module: `pub(crate)` for internal utilities like `Market::load()`

## Error Handling

**Patterns:**
- Use `panic_with_error!(e, ErrorVariant)` macro from soroban_sdk
- Example: `panic_with_error!(e, TradingError::UtilizationExceeded);` (market.rs line 60)
- All errors defined as `#[contracterror]` enum with u32 discriminants
- Errors grouped by domain (Access, Configuration, Position, Price, etc.) with corresponding error codes
- No try-catch or Result types in contract code; panics stop execution
- Test assertions use Rust's `assert_eq!()`, `assert!()`, `assert_ne!()`

**Error Definition Location:** `trading/src/errors.rs`
- Access errors: 1xx
- Configuration: 7xx
- Market: 7xx
- Position: 7xx
- Status: 76x
- Circuit breaker/ADL: 78x
- Funding/Utilization: 79x+

## Fixed-Point Math Conventions

**CRITICAL: Scalar System**

Two primary scalars are used throughout:

- **SCALAR_7** (`10_000_000` = 10^7): Used for all rates, fees, ratios, weights, leverage, utilization
  - Example: `fee_rate` in SCALAR_7 precision
  - Example: `margin` (initial margin) in SCALAR_7 precision
  - Example: `r_borrow` (per-market weight) multiplier in SCALAR_7
  - Location: `trading/src/constants.rs` line 2

- **SCALAR_18** (`1_000_000_000_000_000_000` = 10^18): Used for interest indices, funding rates, borrowing rates
  - Example: `l_fund_idx`, `s_fund_idx`, `l_borr_idx`, `s_borr_idx` all SCALAR_18
  - Example: Position snapshots `fund_idx`, `borr_idx`, `adl_idx` all SCALAR_18
  - Location: `trading/src/constants.rs` line 3

**Price Scalar (Derived):**
- Computed at runtime: `price_scalar = 10^(-exponent)` from Pyth oracle
- NOT stored; recalculated on each price verification
- Example: BTC with exponent -8 → `price_scalar = 100_000_000`
- Used in fixed-point operations: `position.notional.fixed_div_floor(e, &position.entry_price, &price_scalar)`

**Token Scalar:**
- Only used for collateral bounds validation in `require_valid_config()`
- Derived from token decimals: `10^token_decimals`
- NOT used in fee/rate calculations (those use SCALAR_7 or SCALAR_18)

**Fixed-Point Operations:**
- All multiplication/division must use `SorobanFixedPoint` trait methods
- `fixed_mul_ceil()`: Rounding up — preferred for fee calculations
- `fixed_mul_floor()`: Rounding down — used for PnL calculations to be conservative
- `fixed_div_ceil()`: Rounding up — used for utilization checks
- `fixed_div_floor()`: Rounding down — used for entry-weight calculations
- Example (market.rs line 63): `let market_util = market_notional.fixed_div_ceil(e, &self.vault_balance, &SCALAR_7);`

**Naming Conventions for Fixed-Point Values:**
Suffixes indicate precision:
- No suffix: Token units (collateral, notional in token_decimals)
- `_scalar`: The divisor used in operations (e.g., `price_scalar`, `SCALAR_7`)
- `_idx`: Index/cumulative rate values (always SCALAR_18)
- `_rate`: Rate values (SCALAR_7 for weights, SCALAR_18 for interest rates)
- `_fee`: Fee amounts (token_decimals unless explicitly scaled)

**Key Constants (trading/src/constants.rs):**
- `SCALAR_7`: All fees, rates, weights, ratios
- `SCALAR_18`: All interest indices (funding, borrowing, ADL)
- `UTIL_ONICE`: 95% in SCALAR_7 (circuit breaker threshold)
- `UTIL_ACTIVE`: 90% in SCALAR_7 (hysteresis to prevent flapping)
- `ONE_HOUR_SECONDS`: 3600 (used in rate accrual conversions)

## Comments

**When to Comment:**
- Complex mathematical formulas: Always document the formula and variable meanings
- Example (rates.rs line 10-11): `Formula: baseRate × |L - S| / (L + S)`
- Boundary conditions: Document special cases and why they matter
- Example (rates.rs line 20-26): Match arms explaining empty positions, one-sided markets
- Non-obvious domain logic: E.g., why only the dominant side pays borrowing
- Example (market.rs line 173): `// Borrowing: dominant side only`

**Avoid Commenting:**
- Self-explanatory code logic
- Variable assignments that are clear from naming
- Standard Rust patterns

**Doc Comments (///)**
- Not extensively used in current codebase
- Function documentation exists but is sparse
- Example: `pub fn load(e: &Env, price_data: &PriceData) -> Self {` (no doc comment)

## Function Design

**Size:**
- Target: < 50 lines
- Examples that exceed: `Market::open()` (23 lines), `MarketData::accrue()` (58 lines)
- Large functions are justified by domain complexity (fee accrual, borrowing calculations)

**Parameters:**
- Functions use multiple parameters rather than structs (common in Soroban)
- Example: `validate(e, enabled, min_notional, max_notional, margin)` 5 parameters
- Where many related parameters exist, they're grouped (e.g., MarketConfig struct)
- Allowlist clippy too_many_arguments when needed (position.rs line 51)

**Return Values:**
- Single return type preferred: Direct return or named result struct
- Multiple values: Tuple returns like `(base_fee, impact_fee)` in `Market::open()`
- Result struct usage: `Settlement` struct for position close/settle operations
- Example (position.rs line 11-17): Settlement wraps pnl, fees (base, impact, funding, borrowing)

## Module Design

**Exports:**
- Main contract: `pub use contract::*` exposes public contract methods
- Types: `pub use interface::*` for client-facing types
- Errors: `pub use errors::TradingError`
- Testutils: Conditional: `#[cfg(any(test, feature = "testutils"))] pub mod testutils`
- Example (lib.rs lines 20-25): Structured re-exports for clarity

**Barrel Files:**
- `mod.rs` pattern not used in this codebase
- Instead: Direct module declarations in lib.rs with conditional compilation
- Nested modules: Declared inline (e.g., `pub mod trading` contains `market.rs`, `position.rs`, `rates.rs`)

**Visibility Hierarchy:**
- Library mode: Contracts have `feature = "library"` for cross-contract calls
- Test mode: All test utilities are gated behind `#[cfg(any(test, feature = "testutils"))]`
- Example (lib.rs line 13-14): Contract code conditionally compiled based on feature flags

## Attribute Patterns

**Conditional Compilation:**
- `#[cfg(any(not(feature = "library"), test, feature = "testutils"))]` for contract entry points
- `#[cfg(any(test, feature = "testutils"))]` for test-only code
- `#[cfg(test)]` for unit tests within modules

**Macros:**
- `#[contract]` for contract structs (Soroban contracts)
- `#[contractimpl]` for contract implementations
- `#[contracttype]` for serializable types (storage, parameters)
- `#[contracterror]` for error enums
- Example (testutils.rs line 33): `#[contract] pub struct MockPriceVerifier`

## Key Soroban-Specific Patterns

**Contract State Access:**
- `e.as_contract(&address, || { ... })` required to access contract storage from tests
- Example (market.rs line 266): Used in tests accessing storage to calculate indices
- This pattern is essential for verifying internal state in unit tests

**Ledger/Timestamp Access:**
- `e.ledger().timestamp()` for current block time
- Used in accrual calculations and MIN_OPEN_TIME validation
- Example (market.rs line 163): `let current_time = e.ledger().timestamp();`

**Authorization:**
- `e.mock_all_auths()` in tests to bypass authorization checks
- Real contracts enforced by Soroban host, not in contract code
- Example (test_fixture.rs line 46): Full auth mocking for tests

---

*Convention analysis: 2026-03-24*
