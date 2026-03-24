# External Integrations

**Analysis Date:** 2025-03-24

## Oracle & Price Feeds

**Pyth Lazer (Primary Oracle):**
- Service: Real-time oracle pricing from Pyth Network
- Integration: `price-verifier` contract (`/home/robin/Zenith/Zenex/zenex-contracts/price-verifier/src/lib.rs`)
- Implementation:
  - Verifies signed price updates via Ed25519 signature (`env.crypto().ed25519_verify()`)
  - Extracts price feeds from binary-encoded Pyth Lazer format (Solana-compatible wire format)
  - Validates staleness: prices rejected if older than `max_staleness` (configurable)
  - Validates confidence interval: prices must meet `max_confidence_bps` threshold
- Contract Interface:
  - `verify_price(env, update_data: Bytes) -> PriceData` - Single price feed
  - `verify_prices(env, update_data: Bytes) -> Vec<PriceData>` - Multiple price feeds
- Configuration:
  - `trusted_signer` - Ed25519 public key (32 bytes) for signature verification
  - `max_confidence_bps` - Maximum basis points of acceptable confidence
  - `max_staleness` - Maximum age in seconds (owner configurable)
- Data Format: Returns `PriceData { feed_id: u32, price: i128, exponent: i32, publish_time: u64 }`
- Price Scalar: Derived from exponent: `10^(-exponent)` (e.g., exponent -8 → scalar 10^8)
- Location: `price-verifier/src/pyth.rs` - Contains magic bytes, payload parsing, staleness checks

## Cross-Contract Dependencies

**Trading ↔ Vault Integration:**
- Location: `trading/src/dependencies/vault.rs`
- Client: `VaultClient` auto-generated from `VaultInterface` trait
- Methods:
  - `query_asset()` - Get underlying token address
  - `total_assets()` - Query vault TVL
  - `strategy_withdraw(strategy: Address, amount: i128)` - Withdraw collateral (strategy-gated)
- Purpose: Strategy (trading contract) manages vault liquidity; vault holds collateral in ERC-4626 format
- Deployment: Factory atomically deploys both via `deploy_v2()` with precomputed addresses

**Trading ↔ Price Verifier Integration:**
- Location: `trading/src/dependencies/price_verifier.rs`
- Client: `PriceVerifierClient` from `price-verifier` contract interface
- Methods:
  - `verify_prices(env, update_data: Bytes) -> Vec<PriceData>`
- Usage: Trading contract requires signed price data for market orders, position closes, and updates
- Data Flow:
  1. Keeper submits price update (Pyth Lazer format) to trading contract
  2. Trading calls `price_verifier.verify_prices()` with signed blob
  3. Price verifier validates signature and staleness
  4. Returns `PriceData` tuple with price, exponent, publish_time
  5. Trading derives `price_scalar = 10^(-exponent)` for fixed-point math

**Trading ↔ Treasury Integration:**
- Location: `trading/src/dependencies/treasury.rs`
- Client: `TreasuryClient` from treasury contract interface
- Methods:
  - `get_rate(env) -> i128` - Fetch protocol fee rate (SCALAR_7)
  - `get_fee(env, total_fee: i128) -> i128` - Calculate protocol fee split
- Purpose: Calculate protocol fee portion from trading fees
- Configuration: Treasury rate updatable only by owner via governance

**Trading ↔ Factory Integration:**
- Location: `factory/src/lib.rs`
- Client: Implicit via constructor args and `deploy_v2()`
- Methods:
  - `deploy()` - Deploy trading + vault atomically
  - `is_deployed()` - Verify contract legitimacy
- Address Precomputation:
  - Factory uses `e.deployer().deployed_address()` to precompute vault and trading addresses
  - Enables circular dependency resolution: vault stores trading address, trading stores vault address
  - Deterministic salts prevent front-running: `compute_salts(admin, user_salt)`

**Governance ↔ Trading Integration:**
- Location: `governance/src/lib.rs`
- Client: `TradingClient` for calling trading contract methods
- Methods:
  - `set_config()` - Update global trading parameters (with timelock)
  - `set_market()` - Add/update market configuration (with timelock)
  - `set_status()` - Update trading status (halts/resumes trading)
- Timelock Mechanism:
  - Governance queues config changes with `unlock_time`
  - Changes must be locked for `LEDGER_THRESHOLD_TEMP` (100 days of ledgers) before execution
  - Prevents flash-loan governance attacks

## Token Integration

**Stellar Native Asset Tokens:**
- Integration: `soroban-sdk::token::TokenClient` (native Soroban token interface)
- Usage in Trading:
  - Collateral token for positions (specified at trading contract construction)
  - Retrieved via `get_token()` getter
