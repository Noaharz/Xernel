//! x86_64 `syscall`/`sysret` fast-system-call support.
//!
//! User code enters the kernel with the `syscall` instruction:
//!   - syscall number in `rax`
//!   - arguments in `rdi, rsi, rdx, r10, r8, r9` (note: `r10`, not `rcx`,
//!     because `syscall` clobbers `rcx` with the return address)
//!   - return value in `rax`
//!
//! `syscall` itself saves the user RIP in `rcx` and RFLAGS in `r11`, masks
//! RFLAGS with `SFMASK`, and loads CS/SS from `STAR`. It does **not** switch
//! stacks — [`syscall_entry`] does that by hand via `swapgs` and a per-CPU
//! scratch area reachable through `gs`.

use core::arch::naked_asm;

use x86_64::registers::model_specific::{Efer, EferFlags, KernelGsBase, LStar, SFMask, Star};
use x86_64::registers::rflags::RFlags;
use x86_64::VirtAddr;

use super::gdt;

/// Per-CPU scratch reachable via `gs` inside the syscall entry stub. Field
/// order is part of the ABI with the assembly below: `kernel_rsp` at offset 0,
/// `user_rsp` at offset 8. Do not reorder.
#[repr(C)]
struct PerCpu {
    kernel_rsp: u64,
    user_rsp: u64,
}

static mut PERCPU: PerCpu = PerCpu {
    kernel_rsp: 0,
    user_rsp: 0,
};

const SYSCALL_STACK_SIZE: usize = 4096 * 5;
static mut SYSCALL_STACK: [u8; SYSCALL_STACK_SIZE] = [0; SYSCALL_STACK_SIZE];

/// Register frame the entry stub builds on the kernel stack. The field order
/// matches the `push`/`pop` sequence in [`syscall_entry`]; `&mut SyscallFrame`
/// is handed to [`dispatch`].
#[repr(C)]
pub struct SyscallFrame {
    pub r9: u64,
    pub r8: u64,
    pub r10: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rax: u64, // syscall number on entry, return value on exit
    pub rcx: u64, // user RIP (consumed by sysretq)
    pub r11: u64, // user RFLAGS (consumed by sysretq)
}

pub fn init() {
    // Enable the SYSCALL/SYSRET instructions.
    // SAFETY: setting SCE on a long-mode CPU is the documented way to turn on
    // syscall; we set the related MSRs immediately below.
    unsafe { Efer::update(|f| *f |= EferFlags::SYSTEM_CALL_EXTENSIONS) };

    let sel = gdt::selectors();
    // STAR encodes the segment selectors syscall/sysret load. Layout (see the
    // crate docs): SYSCALL CS = `syscall` field, SS = +8; SYSRET CS =
    // `sysret` + 16, SS = `sysret` + 8. Our GDT order makes this work out to:
    //   syscall = kernel_code, sysret = user_code - 16.
    let syscall_base = sel.kernel_code.0;
    let sysret_base = sel.user_code.0 - 16;
    // sysret derives SS from `sysret_base + 8`; that must be the user data
    // selector. If this trips, the GDT order in `gdt.rs` was changed.
    debug_assert_eq!(sysret_base + 8, sel.user_data.0, "GDT user seg order");
    // SAFETY: the selectors come straight from our loaded GDT and satisfy the
    // adjacency syscall/sysret require.
    unsafe { Star::write_raw(sysret_base, syscall_base) };

    // Entry point and the RFLAGS bits cleared on entry (mask interrupts and the
    // direction/trap flags while we transition).
    LStar::write(VirtAddr::new(syscall_entry as *const () as u64));
    SFMask::write(RFlags::INTERRUPT_FLAG | RFlags::DIRECTION_FLAG | RFlags::TRAP_FLAG);

    // Point the per-CPU scratch at our syscall kernel stack (16-byte aligned so
    // the `call` inside the stub stays ABI-aligned).
    // SAFETY: single-CPU bring-up; PERCPU/SYSCALL_STACK are touched only here
    // and from the entry stub, which cannot run until these MSRs are set.
    unsafe {
        let top = core::ptr::addr_of!(SYSCALL_STACK).cast::<u8>() as u64 + SYSCALL_STACK_SIZE as u64;
        PERCPU.kernel_rsp = top & !0xf;
        KernelGsBase::write(VirtAddr::new(core::ptr::addr_of!(PERCPU) as u64));
    }
}

/// Enter ring 3 at `entry` with `user_stack_top` as the user stack pointer.
/// Never returns to the caller.
///
/// # Safety
/// `entry` and `user_stack_top` must be valid, user-accessible mappings, and
/// the syscall MSRs must already be initialised via [`init`].
pub unsafe fn enter_user(entry: u64, user_stack_top: u64) -> ! {
    // We deliberately do not `swapgs` here: while in the kernel GS.base is 0
    // (the user value) and KERNEL_GS_BASE holds the per-CPU pointer, which is
    // exactly the state user code should run with. The first `swapgs` happens
    // on syscall entry. RFLAGS 0x202 = reserved bit + IF (interrupts on).
    unsafe {
        core::arch::asm!(
            "mov rsp, {stack}",
            "sysretq",
            stack = in(reg) user_stack_top,
            in("rcx") entry,        // sysretq loads RIP from rcx
            in("r11") 0x202u64,     // sysretq loads RFLAGS from r11
            options(noreturn),
        )
    }
}

/// Assembly entry point installed in LSTAR. Switches to the kernel stack, saves
/// the user register frame, calls [`dispatch`], restores, and `sysretq`s back.
#[unsafe(naked)]
unsafe extern "C" fn syscall_entry() {
    naked_asm!(
        "swapgs",                 // GS.base -> per-CPU scratch
        "mov gs:[8], rsp",        // PERCPU.user_rsp = user rsp
        "mov rsp, gs:[0]",        // rsp = PERCPU.kernel_rsp
        "push r11",               // user RFLAGS
        "push rcx",               // user RIP
        "push rax",               // syscall nr (return value slot)
        "push rdi",
        "push rsi",
        "push rdx",
        "push r10",
        "push r8",
        "push r9",
        "mov rdi, rsp",           // &SyscallFrame
        "sub rsp, 8",             // re-align to 16 for the call
        "call {dispatch}",
        "add rsp, 8",
        "pop r9",
        "pop r8",
        "pop r10",
        "pop rdx",
        "pop rsi",
        "pop rdi",
        "pop rax",                // return value
        "pop rcx",                // user RIP
        "pop r11",                // user RFLAGS
        "mov rsp, gs:[8]",        // restore user rsp
        "swapgs",
        "sysretq",
        dispatch = sym dispatch,
    )
}

extern "C" fn dispatch(frame: &mut SyscallFrame) {
    let ret = crate::syscall::dispatch(
        frame.rax,
        [frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8, frame.r9],
    );
    frame.rax = ret;
}
