#![no_std]
#![no_main]

//! virtio driver — phase 4. virtio-blk first (QEMU-friendly), then -net, -gpu.

#[no_mangle]
pub extern "C" fn _start() -> ! {
    loop {}
}

#[panic_handler]
fn ph(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
