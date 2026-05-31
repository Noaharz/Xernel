#![no_std]
#![no_main]

//! Net stack — phase 5, built on `smoltcp`.

#[no_mangle]
pub extern "C" fn _start() -> ! {
    loop {}
}

#[panic_handler]
fn ph(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
