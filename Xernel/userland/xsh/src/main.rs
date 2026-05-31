#![no_std]
#![no_main]

//! Xernel shell — phase 5/6. Native, not a bash port; pipes built on
//! Endpoint caps.

#[no_mangle]
pub extern "C" fn _start() -> ! {
    loop {}
}

#[panic_handler]
fn ph(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
