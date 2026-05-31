#![no_std]
#![no_main]

//! Process Manager — phase 5. Owns the supervision tree, hot-restarts drivers.

#[no_mangle]
pub extern "C" fn _start() -> ! {
    loop {}
}

#[panic_handler]
fn ph(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
