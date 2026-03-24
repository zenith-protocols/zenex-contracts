# Codebase Structure

**Analysis Date:** 2026-03-24

## Directory Layout

```
zenex-contracts/
├── Cargo.toml              # Workspace root, members list, shared dependencies
├── Cargo.lock              # Locked dependency versions
├── Makefile                # Build/deploy targets
├── trading/                # Core perpetual futures trading contract
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs                    # Public exports (interface, types, testutils)
│   │   ├── contract.rs               # #[contract] impl + Trading trait impl
│   │   ├── interface.rs              # Trading trait definition
│   │   ├── types.rs                  # TradingConfig, MarketConfig, Position, MarketData, enums
│   │   ├── constants.rs              # SCALAR_7, SCALAR_18, fees/rate caps, time constants
│   │   ├── errors.rs                 # TradingError enum
│   │   ├── events.rs                 # All event structs (PlaceLimit, OpenMarket, Close, ADL, etc.)
│   │   ├── storage.rs                # Ledger keys, getters/setters, TTL management
│   │   ├── validation.rs             # require_valid_config, require_active, status checks
│   │   ├── testutils.rs              # Test fixtures (cfg: testutils feature only)
│   │   ├── dependencies/
│   │   │   ├── mod.rs                # Re-exports (PriceData, clients)
│   │   │   ├── vault.rs              # VaultClient trait (query_asset, total_assets, strategy_withdraw)
│   │   │   ├── treasury.rs           # TreasuryClient trait (get_rate, get_fee)
│   │   │   └── price_verifier.rs     # PriceVerifierClient, PriceData, scalar_from_exponent
│   │   └── trading/
│   │       ├── mod.rs                # Re-exports (execute_* functions)
│   │       ├── actions.rs            # execute_create_limit, execute_cancel_limit, execute_create_market, execute_close_position, execute_modify_collateral, execute_set_triggers
│   │       ├── execute.rs            # execute_trigger (keeper batch executor)
│   │       ├── market.rs             # Market context struct with open/close logic
│   │       ├── position.rs           # Position struct with create/fill/settle methods, Settlement struct
│   │       ├── rates.rs              # Funding/borrowing rate calculations
│   │       ├── adl.rs                # execute_update_status (ADL + circuit breaker)
│   │       └── config.rs             # execute_set_config, execute_set_market, execute_del_market, execute_set_status
│   └── target/wasm32-v1-none/release/  # Compiled WASM (ignored, built via cargo)
├── strategy-vault/         # ERC-4626 compliant vault with deposit lock
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs          # Re-exports
│   │   ├── contract.rs     # StrategyVaultContract: FungibleToken + FungibleVault impls, lock enforcement
│   │   ├── storage.rs      # Lock time, strategy address, last deposit timestamps
│   │   ├── strategy.rs     # StrategyVault helper methods (require_unlocked, withdraw)
│   │   └── test.rs         # Integration tests
├── factory/                # Atomic vault + trading deployment
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs          # Module re-exports, FactoryContract struct
│   │   ├── events.rs       # Deploy event
│   │   ├── storage.rs      # Compiled hashes + treasury for deployed contracts
│   │   └── test.rs         # Deployment tests
├── price-verifier/         # Pyth Lazer price verification
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs          # PriceVerifier contract, __constructor, verify_price/prices
│   │   ├── error.rs        # PriceVerifierError (unused, left for future)
│   │   ├── pyth.rs         # verify_and_extract (Pyth signature verification), check_staleness
│   │   ├── storage.rs      # Signer, max_confidence_bps, max_staleness
│   │   └── test.rs         # Verification tests
├── treasury/               # Protocol fee collection (simple rate × revenue)
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs          # TreasuryContract: get_rate, get_fee, set_rate, withdraw
│   │   ├── storage.rs      # Rate storage
│   │   └── test.rs
├── governance/             # Timelock for trading config updates
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs          # GovernanceContract: queue/execute config + market updates, set_status
│   │   └── ... (no separate module files)
├── test-suites/            # High-level integration tests (OUT OF SYNC)
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── dependencies/   # Mock trading client
│   │   └── ... (old oracle pattern, not actively maintained)
├── security/               # Legacy security analysis files (not code)
├── security-v2/            # Updated security analysis files (not code)
└── .planning/              # Orchestration docs (this is created by GSD, not part of build)
    └── codebase/           # Architecture/structure analysis docs
```

