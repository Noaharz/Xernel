//! Interrupt Descriptor Table and CPU-exception handlers.
//!
//! All CPU exceptions get a handler so a fault prints a readable dump on the
//! serial console instead of silently triple-faulting. Double-fault and
//! page-fault handlers run on dedicated IST stacks (see [`super::gdt`]).
//!
//! Device IRQs (vectors 32+) are routed via the APIC and registered in
//! [`super::apic`]; this module owns only the architectural exception vectors.

use spin::Once;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use x86_64::registers::control::Cr2;

use super::gdt;

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub fn init() {
    let idt = IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        idt.divide_error.set_handler_fn(divide_error);
        idt.debug.set_handler_fn(debug);
        idt.non_maskable_interrupt.set_handler_fn(nmi);
        idt.breakpoint.set_handler_fn(breakpoint);
        idt.overflow.set_handler_fn(overflow);
        idt.bound_range_exceeded.set_handler_fn(bound_range);
        idt.invalid_opcode.set_handler_fn(invalid_opcode);
        idt.device_not_available.set_handler_fn(device_na);
        idt.invalid_tss.set_handler_fn(invalid_tss);
        idt.segment_not_present.set_handler_fn(segment_not_present);
        idt.stack_segment_fault.set_handler_fn(stack_segment);
        idt.general_protection_fault.set_handler_fn(gpf);
        idt.page_fault.set_handler_fn(page_fault);
        idt.x87_floating_point.set_handler_fn(x87);
        idt.alignment_check.set_handler_fn(alignment);
        idt.simd_floating_point.set_handler_fn(simd);

        // Device IRQs delivered via the LAPIC. The timer uses a naked entry
        // (full context save + preemption), so it is installed by raw address.
        // SAFETY: `timer_isr` is a valid interrupt entry that saves/restores all
        // state and ends in `iretq`.
        unsafe {
            idt[super::apic::TIMER_VECTOR]
                .set_handler_addr(x86_64::VirtAddr::new(super::apic::timer_isr as *const () as u64));
        }
        idt[super::apic::SPURIOUS_VECTOR].set_handler_fn(super::apic::spurious_handler);
        idt[super::keyboard::KEYBOARD_VECTOR].set_handler_fn(super::keyboard::irq_handler);

        // SAFETY: these IST indices are configured with valid stacks in the TSS
        // before the IDT is loaded.
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
            idt.page_fault
                .set_handler_fn(page_fault)
                .set_stack_index(gdt::PAGE_FAULT_IST_INDEX);
        }
        idt
    });
    idt.load();
}

macro_rules! trap {
    ($name:ident, $msg:literal) => {
        extern "x86-interrupt" fn $name(frame: InterruptStackFrame) {
            panic!("CPU exception: {}\n{:#?}", $msg, frame);
        }
    };
}

macro_rules! trap_err {
    ($name:ident, $msg:literal) => {
        extern "x86-interrupt" fn $name(frame: InterruptStackFrame, code: u64) {
            panic!("CPU exception: {} (error code {:#x})\n{:#?}", $msg, code, frame);
        }
    };
}

trap!(divide_error, "#DE divide error");
trap!(debug, "#DB debug");
trap!(nmi, "NMI");
trap!(overflow, "#OF overflow");
trap!(bound_range, "#BR bound range exceeded");
trap!(invalid_opcode, "#UD invalid opcode");
trap!(device_na, "#NM device not available");
trap!(x87, "#MF x87 floating point");
trap!(simd, "#XM SIMD floating point");
trap_err!(invalid_tss, "#TS invalid TSS");
trap_err!(segment_not_present, "#NP segment not present");
trap_err!(stack_segment, "#SS stack-segment fault");
trap_err!(gpf, "#GP general protection fault");
trap_err!(alignment, "#AC alignment check");

extern "x86-interrupt" fn breakpoint(frame: InterruptStackFrame) {
    // Non-fatal: log and continue. Useful as a sanity check (`int3`).
    crate::println!("[xernel] #BP breakpoint at {:#x}", frame.instruction_pointer);
}

extern "x86-interrupt" fn double_fault(frame: InterruptStackFrame, code: u64) -> ! {
    panic!("CPU exception: #DF double fault (error code {code:#x})\n{frame:#?}");
}

extern "x86-interrupt" fn page_fault(frame: InterruptStackFrame, code: PageFaultErrorCode) {
    let addr = Cr2::read().map(|a| a.as_u64()).unwrap_or(u64::MAX);
    panic!("CPU exception: #PF page fault at {addr:#x}, code {code:?}\n{frame:#?}");
}
