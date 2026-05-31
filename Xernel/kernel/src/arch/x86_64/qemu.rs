//! QEMU control via the `isa-debug-exit` device.
//!
//! Wired up by `xtask` with `-device isa-debug-exit,iobase=0xf4,iosize=0x04`.
//! Writing a value `v` to port 0xf4 makes QEMU exit with status `(v << 1) | 1`,
//! so the codes below are chosen to be distinguishable after that transform.
//!
//! This only has an effect under QEMU; on real hardware the write is a no-op.

use x86_64::instructions::port::Port;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit(code: ExitCode) -> ! {
    // SAFETY: 0xf4 is the iobase we tell QEMU to expose isa-debug-exit on.
    // The write has no architectural side effects beyond signalling QEMU.
    unsafe {
        let mut port = Port::new(0xf4);
        port.write(code as u32);
    }
    // If we're not under QEMU (or the device is absent), fall back to halting.
    super::halt_forever();
}
