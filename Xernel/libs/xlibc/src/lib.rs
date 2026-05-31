#![no_std]

//! Xernel POSIX compatibility layer.
//!
//! Implements a *subset* of POSIX on top of native Xernel capabilities so that
//! ports of `coreutils`, `bash`, `vim`, etc. become tractable.
//!
//! **Explicitly not Linux:** there is no `glibc` heritage and no Linux-syscall
//! emulation. This crate exists because POSIX is a useful target surface, not
//! because we want a Linux clone.
