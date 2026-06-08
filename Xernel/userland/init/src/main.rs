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
const SYS_PORT_IN: u64 = 15;
const SYS_PORT_OUT: u64 = 16;
const SYS_CAP_IDENTIFY: u64 = 17;
const SYS_SEND: u64 = 18;
const SYS_RECV: u64 = 19;
const SYS_SPAWN: u64 = 20;

/// Endpoint capability slots (seeded by the kernel): `EP_SLOT` carries requests
/// from a client to the file-service, `REPLY_EP_SLOT` carries replies back.
/// `NO_CAP` is the send/recv sentinel for "no capability".
const EP_SLOT: u64 = 3;
const REPLY_EP_SLOT: u64 = 4;
const NO_CAP: u64 = u64::MAX;

/// File-service protocol. A request is one `u64`: the opcode in the top byte,
/// its argument in the low 56 bits. Each request gets exactly one `u64` reply.
const OP_BYE: u64 = 0; // stop serving (no reply)
const OP_NFILES: u64 = 1; // -> number of files
const OP_FSIZE: u64 = 2; // arg = index            -> file size in bytes
const OP_NAMECH: u64 = 3; // arg = index<<8 | pos   -> one byte of the name
const OP_DATACH: u64 = 4; // arg = index<<16 | off  -> one byte of the contents

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

/// Find the PCI bus-0 slot of the virtio device with device-id `devid`
/// (0x1001 = block, 0x1000 = network), or 0xFF if absent. Lets us pick a
/// specific virtio device now that several are present.
fn pci_find_virtio(devid: u16) -> u64 {
    for dev in 0..32u64 {
        let id = pci_read(0, dev, 0, 0);
        if (id & 0xFFFF) as u16 == 0x1af4 && (id >> 16) as u16 == devid {
            return dev;
        }
    }
    0xFF
}

fn iomap(phys: u64, len: u64) -> u64 {
    syscall3(SYS_IOMAP, phys, len, 0)
}

fn port_in(port: u16, size: u64) -> u32 {
    syscall3(SYS_PORT_IN, u64::from(port), size, 0) as u32
}

fn port_out(port: u16, size: u64, value: u32) {
    syscall3(SYS_PORT_OUT, u64::from(port), size, u64::from(value));
}

// --- Legacy virtio-blk register offsets (relative to the I/O BAR base) ---
const VIO_DEVICE_FEATURES: u16 = 0x00; // u32, R
const VIO_GUEST_FEATURES: u16 = 0x04; // u32, W
const VIO_QUEUE_PFN: u16 = 0x08; // u32, RW (phys >> 12)
const VIO_QUEUE_SIZE: u16 = 0x0C; // u16, R
const VIO_QUEUE_SELECT: u16 = 0x0E; // u16, W
const VIO_QUEUE_NOTIFY: u16 = 0x10; // u16, W
const VIO_STATUS: u16 = 0x12; // u8, RW
const VIO_CONFIG: u16 = 0x14; // device config begins here (no MSI-X)

// Device-status bits.
const ST_ACK: u32 = 1;
const ST_DRIVER: u32 = 2;
const ST_DRIVER_OK: u32 = 4;

// Descriptor flags + request type.
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2; // device writes into this buffer
const BLK_T_IN: u32 = 0; // read
const BLK_T_OUT: u32 = 1; // write

const QALIGN: u64 = 4096;

fn align_up(x: u64, a: u64) -> u64 {
    (x + a - 1) & !(a - 1)
}

/// A brought-up virtio-blk device: virtqueue 0 is ready and request buffers are
/// allocated. Reused across requests by `blk_rw` — the user-space block layer.
struct Blk {
    io: u16,
    q_va: u64,
    used_off: u64,
    n: u64,
    avail_va: u64,
    hdr_va: u64,
    hdr_phys: u64,
    data_va: u64,
    data_phys: u64,
    status_va: u64,
    status_phys: u64,
    seq: u16, // number of requests submitted so far (= expected used.idx)
}

