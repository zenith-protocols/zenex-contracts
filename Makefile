default: build

test: build
	cargo test --all --tests

build:
	stellar contract build

fmt:
	cargo fmt --all

clean:
	cargo clean

coverage:
	cargo llvm-cov --workspace --exclude test-suites --ignore-filename-regex '(testutils|events|test\.rs|test_)'

coverage-html:
	cargo llvm-cov --workspace --exclude test-suites --ignore-filename-regex '(testutils|events|test\.rs|test_)' --html
	@echo "Coverage report generated at target/llvm-cov/html/index.html"

coverage-trading:
	cargo llvm-cov --package trading --lib --ignore-filename-regex '(testutils|events)'