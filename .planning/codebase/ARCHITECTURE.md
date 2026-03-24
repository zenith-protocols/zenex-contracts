# Architecture

**Analysis Date:** 2026-03-24

## Pattern Overview

**Overall:** Multi-contract perpetual futures protocol using factory deployment pattern with atomic vault+trading creation.

**Key Characteristics:**
- **Modular contracts**: Trading, vault, price verification, treasury, and governance contracts deployed via factory
- **Fixed-point math**: SCALAR_7 (1e7) for rates/fees/ratios, SCALAR_18 (1e18) for indices/funding
- **Cross-contract calls**: Trading calls vault (withdrawals), price verifier (verification), treasury (fee distribution)
- **On-chain state**: Per-market and per-position data persisted on Stellar ledger with automatic TTL management
- **Deterministic deployment**: Factory precomputes addresses to resolve circular dependencies between vault and trading

## Layers

**Entry Point Layer (Contract Implementations):**
- Purpose: Expose contract interfaces via `#[contractimpl]` trait implementations
- Location: `trading/src/contract.rs`, `factory/src/lib.rs`, `price-verifier/src/lib.rs`, `strategy-vault/src/contract.rs`, `treasury/src/lib.rs`, `governance/src/lib.rs`
- Contains: Trait implementations, authorization checks (via `#[only_owner]`, `require_auth()`), storage persistence calls
- Depends on: Business logic modules, storage layer, error enums
- Used by: Soroban SDK for contract dispatch; external callers via contract clients

**Business Logic Layer:**
- Purpose: Core trading operations, fee calculations, state transitions
- Location: `trading/src/trading/` — actions, execute, market, position, rates, adl, config
- Contains: Position creation/modification, fee calculations (base, impact, funding, borrowing), ADL logic, market state accrual
- Depends on: Storage layer, fixed-point math crate, types, constants
- Used by: Contract implementations; internal orchestration of multi-step operations

**Market & Position Layer:**
- Purpose: Domain models with embedded behavior for market and position operations
- Location: `trading/src/trading/market.rs`, `trading/src/trading/position.rs`
- Contains: `Market` struct (full context: config, data, vault info, prices), `Position` struct (snapshots at fill), settlement calculations
- Depends on: Types, storage, fixed-point math, vault/treasury clients
- Used by: Actions layer for open/close/modify operations

**Storage Layer:**
- Purpose: Ledger key management, persistence, TTL bumping with default values
- Location: `trading/src/storage.rs`, `strategy-vault/src/storage.rs`, `treasury/src/storage.rs`
- Contains: Instance storage (hot data: config, status), persistent storage (market configs, market data, positions, user positions), getter/setter functions with automatic TTL extension
- Depends on: Soroban SDK storage interface, types, constants
- Used by: All contract logic layers

**Dependencies/Integration Layer:**
- Purpose: External contract client interfaces and price data types
- Location: `trading/src/dependencies/` — vault.rs, price_verifier.rs, treasury.rs
- Contains: `VaultClient`, `PriceVerifierClient`, `TreasuryClient` with manually-defined traits to avoid type conflicts; `PriceData` struct from price verifier
- Depends on: Soroban SDK contractclient macro
- Used by: Trading contract for cross-contract calls

**Type System Layer:**
- Purpose: Domain types for configuration, market state, and positions
- Location: `trading/src/types.rs`
- Contains: `TradingConfig` (global trading parameters), `MarketConfig` (per-market parameters), `MarketData` (per-market mutable state with indices), `Position` (position snapshots), enums for status and request types
- Depends on: Soroban SDK contracttype macro
- Used by: All business logic and storage layers

**Validation/Error Layer:**
- Purpose: Input validation and error categorization
- Location: `trading/src/validation.rs`, `trading/src/errors.rs`
- Contains: Boundary checks for config values, status guards (require_active, require_can_manage), exhaustive error enum
- Depends on: Types, constants
- Used by: Contract implementations at entry points

**Event Layer:**
- Purpose: Off-chain event emission for indexing and monitoring
- Location: `trading/src/events.rs`, `factory/src/events.rs`
- Contains: Event structs (PlaceLimit, OpenMarket, ClosePosition, Liquidation, ApplyFunding, ADL-triggered, etc.) with `#[topic]` attributes
- Depends on: Soroban SDK contractevent macro, types
- Used by: Business logic layers after state mutations

