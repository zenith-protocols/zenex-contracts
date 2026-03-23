#![no_std]

pub mod constants;
pub mod interface;
pub mod storage;
pub mod trading;
mod dependencies;
mod errors;
mod events;
mod types;
mod validation;

#[cfg(any(not(feature = "library"), test, feature = "testutils"))]
mod contract;

#[cfg(any(test, feature = "testutils"))]
pub mod testutils;

#[cfg(any(not(feature = "library"), test, feature = "testutils"))]
pub use contract::*;
pub use interface::*;
pub use errors::TradingError;
pub use trading::{ExecuteRequest, ExecuteRequestType};
pub use dependencies::{PriceData, scalar_from_exponent};
pub use types::*;