/// Bring up virtio-blk device `dev`: status handshake, feature negotiation,
/// virtqueue 0 layout and request-buffer allocation. Returns a handle for
/// `blk_rw`, or `None` on failure. Prints the device capacity.
fn blk_init(dev: u64) -> Option<Blk> {
    let bar0 = pci_read(0, dev, 0, 0x10);
    if (bar0 & 1) != 1 {
        print(" virtio-blk: BAR0 ist kein I/O-Port\n");
        return None;
    }
    let io = (bar0 & 0xFFFC) as u16; // legacy virtio register block
    print(" virtio-blk @ I/O 0x");
    print_hex(u64::from(io), 4);
    print("\n");

    // Status handshake: reset -> ACKNOWLEDGE -> DRIVER.
    port_out(io + VIO_STATUS, 1, 0);
    port_out(io + VIO_STATUS, 1, ST_ACK);
    port_out(io + VIO_STATUS, 1, ST_ACK | ST_DRIVER);

    // Capacity (device config, u64 count of 512-byte sectors).
    let lo = u64::from(port_in(io + VIO_CONFIG, 4));
    let hi = u64::from(port_in(io + VIO_CONFIG + 4, 4));
    let sectors = lo | (hi << 32);
    print(" Kapazität: ");
    print_u64(sectors);
    print(" Sektoren (");
    print_u64(sectors * 512 / 1024);
    print(" KiB)\n");

    // Feature negotiation: we need nothing fancy -> accept none.
    let _devf = port_in(io + VIO_DEVICE_FEATURES, 4);
    port_out(io + VIO_GUEST_FEATURES, 4, 0);

    // Virtqueue 0: read its device-fixed size, lay out the legacy vring
    // (desc | avail | pad | used) in one contiguous DMA buffer.
    port_out(io + VIO_QUEUE_SELECT, 2, 0);
    let n = u64::from(port_in(io + VIO_QUEUE_SIZE, 2));
    if n == 0 {
        print(" virtio-blk: queue 0 nicht vorhanden\n");
        return None;
    }
    let desc_sz = 16 * n;
    let used_off = align_up(desc_sz + 6 + 2 * n, QALIGN);
    let total = used_off + 6 + 8 * n;

    let (q_va, q_phys) = dma_alloc(total);
    if q_va == u64::MAX {
        print(" virtio-blk: queue-DMA FEHLER\n");
        return None;
    }
    port_out(io + VIO_QUEUE_PFN, 4, (q_phys >> 12) as u32);
    port_out(io + VIO_STATUS, 1, ST_ACK | ST_DRIVER | ST_DRIVER_OK);

    // Request buffers: header (16) | data (512) | status (1), one DMA page.
    let (b_va, b_phys) = dma_alloc(4096);
    if b_va == u64::MAX {
        print(" virtio-blk: req-DMA FEHLER\n");
        return None;
    }
    Some(Blk {
        io,
        q_va,
        used_off,
        n,
        avail_va: q_va + desc_sz,
        hdr_va: b_va,
        hdr_phys: b_phys,
        data_va: b_va + 16,
        data_phys: b_phys + 16,
        status_va: b_va + 16 + 512,
        status_phys: b_phys + 16 + 512,
        seq: 0,
    })
}

/// Transfer one 512-byte sector to/from the device: read (`write = false`) or
/// write (`write = true`) sector `sector` through the 512-byte buffer `buf`.
/// Returns `true` on success. Polls the used ring for completion (no IRQ).
fn blk_rw(b: &mut Blk, sector: u64, write: bool, buf: *mut u8) -> bool {
    // SAFETY: every address below is mapped DMA/user memory set up by blk_init;
    // the buffers are physically contiguous for the device to DMA.
    unsafe {
        // Request header: type, reserved, sector.
        (b.hdr_va as *mut u32).write_volatile(if write { BLK_T_OUT } else { BLK_T_IN });
        ((b.hdr_va + 4) as *mut u32).write_volatile(0);
        ((b.hdr_va + 8) as *mut u64).write_volatile(sector);
        (b.status_va as *mut u8).write_volatile(0xFF); // sentinel

        // For a write we fill the data buffer (device READS it); for a read the
        // device WRITES into it (DESC_WRITE).
        if write {
            for i in 0..512usize {
                ((b.data_va + i as u64) as *mut u8).write_volatile(*buf.add(i));
            }
        }
        let data_flags = DESC_NEXT | if write { 0 } else { DESC_WRITE };
        write_desc(b.q_va, 0, b.hdr_phys, 16, DESC_NEXT, 1);
        write_desc(b.q_va, 1, b.data_phys, 512, data_flags, 2);
        write_desc(b.q_va, 2, b.status_phys, 1, DESC_WRITE, 0);

        // Publish descriptor head 0 into the next available-ring slot, bump idx.
        let slot = u64::from(b.seq) % b.n;
        ((b.avail_va + 4 + 2 * slot) as *mut u16).write_volatile(0);
        b.seq = b.seq.wrapping_add(1);
        ((b.avail_va + 2) as *mut u16).write_volatile(b.seq);
    }

    port_out(b.io + VIO_QUEUE_NOTIFY, 2, 0);

    // Poll the used ring until it catches up to our submitted count.
    let used_idx = (b.q_va + b.used_off + 2) as *const u16;
    let mut spins = 0u64;
    loop {
        // SAFETY: used-ring index lives in our mapped queue buffer.
        if unsafe { used_idx.read_volatile() } == b.seq {
            break;
        }
        spins += 1;
        if spins > 200_000_000 {
            print(" virtio-blk: TIMEOUT\n");
            return false;
        }
    }

    // SAFETY: completion is signalled; status (and data, for a read) are valid.
    let st = unsafe { (b.status_va as *const u8).read_volatile() };
    if st != 0 {
        print(" virtio-blk: Status ");
        print_u64(u64::from(st));
        print(" (Fehler)\n");
        return false;
    }
    if !write {
        // SAFETY: the device wrote 512 bytes into the data buffer.
        unsafe {
            for i in 0..512usize {
                *buf.add(i) = ((b.data_va + i as u64) as *const u8).read_volatile();
            }
        }
    }
    true
}

