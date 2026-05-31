//! PS/2 keyboard driver.
//!
//! The 8042 PS/2 controller raises IRQ 1 on each key event. We route that line
//! through the IO-APIC to a LAPIC interrupt vector; the handler reads the raw
//! scancode from port 0x60, translates make-codes (scancode set 1) to ASCII,
//! and pushes the result into a ring buffer. [`read`] drains that buffer,
//! blocking (via `sti; hlt`) until at least one byte is available.
//!
//! Scope: US layout, lowercase only — no shift/caps/ctrl handling yet. Enough
//! for a first interactive shell; modifiers are a later refinement.

use spin::Mutex;
use x86_64::instructions::interrupts;
use x86_64::instructions::port::Port;
use x86_64::structures::idt::InterruptStackFrame;

use super::paging;

/// LAPIC vector the keyboard IRQ is delivered on.
pub const KEYBOARD_VECTOR: u8 = 0x21;

// QEMU q35 / standard PC IO-APIC MMIO base, and the virtual address we map it
// to. (A real ACPI MADT parse would discover this; hard-coding is fine for the
// platforms we target during bring-up.)
const IOAPIC_PHYS: u64 = 0xFEC0_0000;
const IOAPIC_VIRT: u64 = 0xffff_9300_0000_0000;
// Redirection-table registers for GSI 1 (the keyboard line).
const IOREDTBL1_LOW: u32 = 0x12;
const IOREDTBL1_HIGH: u32 = 0x13;

const RING_CAP: usize = 256;

struct Ring {
    data: [u8; RING_CAP],
    head: usize,
    tail: usize,
}

impl Ring {
    const fn new() -> Self {
        Self {
            data: [0; RING_CAP],
            head: 0,
            tail: 0,
        }
    }
    /// Push a byte; silently drops it if the buffer is full.
    fn push(&mut self, b: u8) {
        let next = (self.tail + 1) % RING_CAP;
        if next != self.head {
            self.data[self.tail] = b;
            self.tail = next;
        }
    }
    fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            return None;
        }
        let b = self.data[self.head];
        self.head = (self.head + 1) % RING_CAP;
        Some(b)
    }
}

static BUFFER: Mutex<Ring> = Mutex::new(Ring::new());

/// Scancode set 1 make-code -> ASCII (US layout, lowercase). 0 = no character.
#[rustfmt::skip]
const SC1: [u8; 0x3A] = [
    0,    0,    b'1', b'2', b'3', b'4', b'5', b'6',
    b'7', b'8', b'9', b'0', b'-', b'=', 0x08, b'\t',
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i',
    b'o', b'p', b'[', b']', b'\n', 0,   b'a', b's',
    b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';',
    b'\'',b'`', 0,    b'\\',b'z', b'x', b'c', b'v',
    b'b', b'n', b'm', b',', b'.', b'/', 0,    b'*',
    0,    b' ',
];

/// Translate a raw scancode to ASCII. Returns `None` for release (break) codes
/// and keys we don't map.
fn translate(scancode: u8) -> Option<u8> {
    let sc = scancode as usize;
    if sc < SC1.len() && SC1[sc] != 0 {
        Some(SC1[sc])
    } else {
        None
    }
}

fn ioapic_write(reg: u32, value: u32) {
    // SAFETY: IOAPIC MMIO is mapped at IOAPIC_VIRT (uncached). IOREGSEL is at
    // offset 0, IOWIN at offset 0x10 (4 u32s further).
    unsafe {
        let base = IOAPIC_VIRT as *mut u32;
        base.write_volatile(reg);
        base.add(4).write_volatile(value);
    }
}

/// Map the IO-APIC and unmask the keyboard line, delivering it to
/// [`KEYBOARD_VECTOR`] on the boot CPU (APIC id 0). Call after the LAPIC is up.
pub fn init() {
    paging::map_mmio(IOAPIC_VIRT, IOAPIC_PHYS).expect("map IO-APIC MMIO");
    // High dword: destination APIC id 0 in bits 24..32.
    ioapic_write(IOREDTBL1_HIGH, 0);
    // Low dword: vector, fixed delivery, physical dest, edge, active-high,
    // unmasked (all those fields are 0 here besides the vector).
    ioapic_write(IOREDTBL1_LOW, u32::from(KEYBOARD_VECTOR));
}

pub extern "x86-interrupt" fn irq_handler(_frame: InterruptStackFrame) {
    // SAFETY: 0x60 is the PS/2 data port; reading it consumes one scancode.
    let scancode: u8 = unsafe { Port::new(0x60).read() };
    if let Some(ascii) = translate(scancode) {
        BUFFER.lock().push(ascii);
    }
    super::apic::eoi();
}

/// Fill `buf` with available keyboard bytes, blocking until at least one is
/// ready. Returns the number of bytes written.
pub fn read(buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    loop {
        // Drain with interrupts off so the IRQ handler can't deadlock against
        // us on the buffer lock.
        let n = interrupts::without_interrupts(|| {
            let mut ring = BUFFER.lock();
            let mut n = 0;
            while n < buf.len() {
                match ring.pop() {
                    Some(b) => {
                        buf[n] = b;
                        n += 1;
                    }
                    None => break,
                }
            }
            n
        });
        if n > 0 {
            return n;
        }
        // Nothing buffered: enable interrupts and idle until the next one.
        interrupts::enable_and_hlt();
    }
}

/// Non-blocking variant of [`read`]: drains whatever is buffered (possibly
/// nothing) and returns immediately. Useful for input polling during
/// animation/idle loops.
pub fn read_nb(buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    interrupts::without_interrupts(|| {
        let mut ring = BUFFER.lock();
        let mut n = 0;
        while n < buf.len() {
            match ring.pop() {
                Some(b) => {
                    buf[n] = b;
                    n += 1;
                }
                None => break,
            }
        }
        n
    })
}

/// In-kernel self-test: push synthetic scancodes through the translate + ring
/// path and confirm they come back out as the expected ASCII. Does not touch
/// real hardware or block. Returns `true` on success.
pub fn selftest() -> bool {
    // "hi\n": scancodes 0x23='h', 0x17='i', 0x1C='\n'.
    {
        let mut ring = BUFFER.lock();
        for sc in [0x23u8, 0x17, 0x1C] {
            if let Some(b) = translate(sc) {
                ring.push(b);
            }
        }
    }
    let mut out = [0u8; 8];
    let n = interrupts::without_interrupts(|| {
        let mut ring = BUFFER.lock();
        let mut n = 0;
        while let Some(b) = ring.pop() {
            out[n] = b;
            n += 1;
        }
        n
    });
    &out[..n] == b"hi\n"
}