- Usage in Treasury:
  - Protocol fee withdrawal: `token.transfer(treasury, recipient, amount)`
  - Location: `treasury/src/lib.rs` line 77
- Usage in Vault:
  - Underlying asset for ERC-4626 vault
  - Via `stellar-tokens` FungibleVault trait implementation
  - Collateral deposits/withdrawals tracked via vault shares

**Vault Share Tokens (ERC-4626):**
- Standard: OpenZeppelin FungibleToken + FungibleVault
- Purpose: Represents LP deposits with locking mechanism
- Decimal Offset: Supports inflation attack protection via configurable `decimals_offset` (0-10)
- Lock Time: Configurable per vault (defaults set at factory deployment)
- Location: `strategy-vault/src/contract.rs`

## Stellar Blockchain Integration

**Network:**
- Network: Stellar testnet (`https://soroban-testnet.stellar.org`)
- Contract Interaction: All contracts inherit from Soroban SDK
- Ledger Access:
  - Timestamp for staleness checks: `env.ledger().timestamp()`
  - Sequence number for state expiration: `env.storage().instance().extend_ttl()`

**Authentication & Authorization:**
- Admin Operations:
  - `#[only_owner]` macro validates owner via Soroban `require_auth()`
  - Owner stored via OpenZeppelin `ownable::set_owner()`, retrieved via `ownable::is_owner()`
  - Price-verifier: owner can update signer, confidence, staleness thresholds
  - Treasury: owner can set fee rate and withdraw fees
  - Governance: owner can queue config updates

- Strategy Authorization:
  - Strategy (trading contract) address stored in vault at construction
  - Only strategy can call `strategy_withdraw()`
  - Checked via `strategy.require_auth()`

- Depositor Locking:
  - Vault tracks `last_deposit_time[user]` in contract storage
  - Transfers/withdrawals blocked until `now >= last_deposit_time + lock_time`
  - Location: `strategy-vault/src/storage.rs` and `strategy.rs`

## Webhook & Event Integration

**Factory Events:**
- Event: `Deploy { trading: Address, vault: Address }`
- Purpose: External indexers track new pool deployments
- Emission: Published at `factory/src/lib.rs` line 94 after successful deployment

**Keeper Actions (Implicit):**
- Method: `trading.execute(caller, requests: Vec<ExecuteRequest>, price: Bytes)`
- Keepers submit batches of: fill limit orders, trigger stop-loss/take-profit, liquidate positions
- All backed by signed price data from price-verifier
- Location: `trading/src/interface.rs` line 73

**Funding Rate Application:**
- Method: `trading.apply_funding()`
- Permissionless hourly funding rate accrual
- Updates market funding indices and transfers fees between longs/shorts
- No price data required (uses OI imbalance)

## Configuration & State Files

**Deployment Configuration:**
- File: `zenex-utils/deploy.json` (referenced in CLAUDE.md)
- Contains: Contract constructor args, fee rates, leverage limits, market parameters
- **CRITICAL:** Must be reviewed before every deployment
- Not committed to this repository (located in parent zenex-utils module)

**Development State:**
- File: `.dev-env.toml` (in parent Zenex directory)
- Contains: Deployed contract addresses, Mercury webhook state, tunnel configuration
- Secrets: API keys, webhook tokens (never committed)
- Usage: Source of truth during development; synced to service `.dev.vars` files

**Test Data & Fixtures:**
- Pyth Test Oracle: 8 decimal exponent → 10^8 price scalar
- BTC Price: `10_000_000_000_000` (representing $100k at 8 decimals)
- Token: 7 decimal places (configured in testing)
- Location: `test-suites/src/lib.rs` - shared test utilities and factories

## External System Dependencies

**Mercury Webhook (Indexing):**
- Service: Mercury on-chain event listener
- Integration: Zenex indexer receives webhooks with contract event logs
- State: Webhook URL stored in `.dev-env.toml`, must be recreated after tunnel restarts
- Purpose: Indexes position state into D1 database for query API

**Cloudflared Tunnel (Development):**
- Purpose: Exposes local backend to Mercury for webhook callbacks
- Configuration: Tunnel URL changes on restart
- Issue: Requires manual Mercury webhook recreation after restart
- Location: Managed via `make dev` in parent repository

**D1 Database (Positions Index):**
- Service: Cloudflare D1 (SQLite)
- Shared Location: `.dev-state/` directory
- Used by:
  - Backend: API queries (transactions, positions, leaderboard)
  - Indexer: Stores parsed contract events
- Persistence: Local file-based during development

**OpenZeppelin Relayer:**
- URL: `https://relayer.zenithprotocols.com`
- Purpose: Relays keeper transactions (transaction batching)
- Used by: zenex-keeper bot for liquidation operations

---

*Integration audit: 2025-03-24*
