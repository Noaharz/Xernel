//! Syscall dispatch — the kernel side of the user/kernel boundary.
//!
//! The architecture layer (`arch::*::syscall`) handles the `syscall`
//! instruction, stack switch and register frame, then calls [`dispatch`] with
//! the syscall number and six argument registers. The eventual model is "every
//! syscall is a capability invocation" (`invoke(cap, method, args)`); this fixed
//! set is the bring-up ABI that lets a real first program run.
//!
//! Calling convention (matches the entry stub): number in `rax`, arguments in
//! `rdi, rsi, rdx, r10, r8, r9`, return value in `rax`.

use alloc::string::String;

use spin::Mutex;

use crate::{arch, mm::frame, println};

/// Next free virtual address for device-MMIO mappings (`SYS_IOMAP`). Each
/// process maps into its OWN address space at the returned address, so a single
/// monotonically increasing allocator across processes is fine (the same VA in
/// different address spaces does not collide).
static NEXT_MMIO_VA: Mutex<u64> = Mutex::new(0x5000_0000);
const MMIO_REGION_END: u64 = 0x6000_0000;
const PAGE: u64 = 4096;

/// Write `len` bytes from user address `ptr` to a console fd (1=stdout,
/// 2=stderr; both go to the serial console). Args: fd, ptr, len. Returns the
/// number of bytes written, or `u64::MAX` on a bad buffer.
pub const SYS_WRITE: u64 = 1;
/// Terminate the current program with the exit code in argument 0.
pub const SYS_EXIT: u64 = 2;
/// Print argument 0 as a hex value (register-level debugging aid).
pub const SYS_DEBUG: u64 = 3;
/// Return the number of timer ticks since boot (a coarse uptime/clock).
pub const SYS_GET_TICKS: u64 = 4;
/// Query a system property. Arg 0 selects: 0 = total RAM bytes, 1 = used RAM
/// bytes, 2 = frame size. Returns the value, or `u64::MAX` for unknown keys.
pub const SYS_SYSINFO: u64 = 5;
/// Read keyboard input into a user buffer, blocking until at least one byte is
/// available. Args: fd, ptr, len. Returns the number of bytes read, or
/// `u64::MAX` on a bad buffer.
pub const SYS_READ: u64 = 6;
/// Like [`SYS_READ`] but never blocks: returns immediately with the bytes that
/// were already buffered (possibly 0). Args: fd, ptr, len.
pub const SYS_READ_NB: u64 = 7;
/// Adjust the program break by a signed `delta` (Unix `sbrk`). Arg 0 is the
/// delta in bytes (reinterpreted as i64). Returns the PREVIOUS break address,
/// or `u64::MAX` on failure. `delta == 0` queries the current break.
pub const SYS_SBRK: u64 = 8;
/// Query the framebuffer and map it into user space. Arg 0 is a pointer to a
/// `[u64; 5]` the kernel fills with `[addr, width, height, pitch, bpp]`.
/// Returns 0 on success, `u64::MAX` if there is no framebuffer or a bad buffer.
pub const SYS_FB_INFO: u64 = 9;
/// Return the current process's PID.
pub const SYS_GETPID: u64 = 10;
/// Voluntarily yield the CPU to another ready process. Returns 0.
pub const SYS_YIELD: u64 = 11;
/// Read a 32-bit PCI config dword. Args: bus, dev, func, offset. Returns the
/// dword. Lets a user-space driver discover PCI devices (privileged port I/O is
/// done by the kernel on its behalf).
pub const SYS_PCI_READ: u64 = 12;
/// Map device MMIO into the calling process. Args: phys, len. Returns the user
/// virtual address the region is mapped at (uncached), or `u64::MAX` on failure.
/// Lets a user-space driver reach a device's memory-mapped registers (e.g. a
/// PCI BAR).
pub const SYS_IOMAP: u64 = 13;
/// Allocate a physically-contiguous, zeroed DMA buffer and map it into the
/// caller. Args: len, out_ptr (pointer to a `[u64; 2]` the kernel fills with
/// `[user_vaddr, phys_addr]`). Returns 0 on success, `u64::MAX` on failure. The
/// phys address is what a device is told to DMA to/from.
pub const SYS_DMA_ALLOC: u64 = 14;
/// Read an I/O port. Args: port, size (1/2/4). Returns the value. For
/// user-space drivers of legacy (I/O-BAR) devices.
pub const SYS_PORT_IN: u64 = 15;
/// Write an I/O port. Args: port, size (1/2/4), value. Returns 0.
pub const SYS_PORT_OUT: u64 = 16;

