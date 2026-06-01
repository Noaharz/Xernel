//! Global Descriptor Table + Task State Segment.
//!
//! We run a flat segmentation model (segments cover the whole address space);
//! the GDT exists only because long mode still requires valid code/data
//! selectors and a TSS. The TSS carries:
//!   - Interrupt Stack Table (IST) entries, so double faults and page faults
//!     switch to known-good stacks even if the kernel stack is corrupt.
//!   - `privilege_stack_table[0]` (RSP0), the stack the CPU switches to when an
//!     interrupt is taken while running in ring 3. Without it, the first timer
//!     IRQ during a user program triple-faults.
//!
//! The descriptor order (kernel-code, kernel-data, user-data, user-code) is
//! exactly what `syscall`/`sysret` expect; see `super::syscall`.

use core::ptr::addr_of;

use spin::Once;
use x86_64::instructions::segmentation::{Segment, CS, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub const PAGE_FAULT_IST_INDEX: u16 = 1;

const IST_STACK_SIZE: usize = 4096 * 5;
const KERNEL_STACK_SIZE: usize = 4096 * 5;

static mut DF_STACK: [u8; IST_STACK_SIZE] = [0; IST_STACK_SIZE];
static mut PF_STACK: [u8; IST_STACK_SIZE] = [0; IST_STACK_SIZE];
/// Stack the CPU switches to on a ring 3 -> ring 0 interrupt (TSS RSP0).
static mut RSP0_STACK: [u8; KERNEL_STACK_SIZE] = [0; KERNEL_STACK_SIZE];

#[derive(Clone, Copy)]
pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_code: SegmentSelector,
    pub user_data: SegmentSelector,
    pub tss: SegmentSelector,
}

// The TSS is `static mut` because RSP0 (the kernel stack used on ring 3 -> ring
// 0 interrupts) is updated per process by the scheduler. Single-CPU, so the
// only writer is `set_rsp0`; the CPU reads RSP0 by hardware via the descriptor.
static mut TSS: TaskStateSegment = TaskStateSegment::new();
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();

/// Top-of-stack address for a stack array. x86 stacks grow downward, so this
/// points one past the end of the backing array.
fn stack_top(stack: *const u8, size: usize) -> VirtAddr {
    VirtAddr::from_ptr(stack) + size as u64
}

/// The default kernel stack for ring 3 -> ring 0 interrupt transitions (RSP0),
/// used until the scheduler assigns each process its own.
pub fn rsp0() -> VirtAddr {
    stack_top(addr_of!(RSP0_STACK).cast(), KERNEL_STACK_SIZE)
}

/// Set the kernel stack the CPU switches to on a ring 3 -> ring 0 interrupt
/// (TSS RSP0). Called by the scheduler so each process is preempted onto its
/// own kernel stack.
pub fn set_rsp0(top: u64) {
    // SAFETY: single-CPU; this is the only writer of RSP0, and a 64-bit aligned
    // write is not torn from the CPU's point of view.
    unsafe {
        // The TSS is `packed(4)`, so the field is not 8-byte aligned -> unaligned.
        core::ptr::addr_of_mut!(TSS.privilege_stack_table[0]).write_unaligned(VirtAddr::new(top));
    }
}

pub fn selectors() -> Selectors {
    GDT.get().expect("gdt not initialised").1
}

pub fn init() {
    // SAFETY: init runs once before anything reads the TSS; we set the IST
    // stacks and the default RSP0 via raw-pointer writes (no `&mut` to static).
    unsafe {
        // TSS is `packed(4)`; its VirtAddr fields are unaligned -> write_unaligned.
        core::ptr::addr_of_mut!(TSS.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize])
            .write_unaligned(stack_top(addr_of!(DF_STACK).cast(), IST_STACK_SIZE));
        core::ptr::addr_of_mut!(TSS.interrupt_stack_table[PAGE_FAULT_IST_INDEX as usize])
            .write_unaligned(stack_top(addr_of!(PF_STACK).cast(), IST_STACK_SIZE));
        core::ptr::addr_of_mut!(TSS.privilege_stack_table[0]).write_unaligned(rsp0());
    }

    let (gdt, selectors) = GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();
        // Order matters for syscall/sysret: kernel code, kernel data, then user
        // data, user code. Do not reorder.
        let kernel_code = gdt.append(Descriptor::kernel_code_segment());
        let kernel_data = gdt.append(Descriptor::kernel_data_segment());
        let user_data = gdt.append(Descriptor::user_data_segment());
        let user_code = gdt.append(Descriptor::user_code_segment());
        // The descriptor only captures the TSS base address at build time; no
        // live reference to the (mutable) TSS is kept afterward.
        // SAFETY: TSS is initialised above and lives for the whole kernel.
        let tss_ref: &'static TaskStateSegment = unsafe { &*core::ptr::addr_of!(TSS) };
        let tss = gdt.append(Descriptor::tss_segment(tss_ref));
        (
            gdt,
            Selectors {
                kernel_code,
                kernel_data,
                user_code,
                user_data,
                tss,
            },
        )
    });

    gdt.load();
    // SAFETY: the selectors index descriptors in the GDT we just loaded, and
    // the TSS is a valid 'static object with initialised IST/RSP0 stacks.
    unsafe {
        CS::set_reg(selectors.kernel_code);
        SS::set_reg(selectors.kernel_data);
        load_tss(selectors.tss);
    }
}
