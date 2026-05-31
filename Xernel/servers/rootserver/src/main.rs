#![no_std]
#![no_main]

//! Root server (PID 1).
//!
//! Phase 3 — receives the initial set of `Untyped` caps from the kernel, hands
//! out caps to children, starts `pm`, `vfs`, and the console driver.

#[no_mangle]
pub extern "C" fn _start() -> ! {
    loop {}
}

#[panic_handler]
fn ph(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
