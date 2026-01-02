default: build

test: build
	cargo test --all --tests

build:
	stellar contract build
	stellar contract optimize \
        --wasm target/wasm32v1-none/release/trading.wasm \
        --wasm-out wasm/trading.wasm

fmt:
	cargo fmt --all

clean:
	cargo clean