## Data Flow

**Position Lifecycle (Market Order):**

1. **open_market** (user) → Trading contract entry point
2. **execute_create_market** → Load market context (price, vault balance, accrued indices)
3. **Market::open** → Calculate fees (base + impact), deduct from collateral, fill position, update market stats
4. **Token transfers** → User collateral to contract, vault fee to vault, treasury fee to treasury
5. **Events** → Emit OpenMarket event with final fees
6. Result: Position persisted with filled=true, snapshots of indices

**Position Settlement (Close):**

1. **close_position** (user) → With current price
2. **execute_close_position** → Load position, fetch market context at current price
3. **Position::settle** → Calculate PnL (based on entry vs exit), accrue funding/borrowing indices, compute all fee types, net out fees
4. **Market stats update** → Reduce notional, update entry weights
5. **Token transfers** → Return net (collateral + PnL - fees) to user
6. Result: Position marked as closed (removed from storage), events emitted with settlement breakdown

**Keeper-Triggered Execution (Fill/SL/TP/Liquidate):**

1. **execute** (caller) → Batch of ExecuteRequest with current price
2. **execute_trigger** → Loop through requests, call appropriate handler per request_type
3. **For Fill:** Market conditions checked, fees computed, position filled
4. **For StopLoss/TakeProfit:** Trigger price validation, then close at current price (same settlement as user close)
5. **For Liquidate:** Margin check fails, liquidation fee added to settlement fees
6. Result: Positions transitioned, keeper compensated from fees

**Funding/Borrowing Accrual:**

1. **apply_funding** (permissionless) → Hourly keeper call
2. **For each market:** Accrue borrowing on dominant side (utilization-based), accrue funding P2P (OI imbalance-based)
3. **MarketData::accrue** → Update l/s_borr_idx (borrowing) and fund_rate + l/s_fund_idx (funding)
4. **No position updates** → Indices stored in MarketData; position snapshots (fund_idx, borr_idx) used to calculate PnL delta later
5. Result: Global rate update, no individual position mutations

**ADL (Auto-Deleveraging) on Status Update:**

1. **update_status** (permissionless) → With batch of price feeds
2. **Compute net PnL** across all markets (loop once, cache per-market per-side PnL)
3. **Check thresholds:** Active→OnIce if net_pnl >= 95% of vault; OnIce→Active if net_pnl < 90%
4. **If triggering ADL:** Reduce winning-side notionals/entry_wts/adl_idx by factor (deficit/winner_pnl ratio), capped at full reduction
5. Result: Circuit breaker activated, status set to OnIce, losing side protected

**State Management (Indices):**

- **Global indices:** l/s_fund_idx, l/s_borr_idx, l/s_adl_idx stored in MarketData
- **Position snapshots:** Position stores fund_idx, borr_idx, adl_idx at fill time
- **Settlement calculation:** Accrue market indices to current time, multiply position notional by (current_idx / position_idx) to get accrued fee
- **No position updates on funding:** Position snapshots immutable; only MarketData indices advance

## Key Abstractions

**Market Context (Market Struct):**
- Purpose: Bundle per-market + global state needed for any operation
- Examples: `trading/src/trading/market.rs` Market struct
- Pattern: Load from storage (with auto-accrue), pass by mutable reference, store back after mutation

**Position Settlement (Settlement Struct):**
- Purpose: Breakdown of PnL and all fee types
- Examples: `trading/src/trading/position.rs` Settlement struct
- Pattern: Compute per position/market action, return (pnl, base_fee, impact_fee, funding, borrowing)
- Methods: equity (net worth), total_fee, protocol_fee, trading_fee

**Factory Deployment Pattern:**
- Purpose: Resolve circular dependency: vault needs trading address, trading needs vault address
- Pattern: Precompute both addresses → deploy vault first (no cross-call) → deploy trading (calls vault)
- Location: `factory/src/lib.rs` compute_salts + deploy_v2 flow

**PriceData from Verifier:**
- Purpose: Pyth Lazer price with exponent, used to derive scalar
- Pattern: Verify → extract (feed_id, price, exponent, publish_time) → compute price_scalar = 10^(-exponent)
- Usage: Trading never stores exponent; recalculates scalar on every price use