// --- XernelFS: a minimal on-disk filesystem on top of the block layer ---
//
// Layout (512-byte sectors, 1 MiB disk = 2048 sectors):
//   sector 0           reserved (boot/magic — the FS never touches it)
//   sector 1           superblock: magic | version | total_sectors | next_free
//   sector 2           directory: 16 entries × 32 bytes
//   sectors 3..        data region (files stored contiguously)
//
// A directory entry: name[24] (NUL-padded) | size:u32 | start_sector:u32.
// Flat namespace, bump allocation, no delete-reclaim — deliberately tiny. It is
// pure Ring-3 code over `blk_rw`; the kernel knows nothing about files.
const SECTOR: usize = 512;
const FS_MAGIC: &[u8; 8] = b"XERNFS01";
const SB_SECTOR: u64 = 1;
const DIR_SECTOR: u64 = 2;
const DATA_START: u64 = 3;
const TOTAL_SECTORS: u64 = 2048;
const DIR_ENTRIES: usize = 16;
const ENT_SIZE: usize = 32;
const NAME_LEN: usize = 24;

fn rd_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn wr_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// Write a fresh filesystem: superblock + empty directory. Returns false on I/O
/// error.
fn fs_format(b: &mut Blk) -> bool {
    let mut sb = [0u8; SECTOR];
    sb[..8].copy_from_slice(FS_MAGIC);
    wr_u32(&mut sb, 8, 1); // version
    wr_u32(&mut sb, 12, TOTAL_SECTORS as u32);
    wr_u32(&mut sb, 16, DATA_START as u32); // next_free
    let mut dir = [0u8; SECTOR];
    blk_rw(b, SB_SECTOR, true, sb.as_mut_ptr()) && blk_rw(b, DIR_SECTOR, true, dir.as_mut_ptr())
}

/// Create a file `name` with contents `data`. Bump-allocates contiguous data
/// sectors. Returns false if the directory or disk is full, or on I/O error.
fn fs_create(b: &mut Blk, name: &[u8], data: &[u8]) -> bool {
    let mut sb = [0u8; SECTOR];
    if !blk_rw(b, SB_SECTOR, false, sb.as_mut_ptr()) || &sb[..8] != FS_MAGIC {
        return false;
    }
    let mut next_free = u64::from(rd_u32(&sb, 16));

    let mut dir = [0u8; SECTOR];
    if !blk_rw(b, DIR_SECTOR, false, dir.as_mut_ptr()) {
        return false;
    }
    let Some(slot) = (0..DIR_ENTRIES).find(|&i| dir[i * ENT_SIZE] == 0) else {
        return false; // directory full
    };

    let nsec = data.len().div_ceil(SECTOR) as u64;
    if next_free + nsec > TOTAL_SECTORS {
        return false; // disk full
    }
    let start = next_free;

    // Write the data, zero-padding the final sector.
    let mut off = 0;
    let mut s = start;
    while off < data.len() {
        let mut secbuf = [0u8; SECTOR];
        let n = core::cmp::min(SECTOR, data.len() - off);
        secbuf[..n].copy_from_slice(&data[off..off + n]);
        if !blk_rw(b, s, true, secbuf.as_mut_ptr()) {
            return false;
        }
        off += n;
        s += 1;
    }

    // Fill the directory entry.
    let base = slot * ENT_SIZE;
    let nl = core::cmp::min(NAME_LEN, name.len());
    dir[base..base + nl].copy_from_slice(&name[..nl]);
    wr_u32(&mut dir, base + 24, data.len() as u32);
    wr_u32(&mut dir, base + 28, start as u32);
    if !blk_rw(b, DIR_SECTOR, true, dir.as_mut_ptr()) {
        return false;
    }

    // Commit the bumped free pointer.
    next_free += nsec;
    wr_u32(&mut sb, 16, next_free as u32);
    blk_rw(b, SB_SECTOR, true, sb.as_mut_ptr())
}

/// Look up `name` in a loaded directory sector. Returns (size, start_sector).
fn fs_find(dir: &[u8], name: &[u8]) -> Option<(u32, u32)> {
    (0..DIR_ENTRIES).find_map(|i| {
        let base = i * ENT_SIZE;
        if dir[base] == 0 {
            return None;
        }
        let matches = (0..NAME_LEN).all(|x| {
            let want = if x < name.len() { name[x] } else { 0 };
            dir[base + x] == want
        });
        matches.then(|| (rd_u32(dir, base + 24), rd_u32(dir, base + 28)))
    })
}

