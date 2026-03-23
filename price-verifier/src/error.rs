use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PriceVerifierError {
    InvalidData = 800,
    InvalidPrice = 810,
    PriceStale = 820,
}
