//! AArch64 HAL — placeholder.
//!
//! Implemented from phase 2 onwards (Raspberry Pi 4/5, Apple Silicon, AR
//! glasses with Cortex-A class SoCs).

pub fn init() {}

pub fn serial_write(_s: &str) {}

pub fn halt_forever() -> ! {
    loop {
        // SAFETY: `wfi` is unprivileged-safe in EL1 and idles until an IRQ.
        unsafe { core::arch::asm!("wfi", options(nomem, nostack, preserves_flags)); }
    }
}

pub fn exit(_success: bool) -> ! {
    halt_forever()
}

pub fn hhdm_offset() -> u64 {
    0
}

pub fn usable_regions() -> impl Iterator<Item = (u64, u64)> {
    core::iter::empty()
}

pub fn paging_selftest() -> bool {
    true
}

pub fn init_module() -> Option<&'static [u8]> {
    None
}

pub fn keyboard_read(_buf: &mut [u8]) -> usize {
    0
}

pub fn keyboard_read_nb(_buf: &mut [u8]) -> usize {
    0
}

pub fn framebuffer_info() -> Option<[u64; 5]> {
    None
}

pub fn vspace_new() -> Option<u64> {
    None
}
pub fn vspace_map(_space: u64, _virt: u64, _phys: u64, _writable: bool, _executable: bool) -> bool {
    false
}
pub fn vspace_alloc_map(_space: u64, _virt: u64, _writable: bool, _executable: bool) -> bool {
    false
}
pub unsafe fn vspace_switch(_space: u64) {}
pub fn vspace_current() -> u64 {
    0
}
pub fn vspace_selftest() -> bool {
    true
}
pub fn set_kernel_stack(_top: u64) {}
pub fn pci_config_read(_bus: u8, _dev: u8, _func: u8, _offset: u8) -> u32 {
    0xFFFF_FFFF
}
pub fn map_user_device(_virt: u64, _phys: u64) -> bool {
    false
}
pub fn port_in(_port: u16, _size: u8) -> u32 {
    0xFFFF_FFFF
}
pub fn port_out(_port: u16, _size: u8, _value: u32) {}

pub fn keyboard_selftest() -> bool {
    true
}

pub fn enable_interrupts() {}

pub fn timer_ticks() -> u64 {
    0
}

pub fn init_thread_stack(_stack: &mut [u64], _entry: extern "C" fn() -> !) -> u64 {
    unimplemented!("aarch64 context switching")
}

pub unsafe fn switch_context(_save_rsp: *mut u64, _next_rsp: u64) {
    unimplemented!("aarch64 context switching")
}

pub fn init_syscalls() {}

pub fn map_user(_virt: u64, _phys: u64, _writable: bool, _executable: bool) -> bool {
    false
}

pub unsafe fn enter_user(_entry: u64, _user_stack_top: u64) -> ! {
    unimplemented!("aarch64 user mode")
}
