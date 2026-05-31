//! Kernel-thread context switching (cooperative).
//!
//! A switch saves the System V callee-saved registers of the current thread on
//! its own stack, stores the resulting stack pointer, loads the next thread's
//! stack pointer, and restores its registers. Caller-saved state is preserved
//! by the normal calling convention because [`switch`] is an ordinary `extern
//! "C"` call from the scheduler's point of view.

use core::arch::naked_asm;

/// Save the current context to `*save_rsp`, then load and resume `next_rsp`.
///
/// # Safety
/// `save_rsp` must point to writable storage for one `u64`. `next_rsp` must be
/// a stack pointer previously produced by [`init_stack`] or a prior `switch`.
#[unsafe(naked)]
pub unsafe extern "C" fn switch(save_rsp: *mut u64, next_rsp: u64) {
    // rdi = save_rsp, rsi = next_rsp
    naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}

/// Lay out a fresh thread stack so the first [`switch`] into it begins
/// executing `entry`. Returns the initial saved stack pointer.
///
/// The thread entry must never return.
pub fn init_stack(stack: &mut [u64], entry: extern "C" fn() -> !) -> u64 {
    let end = stack.as_mut_ptr_range().end as u64;
    // Arrange so that on first entry `rsp % 16 == 8`, matching the state a
    // normal `call` would leave.
    let top = (end & !0xf) - 8;
    let mut sp = top as *mut u64;
    let mut push = |value: u64| {
        // SAFETY: each slot is within `stack`; we write 7 words below `top`,
        // and callers size stacks far larger than that.
        unsafe {
            sp = sp.sub(1);
            sp.write(value);
        }
    };
    push(entry as usize as u64); // return address consumed by `ret`
    push(0); // rbp
    push(0); // rbx
    push(0); // r12
    push(0); // r13
    push(0); // r14
    push(0); // r15
    sp as u64
}
