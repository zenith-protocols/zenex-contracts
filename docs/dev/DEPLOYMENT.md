# Factory Deployment Flow

## Overview

The Zenex Factory contract atomically deploys a paired Trading + Strategy Vault system. The core challenge is resolving a circular dependency: the Trading contract needs the Vault address (for withdrawals), and the Vault needs the Trading address (as its authorized strategy). The Factory solves this by precomputing both addresses deterministically before deploying either contract.

## Prerequisites

1. **Compile contracts to WASM:**
   ```bash
   stellar contract build --optimize
   ```
   This produces optimized WASM binaries for all contracts.

2. **Upload WASM hashes to the network:**
   The `trading_hash` and `vault_hash` must be uploaded to the Stellar network before factory deployment.

3. **Deploy Treasury contract** -- the factory references a shared Treasury address.

4. **Deploy Price Verifier contract** -- the factory receives this as a parameter per pool.

## Factory Initialization

The Factory contract is constructed with a `FactoryInitMeta` struct:

```rust
pub struct FactoryInitMeta {
    pub trading_hash: BytesN<32>,  // WASM hash of the trading contract
    pub vault_hash: BytesN<32>,    // WASM hash of the vault contract
    pub treasury: Address,          // Shared treasury address
}
```

Source: `factory/src/storage.rs`

```rust
pub fn __constructor(e: Env, init_meta: FactoryInitMeta) {
    storage::set_init_meta(&e, &init_meta);
}
```

## Deploy Function

```rust
fn deploy(
    e: Env,
    admin: Address,           // Pool admin (becomes trading contract owner)
    salt: BytesN<32>,         // User-provided salt for deterministic addresses
    token: Address,           // Collateral token (e.g., USDC)
    price_verifier: Address,  // Price oracle contract
    config: TradingConfig,    // Initial trading configuration
    vault_name: String,       // ERC-4626 share token name
    vault_symbol: String,     // ERC-4626 share token symbol
    vault_decimals_offset: u32, // Inflation attack mitigation (0-10)
    vault_lock_time: u64,     // LP deposit lock duration in seconds
) -> Address;
```

Source: `factory/src/lib.rs` -- `Factory::deploy()`

## Internal Deployment Steps

### Step 1: Compute Deterministic Salts

```rust
fn compute_salts(e: &Env, admin: &Address, salt: &BytesN<32>) -> (BytesN<32>, BytesN<32>) {
    // trading_salt = keccak256(salt || admin_bytes || 0x00)
    // vault_salt   = keccak256(salt || admin_bytes || 0x01)
}
```

The admin address is mixed into the salt to prevent frontrunning: another user cannot observe a pending deploy transaction and submit their own with the same salt to claim the addresses.

Source: `factory/src/lib.rs` -- `compute_salts()`, lines 104-118.

### Step 2: Precompute Addresses

```rust
let trading_deployer = e.deployer().with_current_contract(trading_salt);
let vault_deployer = e.deployer().with_current_contract(vault_salt);
let trading_address = trading_deployer.deployed_address();
let vault_address = vault_deployer.deployed_address();
```

Both addresses are deterministically derived from the factory address + salt. This is the key to resolving the circular dependency: we know both addresses before either contract exists.

### Step 3: Deploy Vault First

```rust
vault_deployer.deploy_v2(
    init_meta.vault_hash,
    (vault_name, vault_symbol, token.clone(), vault_decimals_offset,
     trading_address.clone(), vault_lock_time),
);
```

The Vault constructor receives `trading_address` as its `strategy` parameter. At this point, the trading contract does not exist yet, but the Vault only stores the address -- it does not make any cross-contract calls during construction.

Vault constructor signature:
```rust
fn __constructor(
    name: String,
    symbol: String,
    asset: Address,
    decimals_offset: u32,
    strategy: Address,    // <-- precomputed trading address
    lock_time: u64,
)
```

### Step 4: Deploy Trading Second

```rust
trading_deployer.deploy_v2(
    init_meta.trading_hash,
    (admin.clone(), token, vault_address.clone(), price_verifier,
     init_meta.treasury, config),
);
```

The Trading constructor receives `vault_address` and stores it. At this point, the Vault is already deployed and live, so any cross-contract calls the Trading contract might make would succeed.

Trading constructor signature:
```rust
fn __constructor(
    owner: Address,
    token: Address,
    vault: Address,         // <-- precomputed vault address
    price_verifier: Address,
    treasury: Address,
    config: TradingConfig,
)
```

### Step 5: Record Deployment

```rust
storage::set_deployed(&e, &trading_address);
```

The factory tracks which trading contracts it deployed for verification via `is_deployed()`.

### Result

After deployment:
- `trading.get_vault() == vault_address` -- trading knows its vault
- `vault.strategy == trading_address` -- vault authorizes trading for `strategy_withdraw()`
- Both contracts reference the same collateral token
- The trading contract is owned by the provided `admin`

## Circular Dependency Resolution

```
                    needs vault address
        Trading  <------------------------  Factory
           |                                   |
           | needs trading address              |
           v                                   v
        Vault    <------------------------  deployed_address()
                   precomputes address
                   before deployment
```

Without precomputed addresses, you would need:
1. Deploy Vault without a strategy
2. Deploy Trading with the vault address
3. Call Vault.set_strategy(trading_address)

This three-step approach has a window where the vault has no authorized strategy or the wrong strategy, creating a security gap. The factory's atomic deployment eliminates this window.

## Salt-Based Deployment

The salt mechanism provides two properties:

1. **Deterministic addressing** -- given the same factory, admin, and salt, the resulting addresses are always the same. This enables off-chain systems to predict addresses before deployment.

2. **Front-run protection** -- the admin address is mixed into the salt derivation. An attacker observing a pending deployment transaction cannot submit their own with the same salt because the resulting keccak256 hash incorporates the admin's address.

## decimals_offset Parameter

The `vault_decimals_offset` parameter (range 0-10) implements the OpenZeppelin virtual shares pattern for inflation attack mitigation.

Without a decimals offset, an attacker could:
1. Deposit 1 wei of collateral to get 1 share
2. Donate a large amount directly to the vault (increasing total_assets)
3. Now 1 share = 1 wei + donation
4. Future depositors lose precision because their deposit maps to < 1 share

With `decimals_offset = N`, the vault internally uses `10^N` virtual shares per actual share, making this attack economically infeasible.

See threat model entry T-ELEV-11 for full analysis.