/// Next free virtual address for DMA-buffer mappings (`SYS_DMA_ALLOC`).
static NEXT_DMA_VA: Mutex<u64> = Mutex::new(0x6000_0000);
const DMA_REGION_END: u64 = 0x7000_0000;

// sysinfo keys.
const INFO_RAM_TOTAL: u64 = 0;
const INFO_RAM_USED: u64 = 1;
const INFO_FRAME_SIZE: u64 = 2;

/// Highest user virtual address (exclusive). User pointers must stay in the
/// lower canonical half; this keeps a stray pointer from reaching kernel space.
const USER_ADDR_MAX: u64 = 0x0000_8000_0000_0000;
/// Upper bound on a single write, so a bogus length can't make us read forever.
const MAX_WRITE: u64 = 1 << 20;

/// Dispatch a syscall. `args` are `[rdi, rsi, rdx, r10, r8, r9]`. Returns the
/// value placed in the user's `rax`.
pub fn dispatch(nr: u64, args: [u64; 6]) -> u64 {
    match nr {
        SYS_WRITE => sys_write(args[0], args[1], args[2]),
        SYS_EXIT => crate::process::exit(args[0]),
        SYS_DEBUG => {
            println!("[user] debug: {:#x}", args[0]);
            0
        }
        SYS_GET_TICKS => arch::timer_ticks(),
        SYS_SYSINFO => sysinfo(args[0]),
        SYS_READ => sys_read(args[0], args[1], args[2]),
        SYS_READ_NB => sys_read_nb(args[0], args[1], args[2]),
        SYS_SBRK => crate::process::sbrk(args[0] as i64).unwrap_or(u64::MAX),
        SYS_FB_INFO => sys_fb_info(args[0]),
        SYS_GETPID => crate::process::getpid(),
        SYS_YIELD => {
            crate::process::yield_now();
            0
        }
        SYS_PCI_READ => u64::from(arch::pci_config_read(
            args[0] as u8,
            args[1] as u8,
            args[2] as u8,
            args[3] as u8,
        )),
        SYS_IOMAP => sys_iomap(args[0], args[1]),
        SYS_DMA_ALLOC => sys_dma_alloc(args[0], args[1]),
        SYS_PORT_IN => u64::from(arch::port_in(args[0] as u16, args[1] as u8)),
        SYS_PORT_OUT => {
            arch::port_out(args[0] as u16, args[1] as u8, args[2] as u32);
            0
        }
        other => {
            println!("[user] syscall: unknown number {other}");
            u64::MAX
        }
    }
}

/// Borrow a user buffer as a slice, validating it lies wholly in user space.
fn user_slice(ptr: u64, len: u64) -> Option<&'static [u8]> {
    if len == 0 {
        return Some(&[]);
    }
    if ptr == 0 || len > MAX_WRITE {
        return None;
    }
    let end = ptr.checked_add(len)?;
    if ptr >= USER_ADDR_MAX || end > USER_ADDR_MAX {
        return None;
    }
    // SAFETY: user pages share the active address space, so a validated
    // lower-half pointer is readable from ring 0 (SMAP is not enabled). The
    // 'static lifetime is a lie we keep contained to this function.
    Some(unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) })
}

fn sys_write(_fd: u64, ptr: u64, len: u64) -> u64 {
    let Some(buf) = user_slice(ptr, len) else {
        return u64::MAX;
    };
    // Lossily decode so arbitrary bytes never panic the kernel; real programs
    // send UTF-8/ASCII.
    let text = String::from_utf8_lossy(buf);
    arch::serial_write(&text);
    len
}

