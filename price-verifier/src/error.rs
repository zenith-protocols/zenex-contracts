use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    // Envelope errors
    InvalidMagic = 800,
    InvalidSigner = 801,

    // Payload errors
    InvalidPayloadMagic = 810,
    UnknownProperty = 811,
    MissingPrice = 812,
    MissingExponent = 813,
    ConfidenceTooHigh = 814,

    // Price errors
    PriceFeedNotFound = 820,

    // Buffer errors
    BufferTooShort = 830,
}
