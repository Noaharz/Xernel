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
/// `[user_vaddr, phys_addr]`). Returns 0 on success, `u64::MAX` on failure (or
/// if the allocation exceeds the caller's `Untyped` budget). The phys address is
/// what a device is told to DMA to/from.
pub const SYS_DMA_ALLOC: u64 = 14;
/// Read an I/O port. Args: port, size (1/2/4). Returns the value. For
/// user-space drivers of legacy (I/O-BAR) devices.
pub const SYS_PORT_IN: u64 = 15;
/// Write an I/O port. Args: port, size (1/2/4), value. Returns 0.
pub const SYS_PORT_OUT: u64 = 16;
/// Identify the capability in one of the caller's own CNode slots. Args: slot,
/// out_ptr (pointer to a `[u64; 3]` the kernel fills with `[type, a, b]`, a
/// normalized view — see `CapEntry::describe`). Returns 0 on success, `u64::MAX`
/// if the slot is empty or out of range. Lets a process enumerate the authority
/// it holds.
pub const SYS_CAP_IDENTIFY: u64 = 17;
/// Send a message over an endpoint. Args: ep_slot (CNode slot holding an
/// `Endpoint` cap), word, cap_slot (CNode slot whose capability to grant the
/// receiver, or `u64::MAX` for none). Non-blocking. Returns 0, or `u64::MAX` if
/// the endpoint cap is missing or `cap_slot` names no capability.
pub const SYS_SEND: u64 = 18;
/// Receive a message from an endpoint, blocking until one arrives. Args: ep_slot
/// (CNode slot holding an `Endpoint` cap), out_ptr (pointer to a `u64` for the
/// message word), dst_slot (CNode slot to install a granted capability into, or
/// `u64::MAX` to discard any). Returns 0, or `u64::MAX` on a missing endpoint
/// cap / bad buffer / occupied destination slot.
pub const SYS_RECV: u64 = 19;
/// Spawn a new process. Arg 0 selects the program image (today only the boot
/// init image, index 0, exists). The newcomer starts in a fresh address space
/// with a freshly seeded capability space and is scheduled as ready. Returns the
/// new PID, or `u64::MAX` on failure. The kernel boots only the root; every other
/// process is created this way.
pub const SYS_SPAWN: u64 = 20;
/// Signal a notification: OR `bits` (arg 1) into the notification named by the
/// `Notification` cap in `notif_slot` (arg 0). Non-blocking, never loses bits.
/// Returns 0, or `u64::MAX` if the slot holds no notification capability. The
/// async readiness primitive — a service signals readiness, a waiter wakes.
pub const SYS_SIGNAL: u64 = 21;
/// Wait on a notification: block (by yielding) until the notification named by
/// the `Notification` cap in `notif_slot` (arg 0) has non-zero bits, then return
/// those bits and clear them. Returns the bits (always non-zero on success), or
/// 0 if the slot holds no notification capability. One wait can cover many
/// readiness sources (each sets its own bit) — the `epoll`/`kqueue` shape.
pub const SYS_WAIT: u64 = 22;
/// Allocate a physically-contiguous, zeroed run of `pages` (arg 0) 4 KiB frames,
/// map it into the caller (cached, RW, NX), install a `Frame` capability naming
/// it into CNode slot `cap_slot` (arg 1), and write the mapped user virtual
/// address to `out_ptr` (arg 2). Returns 0 on success, or `u64::MAX` on failure
/// (or if the allocation exceeds the caller's `Untyped` budget). The Frame cap
/// can then be GRANTED over an endpoint; whoever receives it and maps it (via
/// `SYS_MAP_FRAME`) shares the very same physical memory — the bulk-data path.
pub const SYS_FRAME_ALLOC: u64 = 23;
/// Map a frame named by the `Frame` capability in CNode slot `cap_slot` (arg 0)
/// into the caller's address space (cached, RW, NX) and write the mapped user
/// virtual address to `out_ptr` (arg 1). Returns 0 on success, `u64::MAX` if the
/// slot holds no Frame cap or mapping fails. This is the receiving half of
/// shared memory: two processes that both map the same delegated Frame cap see
/// the same RAM.
pub const SYS_MAP_FRAME: u64 = 24;

