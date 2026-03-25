default: build

test: build
	cargo test --all --tests

build:
	stellar contract build --optimize

fmt:
	cargo fmt --all

clean:
	cargo clean

coverage:
	cargo llvm-cov --workspace --exclude test-suites --ignore-filename-regex '(testutils|events|test\.rs|test_)'

coverage-html:
	cargo llvm-cov --workspace --exclude test-suites --ignore-filename-regex '(testutils|events|test\.rs|test_)' --html
	@echo "Coverage report generated at target/llvm-cov/html/index.html"

# Coverage measurement per-contract (informational breakdown)
coverage-per-crate:
	@echo "=== Workspace Coverage (authoritative measurement) ==="
	cargo llvm-cov --workspace --exclude test-suites --ignore-filename-regex '(testutils|test\.rs|test_)'
	@echo ""
	@echo "=== Per-Crate Breakdown (informational) ==="
	@echo "--- Trading ---"
	cargo llvm-cov --package trading --ignore-filename-regex '(testutils|test\.rs|test_)' 2>&1 | tail -3
	@echo "--- Strategy Vault ---"
	cargo llvm-cov --package strategy-vault --ignore-filename-regex '(testutils|test\.rs|test_)' 2>&1 | tail -3
	@echo "--- Factory ---"
	cargo llvm-cov --package factory --ignore-filename-regex '(testutils|test\.rs|test_)' 2>&1 | tail -3
	@echo "--- Price Verifier ---"
	cargo llvm-cov --package price-verifier --ignore-filename-regex '(testutils|test\.rs|test_)' 2>&1 | tail -3
	@echo "--- Governance ---"
	cargo llvm-cov --package governance --ignore-filename-regex '(testutils|test\.rs|test_)' 2>&1 | tail -3

# Mutation testing (best effort, long-running)
mutants:
	cargo mutants --package trading --timeout 120 2>&1 | tee target/mutants-trading.txt
	@echo "Results: target/mutants-trading.txt"