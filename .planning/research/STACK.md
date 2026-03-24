# Technology Stack: Audit Preparation Tooling

**Project:** Zenex Contracts — Audit Preparation
**Researched:** 2026-03-24

## Recommended Stack

### Threat Modeling

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| Stellar STRIDE Template | N/A (doc template) | Threat model document structure | Mandatory for Stellar Audit Bank applications. SDF provides a specific STRIDE template with four sections: scope, threats, mitigations, and validation. Required format for any Stellar ecosystem audit. | HIGH |
| Mermaid / draw.io | Latest | Data-flow diagrams for threat model | STRIDE template requires data-flow diagrams showing external entities, processes, data flows, storage, and trust boundaries. Mermaid renders in Markdown; draw.io for more complex diagrams. | HIGH |

**STRIDE Template Structure (from Stellar docs):**
1. "What are we working on?" — system description + data-flow diagrams
2. "What can go wrong?" — STRIDE table with at least one issue per category (S/T/R/I/D/E)
3. "What are we going to do about it?" — remediation table with numbered mitigations
4. "Did we do a good job?" — reflection checklist

**Template URL:** https://developers.stellar.org/docs/build/security-docs/threat-modeling/STRIDE-template
**How-to Guide:** https://developers.stellar.org/docs/build/security-docs/threat-modeling/threat-modeling-how-to

### Static Analysis

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| Scout Soroban (cargo-scout-audit) | 0.3.x | Soroban-specific vulnerability detection | The only purpose-built static analysis tool for Soroban smart contracts. Built by CoinFabrik (one of the six Audit Bank firms). Includes 24 detectors across critical, medium, and minor severity. Uses Dylint to hook into Rust compiler HIR/MIR. Outputs HTML, Markdown, JSON, PDF, and SARIF reports. | HIGH |
| Clippy | Bundled with rustc | General Rust linting | Standard Rust linter with 500+ lints. Run with `--all-targets -- -D warnings` to enforce zero warnings. Not smart-contract-aware but catches general Rust issues (unused results, unnecessary clones, complexity). | HIGH |
| cargo-audit | Latest | Dependency vulnerability scanning | Scans Cargo.lock against RustSec Advisory Database. Catches known CVEs in dependencies (soroban-sdk, ed25519-dalek, k256, etc.). Built by Rust Secure Code WG. | HIGH |
| cargo-deny | Latest | License + supply chain analysis | Complements cargo-audit with license compliance, duplicate dependency detection, and source verification. Important for audit because auditors check dependency hygiene. | MEDIUM |

**Scout Soroban Detectors (24 total):**

*Critical (8):*
- `overflow-check` — Missing overflow checks in arithmetic
- `insufficiently-random-values` — Weak randomness sources
- `unprotected-update-current-contract-wasm` — Unguarded upgrade functions
- `set-contract-storage` — Arbitrary storage writes
- `avoid-unsafe-block` — Unsafe Rust in contract code
- `unprotected-mapping-operation` — Unguarded map mutations
- `unrestricted-transfer-from` — Arbitrary transfer sources
- `incorrect-exponentiation` — Wrong exponent operations

*Medium (8):*
- `divide-before-multiply` — Precision loss from operation ordering
- `unsafe-unwrap` / `unsafe-expect` — Panics from unchecked unwraps
- `dos-unbounded-operation` — Unbounded loops/storage
- `dos-unexpected-revert-with-vector` — Vector operation DoS
- `unsafe-map-get` — Unchecked map access
- `zero-or-test-address` — Hardcoded/test addresses

*Minor/Enhancement (8):*
- `avoid-core-mem-forget`, `avoid-panic-error`, `soroban-version`, `unused-return-enum`, `iterators-over-indexing`, `assert-violation`, and more

### Test Coverage Measurement

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| cargo-llvm-cov | 0.6.x+ | Line/region coverage reporting | Recommended by Stellar docs for Soroban. Already configured in project Makefile. Generates HTML reports and lcov.info for IDE integration. The standard and only real option for Rust coverage. | HIGH |
| cargo-mutants | Latest (25.x) | Mutation testing | Recommended by Stellar docs. Goes beyond line coverage: verifies tests actually catch code changes. Identifies "MISSED" mutations where tests pass even after code modification. No special config needed for Soroban. | HIGH |

**Existing Makefile targets (already configured):**
```bash
make coverage        # cargo llvm-cov --workspace --exclude test-suites --ignore-filename-regex '(testutils|test\.rs|test_)'
make coverage-html   # Same + --html flag
```

**Recommended additions to Makefile:**
```bash
# Mutation testing
mutants:
	cargo mutants --package trading --package strategy-vault --package factory --package price-verifier

# LCOV for IDE integration
coverage-lcov:
	cargo llvm-cov --workspace --exclude test-suites --ignore-filename-regex '(testutils|test\.rs|test_)' --lcov --output-path=lcov.info
```