**Client Pattern for Cross-Contract Calls:**
- Purpose: Type-safe interface to external contracts
- Examples: VaultClient::new(e, &vault).total_assets(), PriceVerifierClient::new(e, &verifier).verify_prices(...)
- Pattern: Manually-defined traits to avoid type conflicts (OpenZeppelin tokens); generated via `#[contractclient]` macro

## Entry Points

**Trading Contract:**
- Location: `trading/src/contract.rs`
- Constructor: `__constructor(owner, token, vault, price_verifier, treasury, config)` — validates config, stores addresses
- Public interface (Trading trait):
  - User actions: place_limit, open_market, cancel_limit, close_position, modify_collateral, set_triggers
  - Keeper actions: execute (batch), apply_funding
  - Admin actions: set_config, set_market, del_market, set_status
  - Getters: get_position, get_markets, get_config, etc.

**Factory Contract:**
- Location: `factory/src/lib.rs`
- Constructor: `__constructor(init_meta)` — stores compiled WASM hashes + treasury address
- Entry point: `deploy(admin, salt, token, price_verifier, config, vault_name, vault_symbol, ...)` — returns trading address
- Pattern: Deterministic salts prevent frontrun, precomputed addresses resolve circular deps

**Price Verifier Contract:**
- Location: `price-verifier/src/lib.rs`
- Constructor: `__constructor(owner, trusted_signer, max_confidence_bps, max_staleness)`
- Entry points: verify_price (single), verify_prices (batch) — return PriceData
- Admin: update_trusted_signer, update_max_confidence_bps, update_max_staleness

**Strategy Vault Contract:**
- Location: `strategy-vault/src/contract.rs`
- Constructor: `__constructor(name, symbol, asset, decimals_offset, strategy, lock_time)`
- ERC-4626 compliant: deposit, mint, withdraw, redeem (with lock enforcement)
- Admin: strategy_withdraw (trading contract only)

**Treasury Contract:**
- Location: `treasury/src/lib.rs`
- Constructor: `__constructor(owner, rate)` — rate is SCALAR_7 fraction of fees
- Entry points: get_rate, get_fee (rate × revenue), set_rate (admin), withdraw (admin)

**Governance Contract:**
- Location: `governance/src/lib.rs`
- Constructor: `__constructor(owner, trading, delay)` — delay in seconds before queued updates unlock
- Admin queue: queue_set_config, queue_set_market, set_status (immediate)
- Permissionless execute: set_config, set_market (after delay passes)
- Pattern: Timelock prevents sudden parameter changes

## Error Handling

**Strategy:** Panic with error enums; no recovery or fallback logic.

**Patterns:**
- **Validation errors** (TradingError 702+): Config, market, position validation failures → caught in contract entry points
- **Access control errors** (TradingError 1): Unauthorized actions → via `require_auth()` or `#[only_owner]`
- **Position errors** (TradingError 730+): Position not found, not liquidatable, leverage too high, etc.
- **Status errors** (TradingError 750+): Contract OnIce/Frozen prevents new positions
- **ADL threshold errors** (TradingError 780): Not enough PnL to trigger ADL (returned by update_status when threshold not met)
- **No error recovery:** Failures are terminal; next call must succeed or user must try different parameters

## Cross-Cutting Concerns

**Logging:** None configured; events are primary audit trail.

**Validation:**
- Input bounds checked at entry points via `require_valid_config`, `require_valid_market_config`
- Position validation: notional in [min, max], leverage <= 1/margin, TP > entry (longs) or TP < entry (shorts)
- Status guards: require_active (opens only), require_can_manage (admin actions)

**Authentication:**
- User actions require `user.require_auth()` via Stellar's signature verification
- Admin actions use `#[only_owner]` macro from stellar-access
- Treasury/vault withdrawals check caller identity

**Rent/TTL Management:**
- Instance storage (hot data) bumped on every tx (30-day threshold, 31-day bump)
- Market persistent storage (configs, data) 45-day threshold, 52-day bump
- Position persistent storage (short-lived) 14-day threshold, 21-day bump
- ADL/governance temp storage 100-day threshold, 120-day bump
- Automatic extension on access prevents expiry for active contracts/markets

---

*Architecture analysis: 2026-03-24*
