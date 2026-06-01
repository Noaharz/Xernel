//! Hardware Abstraction Layer.
//!
//! Every architecture exposes the same surface, declared here. New
//! architectures must implement *exactly* this surface — anything generic that
//! creeps in here is a design smell.

#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "x86_64")]
pub use self::x86_64::{
    enable_interrupts, enter_user, exit, halt_forever, hhdm_offset, init, init_module,
    framebuffer_info, init_syscalls, init_thread_stack, keyboard_read, keyboard_read_nb,
    keyboard_selftest, map_user, map_user_device, paging_selftest, pci_config_read, serial_write,
    set_kernel_stack, switch_context, timer_ticks, usable_regions, vspace_alloc_map,
    vspace_current, vspace_map, vspace_new, vspace_selftest, vspace_switch,
};
#[cfg(target_arch = "x86_64")]
pub const NAME: &str = "x86_64";

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "aarch64")]
pub use self::aarch64::{
    enable_interrupts, enter_user, exit, halt_forever, hhdm_offset, init, init_module,
    framebuffer_info, init_syscalls, init_thread_stack, keyboard_read, keyboard_read_nb,
    keyboard_selftest, map_user, map_user_device, paging_selftest, pci_config_read, serial_write,
    set_kernel_stack, switch_context, timer_ticks, usable_regions, vspace_alloc_map,
    vspace_current, vspace_map, vspace_new, vspace_selftest, vspace_switch,
};
#[cfg(target_arch = "aarch64")]
pub const NAME: &str = "aarch64";

#[cfg(target_arch = "riscv64")]
mod riscv64;
#[cfg(target_arch = "riscv64")]
pub use self::riscv64::{
    enable_interrupts, enter_user, exit, halt_forever, hhdm_offset, init, init_module,
    framebuffer_info, init_syscalls, init_thread_stack, keyboard_read, keyboard_read_nb,
    keyboard_selftest, map_user, map_user_device, paging_selftest, pci_config_read, serial_write,
    set_kernel_stack, switch_context, timer_ticks, usable_regions, vspace_alloc_map,
    vspace_current, vspace_map, vspace_new, vspace_selftest, vspace_switch,
};
#[cfg(target_arch = "riscv64")]
pub const NAME: &str = "riscv64";
