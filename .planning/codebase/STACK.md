# Technology Stack

**Analysis Date:** 2025-03-24

## Languages

**Primary:**
- Rust 2021 Edition - All smart contracts (Soroban)

## Runtime

**Environment:**
- Stellar Soroban - Blockchain smart contract runtime on Stellar testnet (`https://soroban-testnet.stellar.org`)

**Build Target:**
- `wasm32-unknown-unknown` - WebAssembly compilation target for Soroban contracts

## Frameworks & SDKs

**Core:**
- Soroban SDK 25.3.0 - Smart contract development framework for Stellar Soroban
- Soroban Fixed-Point Math 1.5.0 - Fixed-point arithmetic library (`SCALAR_7` = 10^7 for rates/fees, `SCALAR_18` for indices)

**OpenZeppelin Stellar Contracts (v0.6.0):**
- `stellar-access` - Ownership and access control traits (`Ownable`, `only_owner` macro)
- `stellar-tokens` - ERC-4626 compliant vault and token implementations (`FungibleToken`, `FungibleVault` traits)
- `stellar-contract-utils` - Upgradeable contract utilities
- `stellar-macros` - Derive macros (`#[only_owner]`, `Upgradeable`)

Commit hash: `63167bb707edf4ad25e46572df11d4332d10b68e` (GitHub: OpenZeppelin/stellar-contracts)

**Testing:**
- Soroban SDK testutils - Built-in testing framework with mock contracts and ledger simulation
- Proptest 1.x - Property-based testing for fuzzing

**Build & Development:**
- Stellar CLI (`stellar contract build --optimize`) - Compiles contracts to optimized WASM
- Cargo - Rust package manager and build system
- Cargo LLVM Coverage - Code coverage analysis

## Project Structure & Crates

**Workspace members** (in `Cargo.toml`):
- `trading` - Core perpetual futures trading contract
- `strategy-vault` - ERC-4626 tokenized vault with deposit locking
- `factory` - Deployment factory for atomic vault+trading deployment
- `price-verifier` - Pyth Lazer price verification contract
- `treasury` - Protocol fee collection and rate management
- `governance` - Admin governance with timelock for config updates
- `test-suites` - Shared integration test library

## Build Configuration

**Optimization Profile** (`[profile.release]`):
```
opt-level = "z"           # Optimize for size (required for WASM)
overflow-checks = true    # Runtime overflow detection
debug = 0                 # Strip debug symbols
strip = "symbols"         # Minimize binary size
debug-assertions = false  # Disable debug assertions in release
panic = "abort"           # Abort instead of unwinding (WASM compat)
codegen-units = 1         # Single codegen unit for optimization
lto = true                # Link-time optimization
```

**Release-with-logs Profile** (`[profile.release-with-logs]`):
- Inherits release settings but enables `debug-assertions = true` for logging during development

## Compiler & Toolchain

**Edition:** Rust 2021

**Library Configuration:**
- All contracts use `[lib] crate-type = ["lib", "cdylib"]` to support:
  - `lib` - Library linkage for tests/test-suites
  - `cdylib` - WebAssembly dynamic library for Soroban deployment

**No-std Environment:**
```
#![no_std]  # Used across all contracts - no Rust std library (WASM constraint)
```

## Dependencies Summary

**Critical Direct Dependencies:**
- `soroban-sdk` 25.3.0 - Core contract development
- `soroban-fixed-point-math` 1.5.0 - Fixed-point arithmetic
- `stellar-access` 0.6.0 - Access control (OpenZeppelin)
- `stellar-tokens` 0.6.0 - Token standards (OpenZeppelin)
- `stellar-contract-utils` 0.6.0 - Upgradeability utilities
- `stellar-macros` 0.6.0 - Proc macros for decorators

**Transitive Dependencies:**
- `soroban-xdr` 25.0.0 - XDR encoding/decoding for Soroban
- `stellar-xdr` 25.0.0 - Stellar XDR types
- Cryptography: `ed25519-dalek`, `k256`, `sha2`, `blake3` (from soroban-sdk)

**Feature Gating:**
- `testutils` feature - Enables `soroban-sdk/testutils` for contract testing environments
- `library` feature - Used to compile contracts as linkable libraries for `test-suites`

## Platform Requirements

**Development:**
- Rust 1.74+ (via rustup)
- Stellar CLI with `contract` subcommand
- WASM build target: `rustup target add wasm32-unknown-unknown`

**Deployment:**
- Stellar testnet access
- Smart account for transaction signing (soroban-smart-account - separate repository)
- Oracle pricing data (Pyth Lazer format)

## Lockfile

**File:** `Cargo.lock` (present)
- Committed and version-controlled
- Ensures reproducible builds across development and CI environments

## Network Configuration

**Default Network:** Stellar Testnet
- RPC Endpoint: `https://soroban-testnet.stellar.org`
- Network ID: Stellar testnet public key

**Relayer:** OpenZeppelin Relayer at `https://relayer.zenithprotocols.com`
- Handles transaction relaying for keeper operations

---

*Stack analysis: 2025-03-24*