/// Read file `name` into `out`. Returns the file size (bytes copied is capped at
/// `out.len()`), or None if not found / on I/O error.
fn fs_read(b: &mut Blk, name: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut dir = [0u8; SECTOR];
    if !blk_rw(b, DIR_SECTOR, false, dir.as_mut_ptr()) {
        return None;
    }
    let (size, start) = fs_find(&dir, name)?;
    let size = size as usize;
    let mut off = 0;
    let mut s = u64::from(start);
    while off < size {
        let mut secbuf = [0u8; SECTOR];
        if !blk_rw(b, s, false, secbuf.as_mut_ptr()) {
            return None;
        }
        let n = core::cmp::min(SECTOR, size - off);
        if off + n <= out.len() {
            out[off..off + n].copy_from_slice(&secbuf[..n]);
        }
        off += n;
        s += 1;
    }
    Some(size)
}

/// Print every file in the directory with its size.
fn fs_list(b: &mut Blk) {
    let mut dir = [0u8; SECTOR];
    if !blk_rw(b, DIR_SECTOR, false, dir.as_mut_ptr()) {
        print(" fs: list FEHLER\n");
        return;
    }
    print(" Dateien:\n");
    for i in 0..DIR_ENTRIES {
        let base = i * ENT_SIZE;
        if dir[base] == 0 {
            continue;
        }
        print("   ");
        for &c in dir[base..base + NAME_LEN].iter() {
            if c == 0 {
                break;
            }
            write(&[c]);
        }
        print("  (");
        print_u64(u64::from(rd_u32(&dir, base + 24)));
        print(" B)\n");
    }
}

/// Prepare the disk the file-service will serve: format it and create a couple
/// of files, then print the catalogue. Returns false on any I/O error.
fn fs_setup(b: &mut Blk) -> bool {
    print(" XernelFS: formatiere und befuelle Disk\n");
    if !fs_format(b) {
        print("   format FEHLER\n");
        return false;
    }
    if !(fs_create(b, b"hallo.txt", b"Hallo von XernelFS!")
        && fs_create(b, b"readme", b"Xernel filesystem v1 - flach, 16 Dateien."))
    {
        print("   create FEHLER\n");
        return false;
    }
    fs_list(b);
    true
}

/// Answer one file-service request: do the real disk I/O the client cannot do
/// itself (it holds no device authority) and return a single `u64` result.
fn serve_one(b: &mut Blk, op: u64, arg: u64) -> u64 {
    let mut dir = [0u8; SECTOR];
    if !blk_rw(b, DIR_SECTOR, false, dir.as_mut_ptr()) {
        return u64::MAX;
    }
    match op {
        // Number of files = non-empty directory entries.
        OP_NFILES => (0..DIR_ENTRIES).filter(|&i| dir[i * ENT_SIZE] != 0).count() as u64,
        // Size of file `arg`.
        OP_FSIZE => {
            let i = arg as usize;
            if i >= DIR_ENTRIES || dir[i * ENT_SIZE] == 0 {
                u64::MAX
            } else {
                u64::from(rd_u32(&dir, i * ENT_SIZE + 24))
            }
        }
        // One byte of file `arg>>8`'s name at position `arg & 0xFF`.
        OP_NAMECH => {
            let i = (arg >> 8) as usize;
            let pos = (arg & 0xFF) as usize;
            if i >= DIR_ENTRIES || pos >= NAME_LEN || dir[i * ENT_SIZE] == 0 {
                0
            } else {
                u64::from(dir[i * ENT_SIZE + pos])
            }
        }
        // One byte of file `arg>>16`'s contents at offset `arg & 0xFFFF`. Reads
        // the file through the existing read path, keyed by the entry's name.
        OP_DATACH => {
            let i = (arg >> 16) as usize;
            let off = (arg & 0xFFFF) as usize;
            if i >= DIR_ENTRIES || dir[i * ENT_SIZE] == 0 {
                return 0;
            }
            let mut name = [0u8; NAME_LEN];
            name.copy_from_slice(&dir[i * ENT_SIZE..i * ENT_SIZE + NAME_LEN]);
            let mut buf = [0u8; SECTOR];
            match fs_read(b, &name, &mut buf) {
                Some(size) if off < size && off < buf.len() => u64::from(buf[off]),
                _ => 0,
            }
        }
        _ => u64::MAX,
    }
}

/// The file-service loop: receive a request on the request endpoint, do the disk
/// I/O, reply on the reply endpoint. Returns when a client says goodbye. This is
/// Xernel's first real microkernel server — a filesystem living in its OWN
/// process, reachable only by message-passing.
fn file_service(b: &mut Blk) {
    print(" Datei-Service: bereit, warte auf Anfragen\n");
    loop {
        let mut req = 0u64;
        if ipc_recv(EP_SLOT, &mut req, NO_CAP) != 0 {
            print(" Datei-Service: recv FEHLER\n");
            return;
        }
        let op = req >> 56;
        let arg = req & ((1u64 << 56) - 1);
        if op == OP_BYE {
            print(" Datei-Service: Abschied — beende\n");
            return;
        }
        let reply = serve_one(b, op, arg);
        if ipc_send(REPLY_EP_SLOT, reply, NO_CAP) != 0 {
            print(" Datei-Service: send FEHLER\n");
            return;
        }
    }
}

