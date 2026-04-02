use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PriceVerifierError {
    InvalidData = 780,
    InvalidPrice = 781,
    PriceStale = 782,
}
