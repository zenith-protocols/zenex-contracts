pub(crate) mod trading_contract_wasm {
    pub const WASM: &[u8] = include_bytes!("../../../target/wasm32v1-none/release/trading.wasm");
}