/// Validate a user buffer and borrow it mutably (for input).
fn user_slice_mut(ptr: u64, len: u64) -> Option<&'static mut [u8]> {
    if len == 0 {
        return None;
    }
    if ptr == 0 || len > MAX_WRITE {
        return None;
    }
    let end = ptr.checked_add(len)?;
    if ptr >= USER_ADDR_MAX || end > USER_ADDR_MAX {
        return None;
    }
    // SAFETY: user pages share the active address space; a validated lower-half
    // pointer is writable from ring 0 (no SMAP). Single-process bring-up means
    // no aliasing with concurrent kernel access.
    Some(unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize) })
}

fn sys_read(_fd: u64, ptr: u64, len: u64) -> u64 {
    let Some(buf) = user_slice_mut(ptr, len) else {
        return u64::MAX;
    };
    arch::keyboard_read(buf) as u64
}

fn sys_read_nb(_fd: u64, ptr: u64, len: u64) -> u64 {
    let Some(buf) = user_slice_mut(ptr, len) else {
        return u64::MAX;
    };
    arch::keyboard_read_nb(buf) as u64
}

fn sys_fb_info(ptr: u64) -> u64 {
    let Some(info) = arch::framebuffer_info() else {
        return u64::MAX;
    };
    // 5 u64 fields: [addr, width, height, pitch, bpp].
    let Some(buf) = user_slice_mut(ptr, 5 * 8) else {
        return u64::MAX;
    };
    for (i, value) in info.iter().enumerate() {
        buf[i * 8..i * 8 + 8].copy_from_slice(&value.to_le_bytes());
    }
    0
}

/// Map `len` bytes of device physical memory (a PCI BAR) into the current
/// process's address space, uncached. Returns the user virtual address.
fn sys_iomap(phys: u64, len: u64) -> u64 {
    if len == 0 || len > (16 << 20) {
        return u64::MAX;
    }
    let phys_base = phys & !(PAGE - 1);
    let offset = phys - phys_base;
    let pages = (offset + len).div_ceil(PAGE);

    let mut next = NEXT_MMIO_VA.lock();
    let va_base = *next;
    match va_base.checked_add(pages * PAGE) {
        Some(end) if end <= MMIO_REGION_END => {}
        _ => return u64::MAX,
    }
    for i in 0..pages {
        if !arch::map_user_device(va_base + i * PAGE, phys_base + i * PAGE) {
            return u64::MAX;
        }
    }
    *next = va_base + pages * PAGE;
    va_base + offset
}

/// Allocate a physically-contiguous, zeroed DMA buffer of `len` bytes, map it
/// into the current process (cached, RW, NX), and write `[user_vaddr, phys]`
/// to `out_ptr`. Returns 0 on success.
fn sys_dma_alloc(len: u64, out_ptr: u64) -> u64 {
    if len == 0 || len > (1 << 20) {
        return u64::MAX;
    }
    let pages = len.div_ceil(PAGE);
    let Some(phys) = frame::alloc_contiguous(pages) else {
        return u64::MAX;
    };
    // Zero the buffer through the HHDM before user code sees it.
    // SAFETY: freshly allocated contiguous frames, reachable via the HHDM.
    unsafe {
        core::ptr::write_bytes(
            (phys + arch::hhdm_offset()) as *mut u8,
            0,
            (pages * PAGE) as usize,
        );
    }
    let mut next = NEXT_DMA_VA.lock();
    let va_base = *next;
    match va_base.checked_add(pages * PAGE) {
        Some(end) if end <= DMA_REGION_END => {}
        _ => return u64::MAX,
    }
    for i in 0..pages {
        if !arch::map_user(va_base + i * PAGE, phys + i * PAGE, true, false) {
            return u64::MAX;
        }
    }
    *next = va_base + pages * PAGE;
    drop(next);

    let Some(buf) = user_slice_mut(out_ptr, 16) else {
        return u64::MAX;
    };
    buf[0..8].copy_from_slice(&va_base.to_le_bytes());
    buf[8..16].copy_from_slice(&phys.to_le_bytes());
    0
}

fn sysinfo(which: u64) -> u64 {
    let (used_frames, total_frames) = frame::stats();
    match which {
        INFO_RAM_TOTAL => total_frames * frame::FRAME_SIZE,
        INFO_RAM_USED => used_frames * frame::FRAME_SIZE,
        INFO_FRAME_SIZE => frame::FRAME_SIZE,
        _ => u64::MAX,
    }
}

