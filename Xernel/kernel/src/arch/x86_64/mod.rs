//! x86_64 HAL.
//!
//! Phase 0 scope: bring up the serial port, hand control back. GDT, IDT, paging
//! and APIC come in phase 1 (`gdt`, `idt`, `paging`, `apic` modules).

mod serial;
pub mod qemu;
mod apic;
mod context;
mod cpu;
mod framebuffer;
mod gdt;
mod idt;
mod keyboard;
mod paging;
mod pci;
mod syscall;
mod vspace;

use limine::request::{HhdmRequest, MemmapRequest, ModulesRequest};
use limine::{BaseRevision, RequestsEndMarker, RequestsStartMarker};

#[used]
#[link_section = ".requests_start_marker"]
static REQS_START: RequestsStartMarker = RequestsStartMarker::new();

// Base revision 2: the classic Limine semantics where bootloader responses use
// higher-half (HHDM) virtual addresses. Revision 6 (the crate default) requires
// a newer bootloader than the v7 binaries we ship and changes several pointers
// to physical addresses — not worth the complexity here.
#[used]
#[link_section = ".requests"]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(2);

#[used]
#[link_section = ".requests"]
static HHDM: HhdmRequest = HhdmRequest::new();

#[used]
#[link_section = ".requests"]
static MEMMAP: MemmapRequest = MemmapRequest::new();

#[used]
#[link_section = ".requests"]
static MODULES: ModulesRequest = ModulesRequest::new();

#[used]
#[link_section = ".requests_end_marker"]
static REQS_END: RequestsEndMarker = RequestsEndMarker::new();

pub fn init() {
    // Serial first: it has no dependencies and gives us a voice for diagnosing
    // everything that follows (including a base-revision mismatch).
    serial::init();
    if !BASE_REVISION.is_supported() {
        crate::println!(
            "[xernel] FATAL: Limine base revision 2 unsupported (bootloader offered {:?})",
            BASE_REVISION.actual_revision(),
        );
        halt_forever();
    }
    gdt::init();
    idt::init();
    cpu::enable_sse();
    paging::init(hhdm_offset());
    // apic::init();  // phase 1

    // Prove the IDT works: a software breakpoint must be caught and resumed.
    x86_64::instructions::interrupts::int3();
}

pub fn serial_write(s: &str) {
    serial::write_str(s);
}

/// Offset at which the bootloader direct-maps all physical memory.
pub fn hhdm_offset() -> u64 {
    HHDM.response().expect("HHDM response missing").offset
}

/// Iterator over usable physical regions as `(start, end)` byte ranges.
pub fn usable_regions() -> impl Iterator<Item = (u64, u64)> {
    let response = MEMMAP.response().expect("memmap response missing");
    response.entries().iter().filter_map(|e| {
        (e.type_ == limine::memmap::MEMMAP_USABLE).then_some((e.base, e.base + e.length))
    })
}

/// Run the arch-specific paging self-test (alloc + map + read-back).
pub fn paging_selftest() -> bool {
    paging::selftest()
}

/// Bytes of the `init` boot module (the first user program), if Limine loaded
/// one. Picks the module whose path ends in `init.elf`, else the first module.
pub fn init_module() -> Option<&'static [u8]> {
    let modules = MODULES.response()?.modules();
    let chosen = modules
        .iter()
        .find(|f| f.path().ends_with("init.elf"))
        .or_else(|| modules.first())?;
    Some(chosen.data())
}

/// Bring up the interrupt controller and unmask interrupts. Must run *after*
/// the frame allocator is live, because configuring the LAPIC maps its MMIO
/// window and that may allocate intermediate page-table frames.
pub fn enable_interrupts() {
    apic::init();
    keyboard::init();
    x86_64::instructions::interrupts::enable();
}

/// Number of LAPIC timer ticks observed so far.
pub fn timer_ticks() -> u64 {
    apic::ticks()
}

/// Block until keyboard input is available, fill `buf`, return bytes read.
pub fn keyboard_read(buf: &mut [u8]) -> usize {
    keyboard::read(buf)
}