// --- virtio-net: a user-space NIC driver (first networking) ---
//
// Same legacy virtio register block as virtio-blk, but two virtqueues: queue 0
// is the receiveq (device writes incoming frames), queue 1 the transmitq. With
// no features negotiated, each buffer is prefixed by a 10-byte virtio_net_hdr.
// We bring the device up, send an ARP request for the SLIRP gateway 10.0.2.2,
// and read back the ARP reply — a real packet exchange, entirely in Ring 3.

const VNET_RX: u16 = 0; // receiveq index
const VNET_TX: u16 = 1; // transmitq index
const VNET_HDR_LEN: u64 = 10; // legacy virtio_net_hdr, no MRG_RXBUF
const NET_BUF: u64 = 2048; // per-direction packet buffer

/// A brought-up virtio-net device: both virtqueues are laid out and one receive
/// buffer is posted. Holds enough to transmit a frame and poll for a reply.
struct Net {
    io: u16,
    mac: [u8; 6],
    rx_ring: u64,
    rx_used_off: u64,
    rx_buf_va: u64,
    tx_ring: u64,
    tx_used_off: u64,
    tx_avail: u64,
    tx_buf_va: u64,
    tx_buf_phys: u64,
}

/// Lay out one legacy virtqueue `idx` in a fresh DMA buffer and tell the device
/// its page frame. Returns (ring_va, used_off, n, avail_va).
fn vq_setup(io: u16, idx: u16) -> Option<(u64, u64, u64, u64)> {
    port_out(io + VIO_QUEUE_SELECT, 2, u32::from(idx));
    let n = u64::from(port_in(io + VIO_QUEUE_SIZE, 2));
    if n == 0 {
        return None;
    }
    let desc_sz = 16 * n;
    let used_off = align_up(desc_sz + 6 + 2 * n, QALIGN);
    let total = used_off + 6 + 8 * n;
    let (q_va, q_phys) = dma_alloc(total);
    if q_va == u64::MAX {
        return None;
    }
    port_out(io + VIO_QUEUE_PFN, 4, (q_phys >> 12) as u32);
    Some((q_va, used_off, n, q_va + desc_sz))
}

/// Bring up virtio-net device `dev`: status handshake, read the MAC, lay out the
/// receive and transmit queues, post one receive buffer, go live. Returns a
/// handle, or `None` on failure.
fn net_init(dev: u64) -> Option<Net> {
    let bar0 = pci_read(0, dev, 0, 0x10);
    if (bar0 & 1) != 1 {
        print(" virtio-net: BAR0 ist kein I/O-Port\n");
        return None;
    }
    let io = (bar0 & 0xFFFC) as u16;

    port_out(io + VIO_STATUS, 1, 0);
    port_out(io + VIO_STATUS, 1, ST_ACK);
    port_out(io + VIO_STATUS, 1, ST_ACK | ST_DRIVER);

    // Accept no optional features -> plain 10-byte header, no offloads.
    let _ = port_in(io + VIO_DEVICE_FEATURES, 4);
    port_out(io + VIO_GUEST_FEATURES, 4, 0);

    // Device config begins at VIO_CONFIG; the first 6 bytes are the MAC.
    let mut mac = [0u8; 6];
    for (i, b) in mac.iter_mut().enumerate() {
        *b = port_in(io + VIO_CONFIG + i as u16, 1) as u8;
    }

    // Receive queue: lay it out and post one buffer the device can write into.
    let (rx_ring, rx_used_off, _rx_n, rx_avail) = vq_setup(io, VNET_RX)?;
    let (rx_buf_va, rx_buf_phys) = dma_alloc(NET_BUF);
    if rx_buf_va == u64::MAX {
        return None;
    }
    // SAFETY: rx_ring/rx_avail point into our mapped DMA queue buffer.
    unsafe {
        write_desc(rx_ring, 0, rx_buf_phys, NET_BUF as u32, DESC_WRITE, 0);
        ((rx_avail + 4) as *mut u16).write_volatile(0); // ring[0] -> desc 0
        ((rx_avail + 2) as *mut u16).write_volatile(1); // avail.idx = 1
    }

    // Transmit queue: laid out, buffer allocated, filled per send.
    let (tx_ring, tx_used_off, _tx_n, tx_avail) = vq_setup(io, VNET_TX)?;
    let (tx_buf_va, tx_buf_phys) = dma_alloc(NET_BUF);
    if tx_buf_va == u64::MAX {
        return None;
    }

    port_out(io + VIO_STATUS, 1, ST_ACK | ST_DRIVER | ST_DRIVER_OK);
    port_out(io + VIO_QUEUE_NOTIFY, 2, u32::from(VNET_RX)); // RX buffer is ready

    print(" virtio-net @ I/O 0x");
    print_hex(u64::from(io), 4);
    print(", MAC ");
    print_mac(&mac);
    print("\n");
    Some(Net {
        io,
        mac,
        rx_ring,
        rx_used_off,
        rx_buf_va,
        tx_ring,
        tx_used_off,
        tx_avail,
        tx_buf_va,
        tx_buf_phys,
    })
}

