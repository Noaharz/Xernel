//! 16550 UART driver on COM1.

use spin::Mutex;
use uart_16550::SerialPort;

static COM1: Mutex<Option<SerialPort>> = Mutex::new(None);

pub fn init() {
    // SAFETY: COM1 lives at the legacy I/O port 0x3F8 on every PC-class system
    // QEMU emulates; reading/writing this port has no side effects beyond the
    // UART itself.
    let mut port = unsafe { SerialPort::new(0x3F8) };
    port.init();
    *COM1.lock() = Some(port);
}

pub fn write_str(s: &str) {
    use core::fmt::Write as _;
    if let Some(port) = COM1.lock().as_mut() {
        let _ = port.write_str(s);
    }
}
