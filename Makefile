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

loc:
	@for dir in trading strategy-vault factory price-verifier treasury governance; do \
		echo "=== $$dir ==="; \
		cd $(CURDIR)/$$dir && cargo warloc 2>/dev/null; \
		echo ""; \
		cd $(CURDIR); \
	done

# Mutation testing (best effort, long-running)
mutants:
	cargo mutants --package trading --timeout 120 2>&1 | tee target/mutants-trading.txt
	@echo "Results: target/mutants-trading.txt"