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

static TSS: Once<TaskStateSegment> = Once::new();
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();

/// Top-of-stack address for a stack array. x86 stacks grow downward, so this
/// points one past the end of the backing array.
fn stack_top(stack: *const u8, size: usize) -> VirtAddr {
    VirtAddr::from_ptr(stack) + size as u64
}

/// The kernel stack used for ring 3 -> ring 0 interrupt transitions (RSP0).
pub fn rsp0() -> VirtAddr {
    stack_top(addr_of!(RSP0_STACK).cast(), KERNEL_STACK_SIZE)
}

pub fn selectors() -> Selectors {
    GDT.get().expect("gdt not initialised").1
}

pub fn init() {
    let tss = TSS.call_once(|| {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
            stack_top(addr_of!(DF_STACK).cast(), IST_STACK_SIZE);
        tss.interrupt_stack_table[PAGE_FAULT_IST_INDEX as usize] =
            stack_top(addr_of!(PF_STACK).cast(), IST_STACK_SIZE);
        tss.privilege_stack_table[0] = rsp0();
        tss
    });

    let (gdt, selectors) = GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();
        // Order matters for syscall/sysret: kernel code, kernel data, then user
        // data, user code. Do not reorder.
        let kernel_code = gdt.append(Descriptor::kernel_code_segment());
        let kernel_data = gdt.append(Descriptor::kernel_data_segment());
        let user_data = gdt.append(Descriptor::user_data_segment());
        let user_code = gdt.append(Descriptor::user_code_segment());
        let tss = gdt.append(Descriptor::tss_segment(tss));
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
