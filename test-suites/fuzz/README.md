# Fuzz Tests

Stateful fuzz tests for the Zenex trading contract running against **WASM binaries** through the Soroban VM. This catches VM-specific issues (budget overflows, memory limits, serialization edge cases) that native Rust tests miss.

## Targets

| Target | Purpose |
|---|---|
| `fuzz_trading_general` | Core flow fuzzer: open, close, modify collateral, price changes, and time jumps across 2 users and 3 assets. Checks token conservation, position validity, and zero-residual invariants after every operation. |
| `fuzz_liquidation` | Focused on liquidation edge cases: interest-only liquidation (no price movement), near-boundary oscillation, and slow margin erosion. Fuzzes time jumps, price changes, and liquidation timing independently. |

## Prerequisites

```bash
# Nightly toolchain (auto-selected via rust-toolchain.toml)
rustup toolchain install nightly

# cargo-fuzz
cargo install cargo-fuzz

# Build WASMs first (from repo root)
cd ../.. && make build
```

## Running

From this directory (`test-suites/fuzz/`):

```bash
# Run a target (runs indefinitely until stopped with Ctrl+C)
cargo fuzz run fuzz_trading_general
cargo fuzz run fuzz_liquidation

# Run with a time limit (seconds)
cargo fuzz run fuzz_trading_general -- -max_total_time=300

# Run with multiple jobs (parallel fuzzing)
cargo fuzz run fuzz_trading_general --jobs 4

# Use more memory (default 2048 MB)
cargo fuzz run fuzz_liquidation -- -rss_limit_mb=4096
```

## Reproducing crashes

When a crash is found, the input is saved to `artifacts/`. Reproduce with:

```bash
cargo fuzz run fuzz_trading_general artifacts/fuzz_trading_general/<crash-file>
```

## Corpus

The `corpus/` directory accumulates interesting inputs across runs. It's gitignored but persists locally. To start fresh:

```bash
rm -rf corpus/
```
