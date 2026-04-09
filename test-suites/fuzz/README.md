# Fuzz Tests

Stateful fuzz tests for the Zenex trading contract using `cargo-fuzz` (libfuzzer). Tests run against native Rust builds with Soroban testutils.

## Targets

| Target | Scope | Users | Markets | Commands/Run |
|---|---|---|---|---|
| `fuzz_trading_general` | Full lifecycle: open, close, limit orders, modify collateral, triggers, funding, ADL, status transitions | 2 | 3 (BTC/ETH/XLM) | 30 |
| `fuzz_liquidation` | Liquidation + ADL edge cases: interest-only liquidation, near-boundary oscillation, ADL reduction verification | 3 + keeper | 1 (BTC) | 3 scenarios x 15 steps |

## Invariants

Checked after every operation in both targets:

| # | Invariant | Priority |
|---|---|---|
| 1 | **Zero residual** — contract holds 0 tokens when no positions are open | P0 |
| 2 | **Valid errors** — contract errors must be valid TradingError codes (no VM/budget panics) | P0 |
| 3 | **Borrowing index monotonicity** — `l/s_borr_idx` never decrease | P1 |
| 4 | **ADL index monotonicity** — `l/s_adl_idx` never increase (reduction only) | P1 |
| 5 | **Liquidated positions removed** — position gone from storage after successful execute() | P2 |
| 6 | **ADL index bounded** — ADL indices always <= SCALAR_18 | P2 |

## Commands (fuzz_trading_general)

| Command | What it exercises |
|---|---|
| `OpenMarket` | Immediate fill at current oracle price |
| `PlaceLimit` | Pending limit order at offset from current price |
| `FillLimit` | Keeper fills pending order via execute() |
| `ClosePosition` | User closes filled position with settlement |
| `CancelLimit` | Cancel unfilled limit order, refund collateral |
| `ModifyCollateral` | Add/withdraw collateral with margin validation |
| `SetTriggers` | Update TP/SL on filled positions |
| `ApplyFunding` | Hourly funding rate recalculation |
| `UpdateStatus` | Circuit breaker + ADL via update_status() |
| `PassTime` | Advance ledger clock (1s - 24h) |
| `UpdatePrice` | Shift oracle price by +/- 50% |

## Prerequisites

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
```

## Running

From this directory (`test-suites/fuzz/`):

```bash
# Run a target (runs indefinitely until Ctrl+C)
cargo +nightly fuzz run fuzz_trading_general
cargo +nightly fuzz run fuzz_liquidation

# Time-limited run (seconds)
cargo +nightly fuzz run fuzz_trading_general -- -max_total_time=300

# Parallel fuzzing
cargo +nightly fuzz run fuzz_trading_general --jobs 4

# More memory
cargo +nightly fuzz run fuzz_liquidation -- -rss_limit_mb=4096
```

## Reproducing crashes

```bash
cargo +nightly fuzz run fuzz_trading_general artifacts/fuzz_trading_general/<crash-file>
```

## Corpus

The `corpus/` directory accumulates interesting inputs across runs. It's gitignored but persists locally. To start fresh:

```bash
rm -rf corpus/
```
