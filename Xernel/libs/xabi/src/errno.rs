//! Error codes returned by capability invocations.
//!
//! Numeric values are part of the ABI — append-only.

#[repr(i32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CapError {
    Ok = 0,
    InvalidCap = 1,
    InvalidArg = 2,
    NoMem = 3,
    NotPermitted = 4,
    WouldBlock = 5,
    BadOp = 6,
    Faulted = 7,
}