### Fuzzing

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| cargo-fuzz | 0.12.x | Fuzzing harness runner (libFuzzer) | Recommended by Stellar for Soroban contracts. Requires nightly toolchain. Existing fuzz targets in test-suites/fuzz/ need updating to current API. | HIGH |
| proptest | 1.x | Property-based testing (stable Rust) | Already a dev-dependency. Converts fuzz findings into reproducible regression tests runnable under stable `cargo test`. Bridges fuzzing and CI. | HIGH |
| arbitrary | 1.x | Structured input generation | Required for Soroban types via `SorobanArbitrary::Prototype`. Already available through soroban-sdk testutils feature. | HIGH |

**Soroban fuzzing requirements:**
- Contract must have `testutils` Cargo feature enabling `soroban-sdk/testutils`
- Fuzz crate needs `libfuzzer-sys`, `soroban-sdk` (with testutils), `soroban-env-host`
- Use `try_*` variants of contract calls (not panicking versions)
- Use `SorobanArbitrary::Prototype` for Soroban types that need `Env`
- Nightly toolchain required: `cargo +nightly fuzz run <target>`
- macOS requires `--sanitizer=thread` flag

### Documentation Generation

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| rustdoc (cargo doc) | Bundled with rustc | API reference from doc comments | Standard Rust documentation. Generates HTML from `///` and `//!` doc comments. Auditors expect function-level documentation explaining invariants, preconditions, and security assumptions. | HIGH |
| mdBook | 0.4.x | Protocol specification / architecture docs | Used by Rust itself and many Soroban ecosystem projects. Renders Markdown book structure into browsable HTML. Suitable for protocol spec, architecture overview, deployment guide, and threat model. | HIGH |

**rustdoc configuration (recommended for audit):**
```bash
# Generate workspace docs with private items visible
cargo doc --workspace --no-deps --document-private-items

# Enforce doc coverage
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

**mdBook structure for audit docs:**
```
docs/
  book.toml
  src/
    SUMMARY.md
    protocol-spec.md        # Trading mechanics, fee system, ADL
    architecture.md         # Contract boundaries, data flow
    threat-model.md         # STRIDE threat model
    api-reference.md        # Public function catalog
    deployment-guide.md     # Deploy flow, config params
    invariants.md           # System invariants and proofs
```

### Formal Verification (Optional / Advanced)

| Technology | Version | Purpose | Why | Confidence |
|------------|---------|---------|-----|------------|
| Certora Sunbeam Prover | Latest | Formal verification of Soroban WASM | The only formal verification tool for Soroban. Operates at WASM bytecode level. Uses CVLR (Cavalier) Rust library with `#[rule]` macros for specs. Cloud-based verification. Used by Blend v2 and Aquarius (Stellar DeFi protocols). | MEDIUM |
| CVLR (cvlr + cvlr-soroban) | Latest | Specification language for Sunbeam | Rust macros (`cvlr_assert!`, `cvlr_assume!`, `cvlr_satisfy!`) for writing formal specifications. Soroban-specific variant handles contract environment. | MEDIUM |

**Why MEDIUM confidence:** Sunbeam is powerful but has limitations: no automatic invariant setup, potential solver timeouts on complex functions, cloud-only execution, and relatively new tooling. Best suited for critical invariants (e.g., "vault balance >= sum of all positions") rather than full protocol verification. Consider for Phase 2 after tests and threat model are complete.

**Setup requires:**
- `certoraSorobanProver` CLI
- `certora_build.py` build script
- CVLR spec files alongside contract code
- Certora cloud account for verification runs

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| Static analysis | Scout Soroban | Clippy alone | Clippy is general Rust. Scout has 24 Soroban-specific detectors including `divide-before-multiply`, `unprotected-mapping-operation`, and `dos-unbounded-operation` that Clippy cannot catch. Use both. |
| Static analysis | Scout Soroban | Rudra | Rudra targets unsafe Rust and memory safety. Soroban contracts are `#![no_std]` with no unsafe blocks. Not relevant for this codebase. |
| Coverage | cargo-llvm-cov | tarpaulin | tarpaulin has incomplete no_std/WASM support. cargo-llvm-cov is the Stellar-recommended tool and already configured. |
| Mutation testing | cargo-mutants | mutest-rs | mutest-rs requires nightly. cargo-mutants works on stable, has Stellar docs integration, and requires zero configuration. |
| Fuzzing | cargo-fuzz | AFL.rs | cargo-fuzz is Stellar-recommended, has Soroban examples in official docs, and integrates with proptest for regression tests. AFL.rs has no Soroban-specific guidance. |
| Documentation | mdBook | Docusaurus | mdBook is Rust-native, supports rustdoc test execution, and integrates naturally with the Rust toolchain. Docusaurus (used for zenex-docs) is JS-based and better for user-facing docs, not audit artifacts. |
| Formal verification | Certora Sunbeam | Kani (AWS) | Kani is a Rust model checker but has no Soroban-specific support. Sunbeam was purpose-built for Soroban WASM and has been used on real Stellar DeFi protocols (Blend, Aquarius). |
| Threat model | Stellar STRIDE | OWASP | Stellar Audit Bank requires STRIDE specifically. OWASP is complementary but not the required format. |
| Supply chain | cargo-deny | cargo-audit alone | cargo-deny adds license compliance and source verification on top of vulnerability scanning. Auditors check both. |