/// Write one byte to a DMA buffer.
///
/// # Safety
/// `va` must be a mapped, writable DMA address.
unsafe fn wr_u8(va: u64, v: u8) {
    (va as *mut u8).write_volatile(v);
}

/// Print a 6-byte MAC address as `xx:xx:xx:xx:xx:xx`.
fn print_mac(mac: &[u8; 6]) {
    for (i, b) in mac.iter().enumerate() {
        if i > 0 {
            print(":");
        }
        print_hex(u64::from(*b), 2);
    }
}

/// Send an ARP request for the SLIRP gateway 10.0.2.2 and read the reply — a
/// real round-trip on the network, driven entirely from Ring 3. Prints the
/// gateway's hardware address on success.
fn net_arp_demo(n: &mut Net) {
    let our_ip = [10u8, 0, 2, 15];
    let gw_ip = [10u8, 0, 2, 2];

    // Build virtio_net_hdr (zeros) + Ethernet header + ARP request.
    // SAFETY: every address is inside our mapped TX DMA buffer.
    unsafe {
        let base = n.tx_buf_va;
        for i in 0..VNET_HDR_LEN {
            wr_u8(base + i, 0);
        }
        let eth = base + VNET_HDR_LEN;
        for i in 0..6 {
            wr_u8(eth + i, 0xFF); // dst = broadcast
            wr_u8(eth + 6 + i, n.mac[i as usize]); // src = us
        }
        wr_u8(eth + 12, 0x08);
        wr_u8(eth + 13, 0x06); // ethertype ARP
        let arp = eth + 14;
        // htype=1, ptype=0x0800, hlen=6, plen=4, op=1 (request)
        for (i, b) in [0u8, 1, 0x08, 0x00, 6, 4, 0, 1].iter().enumerate() {
            wr_u8(arp + i as u64, *b);
        }
        for i in 0..6 {
            wr_u8(arp + 8 + i, n.mac[i as usize]); // sender HW
            wr_u8(arp + 18 + i, 0); // target HW = unknown
        }
        for i in 0..4 {
            wr_u8(arp + 14 + i, our_ip[i as usize]); // sender IP
            wr_u8(arp + 24 + i, gw_ip[i as usize]); // target IP
        }
        let total = VNET_HDR_LEN + 14 + 28;
        write_desc(n.tx_ring, 0, n.tx_buf_phys, total as u32, 0, 0);
        ((n.tx_avail + 4) as *mut u16).write_volatile(0);
        ((n.tx_avail + 2) as *mut u16).write_volatile(1);
    }
    port_out(n.io + VIO_QUEUE_NOTIFY, 2, u32::from(VNET_TX));

    // Poll the transmit queue's used ring for completion.
    let tx_used = (n.tx_ring + n.tx_used_off + 2) as *const u16;
    let mut spins = 0u64;
    while unsafe { tx_used.read_volatile() } != 1 {
        spins += 1;
        if spins > 200_000_000 {
            print(" net: TX-Timeout\n");
            return;
        }
    }
    print(" net: ARP-Request gesendet (wer hat 10.0.2.2?)\n");

    // Poll the receive queue for the gateway's reply.
    let rx_used = (n.rx_ring + n.rx_used_off + 2) as *const u16;
    let mut spins = 0u64;
    while unsafe { rx_used.read_volatile() } != 1 {
        spins += 1;
        if spins > 800_000_000 {
            print(" net: keine Antwort (RX-Timeout)\n");
            return;
        }
    }

    // Skip the virtio-net header; parse the Ethernet frame.
    let pkt = n.rx_buf_va + VNET_HDR_LEN;
    let et = (unsafe { (pkt + 12) as *const u8 }, unsafe {
        (pkt + 13) as *const u8
    });
    let is_arp = unsafe { et.0.read_volatile() } == 0x08 && unsafe { et.1.read_volatile() } == 0x06;
    if is_arp {
        let mut smac = [0u8; 6];
        for (i, b) in smac.iter_mut().enumerate() {
            // ARP sender hardware address = Ethernet payload + 8.
            *b = unsafe { ((pkt + 14 + 8 + i as u64) as *const u8).read_volatile() };
        }
        print(" net: ARP-Antwort von 10.0.2.2 -> MAC ");
        print_mac(&smac);
        print(" — Netzwerk lebt!\n");
    } else {
        print(" net: Frame empfangen, aber kein ARP\n");
    }
}

