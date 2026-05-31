#![no_std]

//! Xernel driver framework.
//!
//! Drivers in Xernel live in user space. This crate provides the building
//! blocks:
//! - `Dma` — pinned, physically-contiguous buffer backed by a `Frame` cap.
//! - `Mmio` — `Frame` cap with `device_memory` attribute, mapped into the
//!   driver's VSpace.
//! - `Irq` — `IrqHandler` cap delivering interrupts as `Notification` bits.
//!
//! Hot-restart is a first-class concern: drivers must be written so the
//! `pm` server can restart them without rebooting the system.
