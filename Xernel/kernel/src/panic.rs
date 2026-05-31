use core::panic::PanicInfo;

use crate::{arch, println};

#[panic_handler]
fn on_panic(info: &PanicInfo) -> ! {
    println!("[xernel] PANIC: {info}");
    // In automated test runs a panic must terminate the VM with a failure
    // status; otherwise the run would hang forever waiting on isa-debug-exit.
    #[cfg(feature = "boot-test")]
    arch::exit(false);
    #[cfg(not(feature = "boot-test"))]
    arch::halt_forever();
}
