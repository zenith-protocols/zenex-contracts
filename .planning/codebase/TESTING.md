# Testing Patterns

**Analysis Date:** 2026-03-24

## Test Framework

**Runner:**
- Framework: Cargo test (built-in Rust)
- Version: 1.70+ (modern Rust edition 2021)
- Config: Standard Cargo.toml with `[dev-dependencies]` for test utilities
- No external test runner; uses Soroban SDK's `Env::default()` for contract testing

**Assertion Library:**
- Rust standard: `assert_eq!()`, `assert!()`, `assert_ne!()` macros
- Soroban testutils for synthetic environment: `soroban_sdk::testutils::{Address, Ledger, BytesN}`
- Custom assertions: `test-suites/src/assertions.rs` module (exists but specific patterns not examined)

**Run Commands:**
```bash
make test                   # Run all tests in workspace
cargo test --all --tests   # Verbose test execution
make coverage              # Line coverage via llvm-cov
make coverage-html         # HTML coverage report
```

## Test File Organization

**Location:**
- **Unit tests**: Co-located within source modules (pattern: `#[cfg(test)] mod tests { ... }`)
- **Integration tests**: Separate `tests/` directory in crates (e.g., `test-suites/tests/`)
- **Test fixtures**: Shared in `test-suites/src/` (test_fixture.rs, setup.rs)

**Naming:**
- Unit tests: `test_*.rs` functions within `#[cfg(test)] mod tests {}`
- Integration tests: `test_*.rs` files in `tests/` directory
- Examples: `test_trading_position.rs`, `test_trading_pnl.rs`, `test_trading_adl.rs`
- Helper functions: `setup_fixture()`, `open_long_position()`, `setup_factory()`

**Structure:**
```
trading/
├── src/
│   ├── lib.rs
│   ├── trading/
│   │   ├── market.rs        # Contains #[cfg(test)] mod tests { ... }
│   │   ├── rates.rs         # Contains #[cfg(test)] mod tests { ... }
│   │   └── position.rs
│   └── testutils.rs         # Conditional: #[cfg(any(test, feature = "testutils"))]
└── target/
    └── debug/deps/          # Test binaries

test-suites/
├── src/
│   ├── lib.rs
│   ├── test_fixture.rs      # TestFixture<'a> struct with helpers
│   ├── setup.rs             # create_fixture_with_data()
│   ├── assertions.rs        # Custom assertion helpers
│   └── token.rs
└── tests/
    ├── test_trading_position.rs
    ├── test_trading_pnl.rs
    ├── test_trading_adl.rs
    ├── test_trading_liquidations.rs
    ├── test_cost_profile.rs
    └── test_trading_proptest.rs
```

## Test Structure

**Suite Organization:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{SCALAR_7, SCALAR_18};
    use crate::testutils::{create_trading, default_market, jump, BTC_FEED_ID};
    use soroban_sdk::Env;

    #[test]
    fn test_some_behavior() {
        let e = Env::default();
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            // Test assertions inside contract context
        });
    }
}
```

**Key Pattern Elements:**
- Imports local to test module (avoid polluting module namespace)
- `Env::default()` creates a fresh, isolated blockchain simulator
- `e.as_contract(&address, || { ... })` wraps code that accesses contract storage
- Tests run in parallel by default; isolation is guaranteed by unique Env instances

**Setup Pattern:**

```rust
fn setup_fixture() -> TestFixture<'static> {
    create_fixture_with_data()
}

#[test]
fn test_example() {
    let fixture = setup_fixture();
    let user = Address::generate(&fixture.env);
    fixture.token.mint(&user, &(100_000 * SCALAR_7));
    // ... assertions
}
```

**Teardown Pattern:**
- Implicit (Soroban Env is dropped at test end)
- No explicit cleanup needed; state is isolated per test
- Example (test_fixture.rs): Each `TestFixture::create()` call produces a new isolated environment

## Mocking

**Framework:**
- Soroban native: `e.register()` to deploy mock contracts
- Pattern: Create `#[contract]` structs that implement contract interfaces

**Patterns:**

