//! Early CPU feature setup.

use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};

/// Enable the x87 FPU and SSE so user (and kernel) code may use the standard
/// x86_64 floating-point/SIMD ABI without taking `#UD`.
///
/// Why this matters: the System V x86_64 ABI passes/returns floats in XMM
/// registers and the compiler emits SSE freely (even for plain memory moves).
/// Without `CR4.OSFXSR` an SSE instruction faults with `#UD`. Enabling it here
/// lets userland be compiled with a normal target instead of fighting the
/// toolchain with soft-float.
///
/// The kernel itself is built soft-float and never touches XMM, so it does not
/// clobber user XMM state across syscalls. (A future multi-process kernel will
/// still need to save/restore XMM on context switch.)
pub fn enable_sse() {
    // SAFETY: standard SSE bring-up on any x86_64 CPU. We clear EM (no FPU
    // emulation), set MP, and turn on OSFXSR + OSXMMEXCPT so FXSAVE-style state
    // and unmasked SIMD exceptions are available.
    unsafe {
        Cr0::update(|f| {
            f.remove(Cr0Flags::EMULATE_COPROCESSOR);
            f.insert(Cr0Flags::MONITOR_COPROCESSOR);
        });
        Cr4::update(|f| {
            f.insert(Cr4Flags::OSFXSR);
            f.insert(Cr4Flags::OSXMMEXCPT_ENABLE);
        });
    }
}
