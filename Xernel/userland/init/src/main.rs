#![no_std]
#![no_main]

//! `init` — Xernel's first user program, demonstrating the bring-up syscall ABI.
//!
//! It is a freestanding (no_std, no alloc) ring-3 program. It talks to the
//! kernel only through `syscall`. This build prints a small boot banner and
//! some real system information queried from the kernel, then exits — a tiny
//! "first OS" you can grow.
//!
//! Syscall ABI: number in `rax`, args in `rdi, rsi, rdx`, return in `rax`.
//! `syscall` clobbers `rcx` and `r11`.

use core::arch::asm;

const SYS_WRITE: u64 = 1;
const SYS_EXIT: u64 = 2;
const SYS_GET_TICKS: u64 = 4;
const SYS_SYSINFO: u64 = 5;
const SYS_SBRK: u64 = 8;
const SYS_FB_INFO: u64 = 9;
const SYS_GETPID: u64 = 10;
const SYS_YIELD: u64 = 11;
const SYS_PCI_READ: u64 = 12;
const SYS_IOMAP: u64 = 13;
const SYS_DMA_ALLOC: u64 = 14;

const STDOUT: u64 = 1;
const INFO_RAM_TOTAL: u64 = 0;
const INFO_RAM_USED: u64 = 1;

#[inline]
fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    // SAFETY: Xernel syscall ABI; rcx/r11 are clobbered by the instruction.
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") nr => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    // SAFETY: Xernel syscall ABI; arg 4 goes in r10 (rcx is clobbered by syscall).
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") nr => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

fn write(bytes: &[u8]) {
    syscall3(SYS_WRITE, STDOUT, bytes.as_ptr() as u64, bytes.len() as u64);
}

/// Print `v` as `digits` lowercase hex digits (no `0x` prefix).
fn print_hex(mut v: u64, digits: usize) {
    let mut buf = [0u8; 16];
    let mut i = digits;
    while i > 0 {
        i -= 1;
        let nib = (v & 0xF) as u8;
        buf[i] = if nib < 10 { b'0' + nib } else { b'a' + nib - 10 };
        v >>= 4;
    }
    write(&buf[..digits]);
}

fn pci_read(bus: u64, dev: u64, func: u64, offset: u64) -> u32 {
    syscall4(SYS_PCI_READ, bus, dev, func, offset) as u32
}

/// Scan PCI bus 0 from user space and report devices. Returns the slot of the
/// first virtio device (vendor 0x1AF4), or 0xFF if none.
fn pci_scan() -> u64 {
    print(" PCI-Scan (Bus 0):\n");
    let mut virtio = 0xFFu64;
    for dev in 0..32u64 {
        let id = pci_read(0, dev, 0, 0); // offset 0: vendor | device<<16
        let vendor = (id & 0xFFFF) as u16;
        if vendor == 0xFFFF {
            continue; // no device in this slot
        }
        let device = (id >> 16) as u16;
        print("   dev ");
        print_u64(dev);
        print(": vendor 0x");
        print_hex(u64::from(vendor), 4);
        print(" device 0x");
        print_hex(u64::from(device), 4);
        if vendor == 0x1af4 {
            print("   <- VIRTIO");
            if virtio == 0xFF {
                virtio = dev;
            }
        }
        print("\n");
    }
    virtio
}

fn iomap(phys: u64, len: u64) -> u64 {
    syscall3(SYS_IOMAP, phys, len, 0)
}

/// Allocate a DMA buffer; returns (user_vaddr, phys_addr) or (u64::MAX, 0).
fn dma_alloc(len: u64) -> (u64, u64) {
    let mut out = [0u64; 2];
    if syscall3(SYS_DMA_ALLOC, len, out.as_mut_ptr() as u64, 0) == u64::MAX {
        return (u64::MAX, 0);
    }
    (out[0], out[1])
}

/// Allocate a DMA buffer, write a pattern and read it back — proof that a
/// user-space driver has physically-contiguous memory it can hand to a device.
fn dma_demo() {
    let (va, phys) = dma_alloc(4096);
    if va == u64::MAX {
        print(" DMA: alloc FEHLER\n");
        return;
    }
    print(" DMA: 4 KiB @ user 0x");
    print_hex(va, 8);
    print(" phys 0x");
    print_hex(phys, 8);
    let p = va as *mut u64;
    let mut ok = true;
    // SAFETY: the kernel mapped [va, va+4096) as user read/write memory.
    unsafe {
        for i in 0..512u64 {
            p.add(i as usize).write_volatile(0xDEAD_0000 + i);
        }
        for i in 0..512u64 {
            if p.add(i as usize).read_volatile() != 0xDEAD_0000 + i {
                ok = false;
                break;
            }
        }
    }
    print(if ok { "  ok\n" } else { "  VERIFY FEHLER\n" });
}

