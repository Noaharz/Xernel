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

use crate::{arch, mm::frame, println};

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
        SYS_EXIT => exit(args[0]),
        SYS_DEBUG => {
            println!("[user] debug: {:#x}", args[0]);
            0
        }
        SYS_GET_TICKS => arch::timer_ticks(),
        SYS_SYSINFO => sysinfo(args[0]),
        SYS_READ => sys_read(args[0], args[1], args[2]),
        SYS_READ_NB => sys_read_nb(args[0], args[1], args[2]),
        SYS_SBRK => crate::user::sbrk(args[0] as i64).unwrap_or(u64::MAX),
        SYS_FB_INFO => sys_fb_info(args[0]),
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

fn sysinfo(which: u64) -> u64 {
    let (used_frames, total_frames) = frame::stats();
    match which {
        INFO_RAM_TOTAL => total_frames * frame::FRAME_SIZE,
        INFO_RAM_USED => used_frames * frame::FRAME_SIZE,
        INFO_FRAME_SIZE => frame::FRAME_SIZE,
        _ => u64::MAX,
    }
}

fn exit(code: u64) -> ! {
    println!("[user] exit({code})");
    #[cfg(feature = "boot-test")]
    {
        println!("[xernel] boot-test: ok");
        crate::arch::exit(code == 0);
    }
    #[cfg(not(feature = "boot-test"))]
    {
        println!("[xernel] first user process exited; halting.");
        crate::arch::halt_forever();
    }
}
