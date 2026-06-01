//! Local APIC + legacy PIC shutdown + LAPIC timer.
//!
//! We don't use the legacy 8259 PICs at all — they get remapped out of the
//! exception range and fully masked. Interrupts are delivered through the Local
//! APIC. The LAPIC timer provides the periodic tick that will later drive
//! preemptive scheduling; for now it just advances [`ticks`].

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Once;
use x86_64::instructions::port::Port;
use x86_64::registers::model_specific::Msr;
use x86_64::structures::idt::InterruptStackFrame;

use super::paging;

pub const TIMER_VECTOR: u8 = 0x20;
pub const SPURIOUS_VECTOR: u8 = 0xff;

/// Fixed virtual address we map the LAPIC MMIO window to.
const LAPIC_VIRT: u64 = 0xffff_9200_0000_0000;

// LAPIC register offsets.
const REG_TPR: u64 = 0x80;
const REG_EOI: u64 = 0xb0;
const REG_SVR: u64 = 0xf0;
const REG_LVT_TIMER: u64 = 0x320;
const REG_TIMER_INIT: u64 = 0x380;
const REG_TIMER_DIV: u64 = 0x3e0;

const LVT_TIMER_PERIODIC: u32 = 1 << 17;
const SVR_ENABLE: u32 = 1 << 8;
const IA32_APIC_BASE: u32 = 0x1b;

static LAPIC: Once<u64> = Once::new();
static TICKS: AtomicU64 = AtomicU64::new(0);

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

fn write_reg(offset: u64, value: u32) {
    let addr = (LAPIC.get().copied().unwrap_or(LAPIC_VIRT) + offset) as *mut u32;
    // SAFETY: `addr` is inside the mapped LAPIC MMIO page; 32-bit aligned.
    unsafe { addr.write_volatile(value) };
}

pub fn eoi() {
    write_reg(REG_EOI, 0);
}

/// Remap both 8259 PICs out of the CPU exception vectors and mask every line.
fn disable_pic() {
    // SAFETY: standard 8259 initialisation sequence on the legacy PIC ports.
    unsafe {
        let mut pic1_cmd = Port::<u8>::new(0x20);
        let mut pic1_data = Port::<u8>::new(0x21);
        let mut pic2_cmd = Port::<u8>::new(0xa0);
        let mut pic2_data = Port::<u8>::new(0xa1);
        let mut wait_port = Port::<u8>::new(0x80);
        let mut io_wait = || wait_port.write(0);

        pic1_cmd.write(0x11);
        io_wait();
        pic2_cmd.write(0x11);
        io_wait();
        pic1_data.write(0x20); // PIC1 vector offset 32
        io_wait();
        pic2_data.write(0x28); // PIC2 vector offset 40
        io_wait();
        pic1_data.write(0x04); // tell PIC1 about PIC2 at IRQ2
        io_wait();
        pic2_data.write(0x02);
        io_wait();
        pic1_data.write(0x01); // 8086 mode
        io_wait();
        pic2_data.write(0x01);
        io_wait();
        pic1_data.write(0xff); // mask everything
        pic2_data.write(0xff);
    }
}

pub fn init() {
    disable_pic();

    // SAFETY: reading IA32_APIC_BASE is safe on any APIC-capable CPU (all our
    // targets). We keep the base, set the global-enable bit, and write it back.
    let base_phys = unsafe {
        let mut msr = Msr::new(IA32_APIC_BASE);
        let val = msr.read();
        msr.write(val | (1 << 11));
        val & 0xffff_f000
    };

    paging::map_mmio(LAPIC_VIRT, base_phys).expect("map LAPIC MMIO");
    LAPIC.call_once(|| LAPIC_VIRT);

    write_reg(REG_TPR, 0);
    write_reg(REG_SVR, SVR_ENABLE | u32::from(SPURIOUS_VECTOR));

    write_reg(REG_TIMER_DIV, 0x3); // divide by 16
    write_reg(REG_LVT_TIMER, u32::from(TIMER_VECTOR) | LVT_TIMER_PERIODIC);
    write_reg(REG_TIMER_INIT, 1_000_000);
}

/// Tick the clock and acknowledge the interrupt.
extern "C" fn timer_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    eoi();
}

/// Tick, acknowledge, and (since we interrupted user mode) preempt: hand the CPU
/// to the next ready process. Returns when this process is scheduled again.
extern "C" fn timer_preempt() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    eoi();
    crate::process::yield_now();
}

/// Naked timer interrupt entry. If it interrupted ring 0 (we were in the kernel,
/// e.g. a syscall), it only ticks + EOIs — kernel code is not preempted. If it
/// interrupted ring 3, it `swapgs`es to the kernel GS, saves the full register
/// context on this process's kernel stack, and preempts via the scheduler;
/// `swapgs` + `iretq` resume whichever process runs next.
#[unsafe(naked)]
pub unsafe extern "C" fn timer_isr() {
    core::arch::naked_asm!(
        "test byte ptr [rsp + 8], 3", // CS & 3 != 0  ->  came from ring 3
        "jnz 2f",
        // ---- from ring 0: tick + EOI only (no preempt, no swapgs) ----
        "push rax", "push rcx", "push rdx", "push rsi", "push rdi",
        "push r8", "push r9", "push r10", "push r11",
        "call {tick}",
        "pop r11", "pop r10", "pop r9", "pop r8", "pop rdi",
        "pop rsi", "pop rdx", "pop rcx", "pop rax",
        "iretq",
        // ---- from ring 3: full save + preempt ----
        "2:",
        "swapgs",
        "push rax", "push rbx", "push rcx", "push rdx", "push rsi",
        "push rdi", "push rbp", "push r8", "push r9", "push r10",
        "push r11", "push r12", "push r13", "push r14", "push r15",
        "call {preempt}",
        "pop r15", "pop r14", "pop r13", "pop r12", "pop r11",
        "pop r10", "pop r9", "pop r8", "pop rbp", "pop rdi",
        "pop rsi", "pop rdx", "pop rcx", "pop rbx", "pop rax",
        "swapgs",
        "iretq",
        tick = sym timer_tick,
        preempt = sym timer_preempt,
    )
}

pub extern "x86-interrupt" fn spurious_handler(_frame: InterruptStackFrame) {
    eoi();
}