/// Map the first memory BAR of `dev` into our address space and read a register
/// — proof that a user-space driver can reach device MMIO.
fn iomap_demo(dev: u64) {
    print(" MMIO-Map (dev ");
    print_u64(dev);
    print("):\n");
    let mut i = 0u64;
    while i < 6 {
        let bar = pci_read(0, dev, 0, (0x10 + i * 4) as u64);
        if bar == 0 || (bar & 1) == 1 {
            i += 1; // empty slot or I/O BAR (we want memory)
            continue;
        }
        let is_64 = ((bar >> 1) & 3) == 2;
        let mut base = u64::from(bar & 0xFFFF_FFF0);
        if is_64 {
            base |= u64::from(pci_read(0, dev, 0, (0x10 + (i + 1) * 4) as u64)) << 32;
        }
        if base == 0 {
            i += if is_64 { 2 } else { 1 };
            continue;
        }
        print("   BAR");
        print_u64(i);
        print(" phys 0x");
        print_hex(base, 8);
        let va = iomap(base, 0x1000);
        if va == u64::MAX {
            print("  -> iomap FEHLER\n");
            return;
        }
        print("  -> user 0x");
        print_hex(va, 8);
        // SAFETY: the kernel just mapped this page as uncached device memory.
        let val = unsafe { (va as *const u32).read_volatile() };
        print(", [0]=0x");
        print_hex(u64::from(val), 8);
        print("\n");
        return;
    }
    print("   keine Memory-BAR\n");
}

fn print(s: &str) {
    write(s.as_bytes());
}

/// Print a u64 in base 10 without any allocation.
fn print_u64(mut n: u64) {
    let mut buf = [0u8; 20];
    if n == 0 {
        write(b"0");
        return;
    }
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    write(&buf[i..]);
}

fn sysinfo(which: u64) -> u64 {
    syscall3(SYS_SYSINFO, which, 0, 0)
}

fn sbrk(delta: i64) -> u64 {
    syscall3(SYS_SBRK, delta as u64, 0, 0)
}

/// Request `n` bytes of heap, write a byte pattern across it, read it back, and
/// report whether it survived — an end-to-end check of the SYS_SBRK path.
fn heap_check(n: usize) {
    let base = sbrk(n as i64);
    if base == u64::MAX {
        print(" heap: sbrk FAILED\n");
        return;
    }
    let p = base as *mut u8;
    let mut ok = true;
    // SAFETY: the kernel just mapped [base, base+n) as user read/write memory.
    unsafe {
        for i in 0..n {
            p.add(i).write_volatile((i & 0xff) as u8);
        }
        for i in 0..n {
            if p.add(i).read_volatile() != (i & 0xff) as u8 {
                ok = false;
                break;
            }
        }
    }
    if ok {
        print(" heap      : ");
        print_u64(n as u64 / 1024);
        print(" KiB ok\n");
    } else {
        print(" heap      : VERIFY FAILED\n");
    }
}

fn ticks() -> u64 {
    syscall3(SYS_GET_TICKS, 0, 0, 0)
}

fn getpid() -> u64 {
    syscall3(SYS_GETPID, 0, 0, 0)
}

fn yield_now() {
    syscall3(SYS_YIELD, 0, 0, 0);
}

fn exit(code: u64) -> ! {
    syscall3(SYS_EXIT, code, 0, 0);
    loop {
        core::hint::spin_loop();
    }
}

/// Query the framebuffer and, if present, paint a colour gradient across it —
/// a visible proof that user-space graphics work. Reports the resolution.
fn fb_demo() {
    let mut info = [0u64; 5];
    if syscall3(SYS_FB_INFO, info.as_mut_ptr() as u64, 0, 0) == u64::MAX {
        print(" fb        : none\n");
        return;
    }
    let [addr, width, height, pitch, _bpp] = info;
    let stride = (pitch / 4) as usize; // 32 bpp -> pixels per row
    let fb = addr as *mut u32;
    let (w, h) = (width as usize, height as usize);
    // SAFETY: the kernel mapped [addr, addr + height*pitch) as user-writable
    // device memory for this framebuffer.
    unsafe {
        for y in 0..h {
            for x in 0..w {
                let red = (x * 255 / w) as u32;
                let green = (y * 255 / h) as u32;
                let blue = 0x40u32;
                fb.add(y * stride + x)
                    .write_volatile((red << 16) | (green << 8) | blue);
            }
        }
    }
    print(" fb        : ");
    print_u64(width);
    print("x");
    print_u64(height);
    print(" ok\n");
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Exactly one init process now (the kernel no longer launches copies).
    let pid = getpid();

    print("\n[init pid ");
    print_u64(pid);
    print("] hello — eigener Adressraum, eigener Heap\n");

    heap_check(8192);

    print("  __  __                    _ \n");
    print(" |  \\/  | ___ _ __ _ __  ___| |\n");
    print(" | |\\/| |/ _ \\ '__| '_ \\/ -_) |   Xernel OS\n");
    print(" |_|  |_|\\___/_|  |_| |_\\___|_|\n");

    // Framebuffer: now mapped into THIS process's address space (per-process),
    // so writing pixels works in any process — not just the first caller.
    fb_demo();
    let vdev = pci_scan();
    if vdev != 0xFF {
        iomap_demo(vdev);
    }
    dma_demo();

    let _ = yield_now; // cooperative yield available for programs that want it
    print("[init] fertig\n");
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