/// Next free virtual address for DMA-buffer mappings (`SYS_DMA_ALLOC`).
static NEXT_DMA_VA: Mutex<u64> = Mutex::new(0x6000_0000);
const DMA_REGION_END: u64 = 0x7000_0000;

/// Next free virtual address for shared-memory frame mappings
/// (`SYS_FRAME_ALLOC` / `SYS_MAP_FRAME`). Like the MMIO/DMA windows this is a
/// single monotonic allocator across processes — each process maps into its OWN
/// address space, so the same VA in different spaces does not collide.
static NEXT_SHARED_VA: Mutex<u64> = Mutex::new(0x7000_0000);
const SHARED_REGION_END: u64 = 0x8000_0000;
/// Upper bound on a single shared-frame allocation (256 KiB = 64 pages), so a
/// bogus page count can't exhaust RAM in one call.
const MAX_SHARED_PAGES: u64 = 64;

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
        SYS_PORT_IN => sys_port_in(args[0] as u16, args[1] as u8),
        SYS_PORT_OUT => sys_port_out(args[0] as u16, args[1] as u8, args[2] as u32),
        SYS_CAP_IDENTIFY => sys_cap_identify(args[0], args[1]),
        SYS_SEND => sys_send(args[0], args[1], args[2]),
        SYS_RECV => sys_recv(args[0], args[1], args[2]),
        SYS_SPAWN => crate::process::spawn(args[0]).unwrap_or(u64::MAX),
        SYS_SIGNAL => sys_signal(args[0], args[1]),
        SYS_WAIT => sys_wait(args[0]),
        SYS_FRAME_ALLOC => sys_frame_alloc(args[0], args[1], args[2]),
        SYS_MAP_FRAME => sys_map_frame(args[0], args[1]),
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

    // Authority check: the process must hold an IoMem capability covering the
    // whole physical span we are about to map — no mapping arbitrary RAM.
    if !crate::process::current_authorizes_mmio(phys_base, pages * PAGE) {
        println!("[cap] DENY iomap phys {phys_base:#x} (no IoMem capability)");
        return u64::MAX;
    }

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
///
/// Bounded by an `Untyped` capability: the allocation is first charged against
/// the process's memory budget, so a driver cannot pin unbounded physical
/// memory for DMA. Unlike the port/MMIO gates (which check a fixed range), this
/// budget is consumable — it shrinks with each allocation.
fn sys_dma_alloc(len: u64, out_ptr: u64) -> u64 {
    if len == 0 || len > (1 << 20) {
        return u64::MAX;
    }
    let pages = len.div_ceil(PAGE);
    let need = pages * PAGE;

    if !crate::process::current_charge_untyped(need) {
        println!("[cap] DENY dma_alloc {need:#x} bytes (Untyped budget exhausted)");
        return u64::MAX;
    }

    // The charge is committed; if the allocation itself fails, give it back.
    match dma_alloc_charged(pages, out_ptr) {
        Some(()) => 0,
        None => {
            crate::process::current_refund_untyped(need);
            u64::MAX
        }
    }
}

/// Do the actual DMA allocation+mapping for `pages` pages, writing
/// `[user_vaddr, phys]` to `out_ptr`. The caller has already charged the
/// process's `Untyped` budget; returns `None` (so the caller refunds) on any
/// failure.
fn dma_alloc_charged(pages: u64, out_ptr: u64) -> Option<()> {
    let phys = frame::alloc_contiguous(pages)?;
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
        _ => return None,
    }
    for i in 0..pages {
        if !arch::map_user(va_base + i * PAGE, phys + i * PAGE, true, false) {
            return None;
        }
    }
    *next = va_base + pages * PAGE;
    drop(next);

    let buf = user_slice_mut(out_ptr, 16)?;
    buf[0..8].copy_from_slice(&va_base.to_le_bytes());
    buf[8..16].copy_from_slice(&phys.to_le_bytes());
    Some(())
}

/// Read an I/O port — but only if the calling process holds an `IoPort`
/// capability covering it. Returns the value, or `u64::MAX` if unauthorized.
fn sys_port_in(port: u16, size: u8) -> u64 {
    if !crate::process::current_authorizes_port(port, size) {
        println!("[cap] DENY port_in 0x{port:x} (no IoPort capability)");
        return u64::MAX;
    }
    u64::from(arch::port_in(port, size))
}