/// Drain any buffered keyboard input without blocking; returns bytes read.
pub fn keyboard_read_nb(buf: &mut [u8]) -> usize {
    keyboard::read_nb(buf)
}

/// Framebuffer geometry + user virtual address `[addr, width, height, pitch,
/// bpp]`, mapped into user space on first call. `None` if there is no
/// framebuffer.
pub fn framebuffer_info() -> Option<[u64; 5]> {
    framebuffer::info()
}

// ---- Per-process address spaces ----

/// Create a new address space (kernel half shared). Returns its handle (PML4
/// physical address) or `None`.
pub fn vspace_new() -> Option<u64> {
    vspace::new()
}

/// Map a user page into address space `space`.
pub fn vspace_map(space: u64, virt: u64, phys: u64, writable: bool, executable: bool) -> bool {
    vspace::map_user(space, virt, phys, writable, executable)
}

/// Allocate+zero+map a fresh user page into `space`.
pub fn vspace_alloc_map(space: u64, virt: u64, writable: bool, executable: bool) -> bool {
    vspace::alloc_map_user(space, virt, writable, executable)
}

/// Switch the active address space.
///
/// # Safety
/// `space` must be a valid address space from [`vspace_new`].
pub unsafe fn vspace_switch(space: u64) {
    unsafe { vspace::switch(space) }
}

/// Handle of the currently active address space.
pub fn vspace_current() -> u64 {
    vspace::current()
}

/// Run the address-space self-test (create + switch + read/write + restore).
pub fn vspace_selftest() -> bool {
    vspace::selftest()
}

/// Read a 32-bit PCI config-space dword (mediated privileged port I/O).
pub fn pci_config_read(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    pci::config_read(bus, dev, func, offset)
}

/// In-kernel keyboard decode/buffer self-test (no hardware, no blocking).
pub fn keyboard_selftest() -> bool {
    keyboard::selftest()
}

/// Prepare a fresh thread stack to begin at `entry`; returns its saved RSP.
pub fn init_thread_stack(stack: &mut [u64], entry: extern "C" fn() -> !) -> u64 {
    context::init_stack(stack, entry)
}

/// Initialise `syscall`/`sysret` support (MSRs + per-CPU scratch).
pub fn init_syscalls() {
    syscall::init();
}

/// Set the per-process kernel stack used by BOTH the syscall entry path
/// (PERCPU) and ring 3 -> ring 0 interrupts (TSS RSP0), so a process is always
/// entered/preempted onto its own kernel stack.
pub fn set_kernel_stack(top: u64) {
    syscall::set_kernel_stack(top);
    gdt::set_rsp0(top);
}

/// Map a user-accessible page (`writable`/`executable` control W/NX).
pub fn map_user(virt: u64, phys: u64, writable: bool, executable: bool) -> bool {
    paging::map_user(virt, phys, writable, executable).is_ok()
}

/// Enter ring 3 at `entry` with `user_stack_top`. Never returns.
///
/// # Safety
/// See [`syscall::enter_user`]: the mappings must be valid and user-accessible
/// and [`init_syscalls`] must have run.
pub unsafe fn enter_user(entry: u64, user_stack_top: u64) -> ! {
    unsafe { syscall::enter_user(entry, user_stack_top) }
}

/// Switch CPU context: save current to `*save_rsp`, resume `next_rsp`.
///
/// # Safety
/// See [`context::switch`].
pub unsafe fn switch_context(save_rsp: *mut u64, next_rsp: u64) {
    unsafe { context::switch(save_rsp, next_rsp) }
}

pub fn halt_forever() -> ! {
    loop {
        // SAFETY: `hlt` is always safe; it only stops the CPU until the next
        // interrupt. With interrupts masked this is an effective deadlock
        // signal for QEMU and a power-friendly idle on real hardware.
        unsafe {
            core::arch::asm!("cli; hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Terminate the machine. Under QEMU this exits the emulator with a status
/// derived from `success`; on real hardware it halts.
pub fn exit(success: bool) -> ! {
    qemu::exit(if success {
        qemu::ExitCode::Success
    } else {
        qemu::ExitCode::Failed
    });
}
