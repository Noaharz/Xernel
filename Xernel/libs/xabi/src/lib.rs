#![no_std]

//! Xernel ABI.
//!
//! This crate is the single source of truth for the binary contract between
//! kernel and userspace. It is **explicitly not** Linux-shaped: every
//! interaction with the kernel is a capability invocation.
//!
//! Both the kernel and user-space libraries (`xstd`, `xlibc`, drivers, …)
//! depend on this crate. Nothing here may pull in `std` or platform-specific
//! crates.

pub mod cap;
pub mod errno;
pub mod syscall;
