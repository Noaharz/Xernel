#![no_std]
#![no_main]

//! Display server.
//!
//! The output abstraction is intentionally wider than "framebuffer": Xernel
//! targets PCs, AR headsets, and embedded panels. The server speaks to
//! back-end driver modules over capabilities; the front-end protocol is the
//! same regardless of whether the sink is a 4K monitor or a stereo HMD.

#[no_mangle]
pub extern "C" fn _start() -> ! {
    loop {}
}

#[panic_handler]
fn ph(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
