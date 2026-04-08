# Zenex Contracts

Perpetual futures trading protocol on [Stellar](https://stellar.org) (Soroban).

## Crates

| Crate | Description |
|-------|-------------|
| `trading` | Core perps engine — positions, fees, funding, liquidations, ADL |
| `strategy-vault` | ERC-4626 vault with deposit locking |
| `factory` | Atomic vault + trading deployment |
| `price-verifier` | Pyth Lazer oracle price verification |
| `treasury` | Protocol fee collection |
| `governance` | Timelock-gated config updates |
| `test-suites` | Integration tests and shared fixtures |

## Build

```sh
make build
```

## Test

```sh
make test
```

## License

Proprietary.