## Directory Purposes

**trading/src:**
- Purpose: Core perpetual futures trading logic, entry point for all position lifecycle operations
- Contains: Position management, market state, fee calculations, keeper automation, ADL circuit breaker
- Key files: `contract.rs` (entry), `trading/market.rs` + `trading/position.rs` (domain models)

**strategy-vault/src:**
- Purpose: Asset vault for protocol deposits, ERC-4626 compliant with lock mechanism
- Contains: Deposit/withdraw/redeem with automatic share price, timelock preventing instant withdrawals
- Key files: `contract.rs` (vault trait impls), `storage.rs` (lock tracking)

**factory/src:**
- Purpose: Deterministic factory for deploying vault+trading pairs atomically
- Contains: Contract address precomputation, WASM hash storage, salt derivation
- Key files: `lib.rs` (deploy logic)

**price-verifier/src:**
- Purpose: Pyth Lazer price feed verification and freshness checking
- Contains: Signature verification, confidence/staleness filtering
- Key files: `pyth.rs` (cryptographic verification)

**treasury/src:**
- Purpose: Protocol fee accumulation and distribution control
- Contains: Simple fee rate (SCALAR_7) application
- Key files: `lib.rs` (single implementation)

**governance/src:**
- Purpose: Timelock upgrade mechanism for trading parameters
- Contains: Queued config updates with unlock delay
- Key files: `lib.rs` (queue/execute logic)

**test-suites/src:**
- Purpose: Integration test harness (NOT actively maintained — uses old oracle pattern)
- Contains: Deprecated test utilities
- Status: Out of sync with current trading API

## Key File Locations

**Entry Points:**
- `trading/src/contract.rs`: TradingContract + Trading trait impl — primary contract interface
- `factory/src/lib.rs`: FactoryContract deploy function — deployment orchestration
- `price-verifier/src/lib.rs`: PriceVerifier verify_price/prices — price feed entry
- `strategy-vault/src/contract.rs`: StrategyVaultContract + FungibleToken/FungibleVault impls — vault entry
- `treasury/src/lib.rs`: TreasuryContract + Treasury trait — fee distribution
- `governance/src/lib.rs`: GovernanceContract + Governance trait — admin timelock

**Configuration:**
- `trading/src/types.rs`: TradingConfig (global), MarketConfig (per-market) — all parameter types
- `trading/src/constants.rs`: SCALAR_7, SCALAR_18, fees/rate caps, time bounds
- `factory/src/storage.rs`: WASM hashes for vault + trading (set at factory construction)

**Core Logic:**
- `trading/src/trading/market.rs`: Market struct, open/close/modify operations
- `trading/src/trading/position.rs`: Position struct, create/fill/settle, Settlement struct
- `trading/src/trading/rates.rs`: Funding rate, borrowing rate calculations
- `trading/src/trading/adl.rs`: ADL logic, circuit breaker (95%/90% thresholds)

**Testing:**
- `trading/src/testutils.rs`: Test helpers (gated by testutils feature)
- `strategy-vault/src/test.rs`: Vault integration tests
- `factory/src/test.rs`: Deployment tests
- `price-verifier/src/test.rs`: Price verification tests

## Naming Conventions

**Files:**
- Contract entry: `contract.rs`
- Trait definition: `interface.rs` (trading only; others inline in contract.rs)
- Domain logic by feature: `{feature_name}.rs` (market.rs, position.rs, adl.rs, etc.)
- Cross-contract clients: `dependencies/{service_name}.rs` (vault.rs, treasury.rs, price_verifier.rs)
- Tests: `test.rs` or integration tests in separate crate (test-suites)