```rust
// Mock Price Verifier (testutils.rs)
#[contract]
pub struct MockPriceVerifier;

#[contractimpl]
impl MockPriceVerifier {
    pub fn set_price(e: Env, feed_id: u32, price: i128) {
        let mut prices: Map<u32, i128> = e.storage().instance().get(&MockPVKey::Prices)
            .unwrap_or(Map::new(&e));
        prices.set(feed_id, price);
        e.storage().instance().set(&MockPVKey::Prices, &prices);
    }

    pub fn verify_prices(e: Env, _price: Bytes) -> Vec<MockPriceData> {
        // Returns all stored prices, ignoring the input bytes
    }
}
```

**Mock Registration:**
```rust
let pv_id = e.register(MockPriceVerifier, ());
let pv_client = MockPriceVerifierClient::new(&e, &pv_id);
pv_client.set_price(&BTC_FEED_ID, &BTC_PRICE);
```

**What to Mock:**
- External oracle services (PriceVerifier)
- Treasury fee calculations (MockTreasury)
- Vault operations (in unit tests; in integration tests, use real vault)
- Ledger state: `e.mock_all_auths()`, `e.ledger().set_timestamp()`, `e.cost_estimate().budget().reset_unlimited()`

**What NOT to Mock:**
- Internal trading logic (test against real implementation)
- Position creation/settlement (test through actual contract)
- Storage operations (use real storage, accessed via `e.as_contract()`)
- Token operations (use `StellarAssetClient` from soroban_sdk testutils)

**Authorization Mocking:**
```rust
e.mock_all_auths();                           // Allow all auths
e.mock_all_auths_allowing_non_root_auth();    // Allow without signature validation
```

## Fixtures and Factories

**Test Data:**

```rust
pub struct TestFixture<'a> {
    pub env: Env,
    pub owner: Address,
    pub users: Vec<Address>,
    pub vault: VaultClient<'a>,
    pub trading: TradingClient<'a>,
    pub price_verifier: MockPriceVerifierClient<'a>,
    pub token: StellarAssetClient<'a>,
    pub factory: FactoryClient<'a>,
    pub treasury: Address,
}

impl TestFixture<'_> {
    pub fn create<'a>() -> TestFixture<'a> {
        let e = Env::default();
        e.cost_estimate().budget().reset_unlimited();
        e.mock_all_auths_allowing_non_root_auth();

        // Deploy all contracts, setup token, register mocks
        // Return fully initialized TestFixture
    }

    pub fn open_and_fill(
        &self,
        user: &Address,
        feed_id: u32,
        collateral: i128,
        notional_size: i128,
        is_long: bool,
        entry_price: i128,
        take_profit: i128,
        stop_loss: i128,
    ) -> u32 {
        self.price_verifier.set_price(&feed_id, &entry_price);
        self.trading.open_market(...)
    }
}
```

**Location:**
- `test-suites/src/test_fixture.rs`: Main TestFixture struct and helpers
- `test-suites/src/setup.rs`: `create_fixture_with_data()` helper for pre-seeding vault
- Shared across all integration tests via imports: `use test_suites::test_fixture::TestFixture`

**Factory Pattern Example (factory/src/test.rs):**
```rust
fn setup_factory(e: &Env) -> (Address, FactoryClient<'_>) {
    let trading_hash = e.deployer().upload_contract_wasm(TRADING_WASM);
    let vault_hash = e.deployer().upload_contract_wasm(VAULT_WASM);
    let treasury = Address::generate(e);
    let init_meta = FactoryInitMeta {
        trading_hash,
        vault_hash,
        treasury,
    };
    let address = e.register(FactoryContract {}, (init_meta,));
    let client = FactoryClient::new(e, &address);
    (address, client)
}
```

**Test Constants:**
```rust
pub const BTC_PRICE_RAW: i128 = 10_000_000_000_000; // $100,000 with exponent -8
pub const BTC_FEED_ID: u32 = 1;
pub const ETH_FEED_ID: u32 = 2;
pub const XLM_FEED_ID: u32 = 3;
pub const PRICE_SCALAR: i128 = 100_000_000;
pub const SCALAR_7: i128 = 10_000_000;
```

## Coverage

**Requirements:**
- Target: 80%+ line coverage (enforced via project instructions)
- Measurement: `cargo llvm-cov` with workspace-level tracking
- Exclusions: Testutils, test files themselves

**View Coverage:**
```bash
make coverage              # Terminal output
make coverage-html         # HTML report: target/llvm-cov/html/index.html
cargo llvm-cov report --workspace --exclude test-suites --ignore-filename-regex '(testutils|test\.rs|test_)'
```