## Installation

```bash
# Static Analysis
cargo install cargo-scout-audit           # Soroban-specific vulnerability scanner
cargo install cargo-dylint dylint-link     # Scout dependency (Rust compiler hook)
cargo install cargo-audit                  # RustSec vulnerability scanner
cargo install cargo-deny                   # License + supply chain checker

# Test Coverage & Mutation Testing
cargo install cargo-llvm-cov              # LLVM-based code coverage (likely already installed)
cargo install --locked cargo-mutants      # Mutation testing

# Fuzzing (requires nightly)
cargo install cargo-fuzz                   # Fuzz test runner
rustup install nightly                     # Nightly toolchain for cargo-fuzz

# Documentation
cargo install mdbook                       # Markdown book generator
# rustdoc ships with rustc — no install needed

# Formal Verification (optional, advanced)
# pip install certora-cli                 # Certora Sunbeam Prover
# See https://docs.certora.com/en/latest/docs/sunbeam/usage.html

# Linting (already available)
# clippy ships with rustc — no install needed
```

## Makefile Integration (Recommended Additions)

```makefile
# Static analysis
scout:
	cargo scout-audit

lint:
	cargo clippy --all-targets --all-features -- -D warnings

audit:
	cargo audit
	cargo deny check

# Coverage
coverage-lcov:
	cargo llvm-cov --workspace --exclude test-suites \
		--ignore-filename-regex '(testutils|test\.rs|test_)' \
		--lcov --output-path=lcov.info

# Mutation testing
mutants:
	cargo mutants --package trading --package strategy-vault \
		--package factory --package price-verifier

# Documentation
docs:
	cargo doc --workspace --no-deps --document-private-items
	@echo "API docs at target/doc/trading/index.html"

docs-book:
	mdbook build docs/
	@echo "Book at docs/book/index.html"

# Fuzzing (requires nightly)
fuzz:
	cd test-suites && cargo +nightly fuzz run fuzz_trading -- -max_len=20000

# Pre-audit check (run all)
pre-audit: lint scout audit test coverage mutants
	@echo "Pre-audit checks complete"
```

## Tool Execution Order for Audit Prep

The tools should be applied in this order during the audit preparation milestone:

1. **cargo clippy + cargo scout-audit** — Fix all warnings and detected issues first (code freeze means fixing detection issues, not logic changes)
2. **cargo audit + cargo deny** — Verify dependency hygiene
3. **cargo llvm-cov** — Establish baseline coverage, identify gaps
4. **Write tests** — Fill coverage gaps identified by llvm-cov
5. **cargo mutants** — Verify test quality beyond line coverage
6. **cargo-fuzz** — Update existing fuzz targets, run fuzzing campaigns
7. **STRIDE threat model** — Document all threats using Stellar template
8. **rustdoc + mdBook** — Generate API reference and protocol docs
9. **Certora Sunbeam** (optional) — Formally verify critical invariants

## Sources

- [Stellar STRIDE Threat Model Template](https://developers.stellar.org/docs/build/security-docs/threat-modeling/STRIDE-template) — HIGH confidence
- [Stellar Threat Modeling How-To Guide](https://developers.stellar.org/docs/build/security-docs/threat-modeling/threat-modeling-how-to) — HIGH confidence
- [Stellar Code Coverage Guide](https://developers.stellar.org/docs/build/guides/testing/code-coverage) — HIGH confidence
- [Stellar Fuzzing Guide](https://developers.stellar.org/docs/build/smart-contracts/example-contracts/fuzzing) — HIGH confidence
- [Stellar Mutation Testing Guide](https://developers.stellar.org/docs/build/guides/testing/mutation-testing) — HIGH confidence
- [Stellar Definitive Guide to Testing](https://stellar.org/blog/developers/the-definitive-guide-to-testing-smart-contracts-on-stellar) — HIGH confidence
- [CoinFabrik Scout Soroban (GitHub)](https://github.com/CoinFabrik/scout-soroban) — HIGH confidence
- [Veridise Soroban Security Checklist](https://veridise.com/blog/audit-insights/building-on-stellar-soroban-grab-this-security-checklist-to-avoid-vulnerabilities/) — HIGH confidence
- [Certora Sunbeam Documentation](https://docs.certora.com/en/latest/docs/sunbeam/index.html) — MEDIUM confidence
- [Certora CVLR GitHub](https://github.com/Certora/cvlr) — MEDIUM confidence
- [Soroban Audit Bank Program](https://stellar.org/grants-and-funding/soroban-audit-bank) — HIGH confidence
- [cargo-llvm-cov (GitHub)](https://github.com/taiki-e/cargo-llvm-cov) — HIGH confidence
- [cargo-mutants Documentation](https://mutants.rs/) — HIGH confidence
- [RustSec Advisory Database](https://rustsec.org/) — HIGH confidence
- [rustdoc Book](https://doc.rust-lang.org/rustdoc/what-is-rustdoc.html) — HIGH confidence
- [mdBook Documentation](https://rust-lang.github.io/mdBook/) — HIGH confidence

---

*Stack research: 2026-03-24*