**Directories:**
- Crate module: lowercase, underscore-separated (trading, strategy-vault, price-verifier)
- Submodules: named by concern (trading/, dependencies/)

**Functions:**
- Contract entry: no prefix (open_market, close_position, place_limit, apply_funding)
- Internal execute: execute_{action} (execute_create_market, execute_close_position, execute_trigger)
- Storage helpers: get_{entity}/{set_entity}/{extend_instance}
- Validation: require_{condition} (require_active, require_can_manage)
- Computation: {verb}_{noun} or noun only (accrue, settle, open, close)

**Types:**
- Config structs: {Entity}Config (TradingConfig, MarketConfig)
- Data structs: {Entity}Data (MarketData)
- Events: PascalCase action name (PlaceLimit, OpenMarket, ClosePosition)
- Error enum: {Entity}Error (TradingError, TreasuryError, AdminError)
- Storage keys: enum TradingStorageKey { VariantName }

## Where to Add New Code

**New Position Action (e.g., increase leverage):**
- Add function to `trading/src/trading/actions.rs`: `execute_increase_leverage(...) -> Result`
- Add to Trading trait in `trading/src/interface.rs`
- Add entry point in `trading/src/contract.rs` #[contractimpl] impl
- Add event struct to `trading/src/events.rs`
- Add validation to `trading/src/validation.rs` if needed
- Tests: Add test cases to existing test files in crates

**New Market Feature (e.g., skew-based funding):**
- Add calculation to `trading/src/trading/rates.rs`
- Add fields to `trading/src/types.rs` MarketConfig if new params needed
- Update `trading/src/trading/market.rs` Market::load() or accrue() if state changes
- Update validation in `trading/src/validation.rs` for new bounds
- Add event emission in relevant action functions

**New Cross-Contract Integration (e.g., oracle wrapper):**
- Add client trait to `trading/src/dependencies/{service_name}.rs`
- Generate client with `#[contractclient(name = "Client")]`
- Add getter in `trading/src/contract.rs` if needed for caller verification
- Use client like: `{Service}Client::new(e, &address).method(args)`

**Utilities and Helpers:**
- Shared math: Add to `trading/src/trading/rates.rs` or new file
- Shared validation: Add to `trading/src/validation.rs`
- Shared storage: Add function to relevant `storage.rs` (e.g., trading/src/storage.rs)
- Test utilities: Add to `trading/src/testutils.rs` (gated by `#[cfg(any(test, feature = "testutils"))]`)

## Special Directories

**target/:**
- Purpose: Build artifacts
- Generated: Yes (cargo output)
- Committed: No (.gitignore)

**.planning/codebase/:**
- Purpose: Architecture/structure documentation (generated by GSD)
- Generated: Yes (by orchestrator)
- Committed: Yes (part of repo state for CI/CD tools)

**security/, security-v2/:**
- Purpose: Legacy/updated security audit files
- Generated: No (manually created)
- Committed: Yes
- Status: Not integrated with build system

---

## Workspace Configuration

**Root Cargo.toml (workspace):**
- Members: price-verifier, strategy-vault, test-suites, trading, governance, treasury, factory
- Shared dependencies: soroban-sdk (25.3.0), soroban-fixed-point-math (1.5.0), stellar-* from OpenZeppelin
- Release profile: opt-level = "z" (size optimization), overflow-checks enabled, LTO, 1 codegen unit

**Per-Crate Features:**
- `library`: Controls whether contract is compiled as library (feature = "library" disables contract impl)
- `testutils`: Enables Soroban testutils + custom test helpers, gated in code

**Build Targets:**
- Primary: `cargo build --target wasm32-unknown-unknown --release` for WASM compilation
- Test: `cargo test` for unit tests
- Note: Factory tests require compiled WASM artifacts first (build dependency)

---

*Structure analysis: 2026-03-24*