**Coverage Strategy:**
- Core logic: Full unit test coverage (rates.rs tests each case exhaustively)
- Position lifecycle: Open, modify, close, liquidate paths all tested
- Accrual calculations: Funding, borrowing, ADL all tested
- Error cases: Validation, margin, utilization errors all tested

## Test Types

**Unit Tests:**
- Scope: Individual functions (e.g., `calc_funding_rate()`, `accrue()`, `settle()`)
- Approach: Minimal fixture; focus on math correctness
- Example (`rates.rs` lines 79-150): 8 tests for funding rate formula covering edge cases
  - No positions
  - Only longs, only shorts
  - Balanced positions
  - Imbalanced positions (2x dominant)
- Location: Within source modules (`#[cfg(test)] mod tests { ... }`)

**Integration Tests:**
- Scope: Full position lifecycle across trading + vault contracts
- Approach: TestFixture with deployed contracts, real trading logic
- Example (`test_trading_position.rs` lines 79-120): Open position → modify collateral → close at profit
- Verify: Balance changes, position state, fee calculations, payout amounts
- Location: `test-suites/tests/test_*.rs`

**E2E Tests:**
- Framework: Not explicitly separate; integration tests double as E2E
- Coverage: Critical user flows (long + short positions, liquidations, ADL)
- Example: `test_long_open_modify_close_profit()` is a full user flow test

**Fuzzing (Exists but Out of Sync):**
- Harnesses: `test-suites/fuzz/fuzz_targets/fuzz_*.rs`
- Status: Out of sync with current trading API (uses old oracle pattern)
- Location: `test-suites/fuzz/fuzz_targets/`

## Common Patterns

**Async Testing:**
- Not applicable (contracts are synchronous; blockchain time is mocked)
- Time advancement: `jump(&e, seconds)` helper function
- Example (`market.rs` line 342): `jump(&e, 3600); data.accrue(&e, ...);`

**Error Testing:**
- Pattern: Use `#[should_panic(expected = "...")]` for expected panics
- Example (`strategy-vault/src/test.rs` line 70): `#[should_panic(expected = "Error(Contract, #421)")]`
- Interpretation: Error codes map to contract error discriminants (e.g., SharesLocked = 421)
- Assertion: Verify panic message matches expected error code

**Parameterized Testing:**
- Rust approach: Create loop-based generators in helpers
- Example (test_trading_position.rs lines 19-43): `open_long_position()`, `open_short_position()`, etc.
- Usage: Call helpers with different inputs to test variations
- Alternative: Use `proptest` crate (partial integration: `test_trading_proptest.rs` exists)

**Cost Profiling:**
- Special test file: `test-suites/tests/test_cost_profile.rs`
- Purpose: Measure contract execution cost (budget usage)
- Uses `e.cost_estimate()` to track resource consumption

## Contract Storage Testing Pattern

**Accessing Storage in Tests:**

```rust
#[test]
fn test_market_data_load_and_store() {
    let e = Env::default();
    let (address, _) = create_trading(&e);

    e.as_contract(&address, || {
        // Inside this closure, storage operations are relative to 'address'
        storage::set_market_data(&e, BTC_FEED_ID, &data);
        let loaded = storage::get_market_data(&e, BTC_FEED_ID);
        assert_eq!(loaded.l_notional, 1000 * SCALAR_18);
    });
}
```

**Key Point:** Tests calculating internal state (interest indices, fees, PnL) must use `e.as_contract()` to access contract storage.

## Test Isolation & Parallel Execution

**Isolation:**
- Each test gets a fresh `Env::default()` (independent blockchain simulator)
- No shared state between tests
- Safe to run in parallel: `cargo test -- --test-threads=N`

**Dependencies:**
- Unit tests (e.g., market.rs) run independently
- Integration tests (test-suites) use shared TestFixture; each test calls `setup_fixture()` fresh
- No test-ordering requirements

## Notable Testing Gaps

**Out of Sync:**
- `test-suites/` has old integration tests using outdated oracle pattern
- `fuzz_liquidation.rs` and `fuzz_trading_general.rs` not fully integrated
- These require maintenance to align with current trading contract API

**Areas Needing Coverage:**
- Governance contract: Minimal/no test structure
- Treasury contract: Mock only, no real implementation tests
- Cross-contract error propagation: Limited coverage
- Extreme price scenarios: Some edge cases untested

---

*Testing analysis: 2026-03-24*