/// Write one virtqueue descriptor at index `i` of the table starting at `base`.
///
/// # Safety
/// `base` must point at a mapped descriptor table with room for index `i`.
unsafe fn write_desc(base: u64, i: u64, addr: u64, len: u32, flags: u16, next: u16) {
    let d = base + i * 16;
    (d as *mut u64).write_volatile(addr);
    ((d + 8) as *mut u32).write_volatile(len);
    ((d + 12) as *mut u16).write_volatile(flags);
    ((d + 14) as *mut u16).write_volatile(next);
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

/// Human-readable name for a capability type tag (see `xabi::cap::CapType`).
fn cap_type_name(ty: u64) -> &'static str {
    match ty {
        1 => "Untyped",
        2 => "CNode",
        3 => "Frame",
        7 => "Endpoint",
        8 => "Notification",
        9 => "IrqHandler",
        10 => "IoPort",
        11 => "IoMem",
        _ => "?",
    }
}

/// Enumerate this process's own capability table and print each capability it
/// holds — the process discovering exactly the authority it was granted. No
/// global view exists: a process can only inspect its OWN CNode.
fn cap_list() {
    print(" Eigene Capabilities:\n");
    for slot in 0..8u64 {
        let mut out = [0u64; 3];
        if syscall3(SYS_CAP_IDENTIFY, slot, out.as_mut_ptr() as u64, 0) == u64::MAX {
            continue; // empty slot
        }
        let [ty, a, b] = out;
        print("   slot ");
        print_u64(slot);
        print(": ");
        print(cap_type_name(ty));
        match ty {
            10 => {
                // IoPort: a = base, b = count
                print("   base 0x");
                print_hex(a, 4);
                print(" count 0x");
                print_hex(b, 4);
            }
            11 => {
                // IoMem: a = base, b = len
                print("    base 0x");
                print_hex(a, 8);
                print(" len 0x");
                print_hex(b, 8);
            }
            1 => {
                // Untyped: a = remaining budget
                print("  budget ");
                print_u64(a);
                print(" bytes");
            }
            _ => {
                print("  0x");
                print_hex(a, 8);
                print(" 0x");
                print_hex(b, 8);
            }
        }
        print("\n");
    }
}

/// Demonstrate that hardware authority is capability-gated. The virtio-blk
/// driver above only worked because this process holds an `IoPort` capability
/// covering the PCI I/O window. A port OUTSIDE that range — here CMOS/RTC at
/// 0x70 — must be refused by the kernel, even though the call is identical.
fn cap_demo() {
    print(" Capability-Check:\n");
    // IoPort: CMOS/RTC at 0x70 is outside our granted port window.
    let r = syscall3(SYS_PORT_IN, 0x70, 1, 0);
    if r == u64::MAX {
        print("   port_in(0x70)    -> VERWEIGERT (keine IoPort-Cap) — korrekt\n");
    } else {
        print("   port_in(0x70)    -> 0x");
        print_hex(r, 2);
        print("  (FEHLER: haette gesperrt sein muessen)\n");
    }
    // IoMem: phys 0x100000 is real RAM/kernel, outside the PCI MMIO window —
    // a driver must not be able to map arbitrary physical memory.
    let m = iomap(0x10_0000, 0x1000);
    if m == u64::MAX {
        print("   iomap(0x100000)  -> VERWEIGERT (keine IoMem-Cap) — korrekt\n");
    } else {
        print("   iomap(0x100000)  -> 0x");
        print_hex(m, 8);
        print("  (FEHLER: haette gesperrt sein muessen)\n");
    }
    // Untyped: a 1 MiB DMA request exceeds our whole memory budget — a driver
    // must not be able to pin unbounded physical memory.
    let mut out = [0u64; 2];
    let d = syscall3(SYS_DMA_ALLOC, 1 << 20, out.as_mut_ptr() as u64, 0);
    if d == u64::MAX {
        print("   dma(1 MiB)       -> VERWEIGERT (Untyped-Budget) — korrekt\n");
    } else {
        print("   dma(1 MiB)       -> 0x");
        print_hex(out[0], 8);
        print("  (FEHLER: haette gesperrt sein muessen)\n");
    }
}

/// Send `word` (and optionally a capability) over the endpoint in `ep_slot`.
fn ipc_send(ep_slot: u64, word: u64, cap_slot: u64) -> u64 {
    syscall3(SYS_SEND, ep_slot, word, cap_slot)
}

/// Block until a message arrives on the endpoint in `ep_slot`; the message word
/// is written to `*out`. A granted capability lands in `dst_slot`.
fn ipc_recv(ep_slot: u64, out: *mut u64, dst_slot: u64) -> u64 {
    syscall3(SYS_RECV, ep_slot, out as u64, dst_slot)
}

/// Spawn a new process from program image `module` (0 = the init image). Returns
/// the new PID, or `u64::MAX` on failure. The kernel boots only the root; the
/// root creates every other process itself through this call.
fn spawn(module: u64) -> u64 {
    syscall3(SYS_SPAWN, module, 0, 0)
}

