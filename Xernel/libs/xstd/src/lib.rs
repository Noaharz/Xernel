#![no_std]

//! Xernel user-space standard crate.
//!
//! Safe, typed wrappers over the raw syscall stubs in `xabi`. Every operation
//! returns `Result<T, xabi::errno::CapError>`.
//!
//! This is **not** a libc replacement — `xlibc` provides the POSIX surface for
//! ports. `xstd` is the native Xernel API.