/// Write an I/O port, gated by an `IoPort` capability. Returns 0 on success,
/// `u64::MAX` if unauthorized (the write does not happen).
fn sys_port_out(port: u16, size: u8, value: u32) -> u64 {
    if !crate::process::current_authorizes_port(port, size) {
        println!("[cap] DENY port_out 0x{port:x} (no IoPort capability)");
        return u64::MAX;
    }
    arch::port_out(port, size, value);
    0
}

/// Identify the capability in the caller's CNode slot `slot`, writing the
/// normalized `[type, a, b]` triple to `out_ptr`. Returns 0 on success, or
/// `u64::MAX` if the slot is empty/out of range or the buffer is bad. A process
/// can only inspect its OWN capabilities — there is no global view.
fn sys_cap_identify(slot: u64, out_ptr: u64) -> u64 {
    let Some((ty, a, b)) = crate::process::current_cap_describe(slot as usize) else {
        return u64::MAX;
    };
    let Some(buf) = user_slice_mut(out_ptr, 24) else {
        return u64::MAX;
    };
    buf[0..8].copy_from_slice(&u64::from(ty).to_le_bytes());
    buf[8..16].copy_from_slice(&a.to_le_bytes());
    buf[16..24].copy_from_slice(&b.to_le_bytes());
    0
}

/// Send `word` (and optionally the capability in `cap_slot`) over the endpoint
/// named by the `Endpoint` cap in `ep_slot`. Non-blocking — the message waits in
/// the endpoint queue for a receiver.
fn sys_send(ep_slot: u64, word: u64, cap_slot: u64) -> u64 {
    let Some(id) = crate::process::current_endpoint_id(ep_slot as usize) else {
        println!("[cap] DENY send (no Endpoint capability in slot {ep_slot})");
        return u64::MAX;
    };
    let cap = if cap_slot == u64::MAX {
        None
    } else {
        match crate::process::current_cap_get(cap_slot as usize) {
            Some(c) => Some(c),
            None => return u64::MAX, // nothing to grant from that slot
        }
    };
    if crate::endpoint::send(id as usize, word, cap) {
        0
    } else {
        u64::MAX
    }
}

/// Receive one message from the endpoint named by the `Endpoint` cap in
/// `ep_slot`, blocking (by yielding the CPU) until one arrives. Writes the
/// message word to `out_ptr` and installs any granted capability into
/// `dst_slot` (or discards it if `dst_slot == u64::MAX`).
fn sys_recv(ep_slot: u64, out_ptr: u64, dst_slot: u64) -> u64 {
    let Some(id) = crate::process::current_endpoint_id(ep_slot as usize) else {
        println!("[cap] DENY recv (no Endpoint capability in slot {ep_slot})");
        return u64::MAX;
    };
    loop {
        if let Some((word, cap)) = crate::endpoint::try_recv(id as usize) {
            if let Some(c) = cap {
                if dst_slot != u64::MAX && !crate::process::current_cap_install(dst_slot as usize, c)
                {
                    return u64::MAX; // destination slot occupied/invalid
                }
            }
            let Some(buf) = user_slice_mut(out_ptr, 8) else {
                return u64::MAX;
            };
            buf.copy_from_slice(&word.to_le_bytes());
            return 0;
        }
        // Nothing yet — let other processes (including the sender) run.
        crate::process::yield_now();
    }
}

/// Signal the notification named by the `Notification` cap in `notif_slot`,
/// OR-ing `bits` into its word. Non-blocking. Returns 0, or `u64::MAX` if the cap
/// is missing.
fn sys_signal(notif_slot: u64, bits: u64) -> u64 {
    let Some(id) = crate::process::current_notification_id(notif_slot as usize) else {
        println!("[cap] DENY signal (no Notification capability in slot {notif_slot})");
        return u64::MAX;
    };
    if crate::notification::signal(id as usize, bits) {
        0
    } else {
        u64::MAX
    }
}