/// Client helper: send one request to the file-service and block for its
/// one-word reply. The client holds no device authority — every answer comes
/// from the service doing the disk work on its behalf.
fn request(op: u64, arg: u64) -> u64 {
    if ipc_send(EP_SLOT, (op << 56) | arg, NO_CAP) != 0 {
        return u64::MAX;
    }
    let mut r = 0u64;
    if ipc_recv(REPLY_EP_SLOT, &mut r, NO_CAP) != 0 {
        return u64::MAX;
    }
    r
}

/// The spawned client (pid != 0): it holds ONLY endpoint capabilities, no device
/// authority — so it cannot touch the disk. It reads the filesystem entirely by
/// asking the file-service over IPC: it learns the file count, reconstructs each
/// name and size, then pulls one file's contents back byte by byte. The visible
/// proof that a client gets filesystem service without any hardware capability.
/// Never returns.
fn file_client() -> ! {
    // Prove we have no device authority: a port the service may touch, we cannot.
    print("[Client] eigene Geraete-Autoritaet? Port 0xc000: ");
    if syscall3(SYS_PORT_IN, 0xc000, 4, 0) == u64::MAX {
        print("VERWEIGERT (korrekt — nur Endpoint-Caps)\n");
    } else {
        print("erlaubt (?!)\n");
    }

    let n = request(OP_NFILES, 0);
    print("[Client] Datei-Service meldet ");
    print_u64(n);
    print(" Dateien — alles via IPC, ohne Disk-Autoritaet:\n");
    for i in 0..n {
        print("   ");
        for pos in 0..(NAME_LEN as u64) {
            let c = request(OP_NAMECH, (i << 8) | pos) as u8;
            if c == 0 {
                break;
            }
            write(&[c]);
        }
        print("  (");
        print_u64(request(OP_FSIZE, i));
        print(" B)\n");
    }

    // Pull file 0's contents back byte by byte, purely through the service.
    let size0 = request(OP_FSIZE, 0);
    if size0 != u64::MAX {
        print("[Client] lese Datei 0 ueber den Service: \"");
        for off in 0..size0 {
            let c = request(OP_DATACH, off) as u8; // index 0: arg = off
            if c == 0 {
                break;
            }
            write(&[c]);
        }
        print("\"\n");
    }

    // Tell the service to stop — goodbye expects no reply.
    ipc_send(EP_SLOT, OP_BYE << 56, NO_CAP);
    print("[Client] fertig\n");
    exit(0);
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
    // The kernel boots only ONE copy of this binary: the root (pid 0). The root
    // brings up the disk, becomes a FILE-SERVICE, and SPAWNS a client itself
    // (like a real init). Each copy takes a role by its PID: pid 0 is the
    // service/driver host below; any other pid is a client that holds no device
    // authority and reaches the filesystem only by asking the service over IPC.
    let pid = getpid();

    print("\n[init pid ");
    print_u64(pid);
    print("] hello — eigener Adressraum, eigener Heap\n");

    if pid != 0 {
        file_client(); // never returns
    }

    heap_check(8192);

    print("  __  __                    _ \n");
    print(" |  \\/  | ___ _ __ _ __  ___| |\n");
    print(" | |\\/| |/ _ \\ '__| '_ \\/ -_) |   Xernel OS\n");
    print(" |_|  |_|\\___/_|  |_| |_\\___|_|\n");

    // Framebuffer: now mapped into THIS process's address space (per-process),
    // so writing pixels works in any process — not just the first caller.
    fb_demo();
    pci_scan(); // print the bus; pick specific devices by id below
    let bdev = pci_find_virtio(0x1001); // virtio-blk
    let mut service_blk = None;
    if bdev != 0xFF {
        iomap_demo(bdev);
        service_blk = blk_init(bdev);
    }
    dma_demo();
    cap_list();
    cap_demo();

    // Networking: bring up the virtio-net NIC and do a real ARP exchange with
    // the gateway — proof that a user-space driver can talk to the network.
    let ndev = pci_find_virtio(0x1000); // virtio-net
    if ndev != 0xFF {
        if let Some(mut net) = net_init(ndev) {
            net_arp_demo(&mut net);
        }
    } else {
        print(" kein virtio-net gefunden\n");
    }

    // Become the file-service: format the disk, spawn a client that has NO
    // device authority, then answer its file requests over IPC — the first
    // Xernel service living in its own process.
    if let Some(mut blk) = service_blk {
        if fs_setup(&mut blk) {
            let client = spawn(0);
            if client == u64::MAX {
                print(" spawn: FEHLER\n");
            } else {
                print(" spawn: Datei-Client erzeugt, pid ");
                print_u64(client);
                print("\n");
            }
            file_service(&mut blk);
        }
    } else {
        print(" kein virtio-blk — kein Datei-Service\n");
    }

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
