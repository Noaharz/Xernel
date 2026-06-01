//! Minimal PCI configuration-space access (mechanism #1, ports 0xCF8/0xCFC).
//!
//! Port I/O is privileged, so a user-space driver cannot do this itself; the
//! kernel performs the access on its behalf (via `SYS_PCI_READ`). That keeps
//! the privileged operation mediated by the kernel — the capability-clean way
//! to give a user-space driver hardware discovery.

use x86_64::instructions::port::Port;

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

/// Read a 32-bit dword from the PCI configuration space of `bus:dev.func` at
/// the dword-aligned byte `offset`.
pub fn config_read(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    let address = 0x8000_0000u32
        | (u32::from(bus) << 16)
        | (u32::from(dev) << 11)
        | (u32::from(func) << 8)
        | (u32::from(offset) & 0xFC);
    // SAFETY: the standard PCI configuration mechanism #1 ports; reading config
    // space has no side effects beyond returning the requested register.
    unsafe {
        let mut addr = Port::<u32>::new(CONFIG_ADDRESS);
        let mut data = Port::<u32>::new(CONFIG_DATA);
        addr.write(address);
        data.read()
    }
}

/// Read `size` (1/2/4) bytes from an I/O port. Privileged, so the kernel does it
/// for a user-space driver (e.g. a legacy virtio device's I/O BAR).
///
/// NOTE: currently any port is allowed; a real system would capability-gate
/// which ports a driver may touch.
pub fn port_in(port: u16, size: u8) -> u32 {
    // SAFETY: a port read; for device registers this is the intended access.
    unsafe {
        match size {
            1 => u32::from(Port::<u8>::new(port).read()),
            2 => u32::from(Port::<u16>::new(port).read()),
            _ => Port::<u32>::new(port).read(),
        }
    }
}

/// Write `size` (1/2/4) bytes to an I/O port. See [`port_in`] for the caveat.
pub fn port_out(port: u16, size: u8, value: u32) {
    // SAFETY: a port write to a device register chosen by the driver.
    unsafe {
        match size {
            1 => Port::<u8>::new(port).write(value as u8),
            2 => Port::<u16>::new(port).write(value as u16),
            _ => Port::<u32>::new(port).write(value),
        }
    }
}
