use soroban_sdk::{contractclient, Env};

/// Treasury client interface - manually defined to avoid duplicate symbol conflicts
/// when linking the treasury crate's Ownable/Upgradeable exports into the trading WASM.
#[allow(dead_code)]
#[contractclient(name = "Client")]
pub trait TreasuryInterface {
    /// Returns the current protocol fee rate (SCALAR_7)
    fn get_rate(e: Env) -> i128;
}
