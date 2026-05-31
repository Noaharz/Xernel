#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![deny(unsafe_op_in_unsafe_fn)]
// Early-stage kernel: much of the HAL surface and several subsystems (the
// capability tables, the cooperative scheduler + IPC demo, the boot-test-only
// self-tests) are deliberately built ahead of their first caller, or are only
// reached under the `boot-test` feature. Silence dead-code/unused-import noise
// for now; this allow should be removed once every subsystem has a live caller.
#![allow(dead_code)]
#![allow(unused_imports)]

//! Xernel — entry point.
//!
//! The boot flow is:
//!   1. Limine hands control to [`kmain`] in 64-bit long mode with paging
//!      already set up and the kernel mapped in the higher half.
//!   2. [`arch::init`] brings up the architecture-specific HAL: serial,
//!      interrupts, paging primitives, timer.
//!   3. Once `arch::init` returns, the kernel is allowed to call into
//!      generic subsystems (memory, scheduler, ipc, cap, syscall).
//!
//! Anything beyond the early boot prologue lives in submodules.

extern crate alloc;

mod arch;
mod cap;
mod demo;
mod elf;
mod ipc;
mod mm;
mod panic;
mod sched;
mod serial;
mod syscall;
mod user;

use core::sync::atomic::{AtomicBool, Ordering};

/// Marker so we can detect a double-entry (which would mean a bug in the
/// bootloader handoff or a runaway secondary CPU).
static BOOTED: AtomicBool = AtomicBool::new(false);

#[no_mangle]
extern "C" fn kmain() -> ! {
    if BOOTED.swap(true, Ordering::SeqCst) {
        // We were entered twice. Halt this CPU; the BSP is already running.
        arch::halt_forever();
    }

    arch::init();
    mm::init_early_heap();
    mm::init_frames();
    arch::enable_interrupts();

    println!(
        "[xernel] hello, xernel — arch={}, build={}",
        arch::NAME,
        if cfg!(debug_assertions) { "debug" } else { "release" },
    );
    mm::report();

    #[cfg(feature = "boot-test")]
    {
        assert!(arch::paging_selftest(), "paging self-test failed");
        wait_for_timer_ticks(5);
        println!("[xernel] timer: {} ticks", arch::timer_ticks());
        cap::selftest().expect("capability self-test failed");
        println!("[xernel] cap: self-test ok");
        assert!(arch::keyboard_selftest(), "keyboard self-test failed");
        println!("[xernel] kbd: self-test ok");
    }

    // Load and enter the first ring-3 user program. Never returns; under
    // boot-test the SYS_EXIT handler exits QEMU on success.
    //
    // Note: the milestone-2.0 IPC demo (`demo::run`) also lives here but cannot
    // run in the same boot, because the cooperative scheduler's `start()`
    // abandons this boot context and never returns. It is exercised on its own.
    user::run();
}

/// Spin until the LAPIC timer has advanced by `n` ticks, with a bounded budget
/// so a dead timer fails the test instead of hanging forever.
#[cfg(feature = "boot-test")]
fn wait_for_timer_ticks(n: u64) {
    let start = arch::timer_ticks();
    let mut budget: u64 = 5_000_000_000;
    while arch::timer_ticks() < start + n {
        core::hint::spin_loop();
        budget -= 1;
        assert!(budget != 0, "timer never ticked — LAPIC timer not firing");
    }
}