/// Wait on the notification named by the `Notification` cap in `notif_slot`,
/// blocking (by yielding the CPU) until its bits are non-zero, then returning and
/// clearing them. Returns the bits (non-zero), or 0 if the cap is missing.
fn sys_wait(notif_slot: u64) -> u64 {
    let Some(id) = crate::process::current_notification_id(notif_slot as usize) else {
        println!("[cap] DENY wait (no Notification capability in slot {notif_slot})");
        return 0;
    };
    loop {
        if let Some(bits) = crate::notification::poll_take(id as usize) {
            return bits;
        }
        crate::process::yield_now();
    }
}

/// Reserve `pages` of the shared-frame VA window and map the physical run
/// `[phys, phys + pages*PAGE)` there in the CURRENT address space (cached, RW,
/// NX). Returns the base user virtual address, or `None` if the window is full
/// or a mapping fails. Shared by `SYS_FRAME_ALLOC` (its own fresh frames) and
/// `SYS_MAP_FRAME` (a delegated frame) — the same physical run mapped into a
/// second address space is exactly shared memory.
fn map_shared(phys: u64, pages: u64) -> Option<u64> {
    let mut next = NEXT_SHARED_VA.lock();
    let va_base = *next;
    match va_base.checked_add(pages * PAGE) {
        Some(end) if end <= SHARED_REGION_END => {}
        _ => return None,
    }
    for i in 0..pages {
        if !arch::map_user(va_base + i * PAGE, phys + i * PAGE, true, false) {
            return None;
        }
    }
    *next = va_base + pages * PAGE;
    Some(va_base)
}

/// Allocate `pages` contiguous frames, map them into the caller as shared
/// memory, install a `Frame` cap naming them into `cap_slot`, and write the
/// mapped virtual address to `out_ptr`. Bounded by the caller's `Untyped`
/// budget. On any failure after allocation the frames are freed and the budget
/// refunded.
fn sys_frame_alloc(pages: u64, cap_slot: u64, out_ptr: u64) -> u64 {
    if pages == 0 || pages > MAX_SHARED_PAGES {
        return u64::MAX;
    }
    let need = pages * PAGE;
    if !crate::process::current_charge_untyped(need) {
        println!("[cap] DENY frame_alloc {need:#x} bytes (Untyped budget exhausted)");
        return u64::MAX;
    }

    let unwind = |phys: Option<u64>| {
        if let Some(p) = phys {
            for i in 0..pages {
                frame::free(p + i * PAGE);
            }
        }
        crate::process::current_refund_untyped(need);
        u64::MAX
    };

    let Some(phys) = frame::alloc_contiguous(pages) else {
        return unwind(None);
    };
    // Zero the shared frames through the HHDM before any process sees them.
    // SAFETY: freshly allocated contiguous frames, reachable via the HHDM.
    unsafe {
        core::ptr::write_bytes((phys + arch::hhdm_offset()) as *mut u8, 0, need as usize);
    }
    let Some(va) = map_shared(phys, pages) else {
        return unwind(Some(phys));
    };
    let Some(buf) = user_slice_mut(out_ptr, 8) else {
        return unwind(Some(phys));
    };
    buf.copy_from_slice(&va.to_le_bytes());
    // Install the Frame cap last: only now does the caller gain a name it can
    // delegate. A bad/occupied slot is a caller error — unwind it.
    if !crate::process::current_cap_install(cap_slot as usize, crate::cap::CapEntry::frame(phys, pages))
    {
        return unwind(Some(phys));
    }
    0
}

/// Map a delegated `Frame` capability (in `cap_slot`) into the caller and write
/// the mapped virtual address to `out_ptr`. No budget charge — the frame is
/// already accounted to whoever allocated it; mapping it here is precisely how
/// the memory becomes shared.
fn sys_map_frame(cap_slot: u64, out_ptr: u64) -> u64 {
    let Some((phys, pages)) = crate::process::current_frame_cap(cap_slot as usize) else {
        println!("[cap] DENY map_frame (no Frame capability in slot {cap_slot})");
        return u64::MAX;
    };
    let Some(va) = map_shared(phys, pages) else {
        return u64::MAX;
    };
    let Some(buf) = user_slice_mut(out_ptr, 8) else {
        return u64::MAX;
    };
    buf.copy_from_slice(&va.to_le_bytes());
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

