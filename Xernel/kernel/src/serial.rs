//! Thin façade over the architecture-specific serial driver.
//!
//! The real driver lives under `arch::<target>::serial`. This module exists so
//! that generic code can write `println!(...)` without caring which UART is
//! used on the current platform.

use core::fmt::{self, Write};

use crate::arch;

pub struct Writer;

impl Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        arch::serial_write(s);
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    let _ = Writer.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::serial::_print(core::format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! println {
    () => {{ $crate::print!("\n"); }};
    ($($arg:tt)*) => {{
        $crate::serial::_print(core::format_args!($($arg)*));
        $crate::print!("\n");
    }};
}
