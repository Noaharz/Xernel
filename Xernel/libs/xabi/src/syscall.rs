//! Raw syscall stubs.
//!
//! There is exactly one syscall in Xernel: `invoke(cap, method, args…)`.
//! Architecture-specific assembly stubs go here.

use crate::cap::CapPtr;

/// Method selector inside an invocation. Each capability type defines its own
/// method namespace.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Method(pub u32);

/// Raw, untyped invocation. Type-safe wrappers live in `xstd`.
///
/// # Safety
/// Caller must ensure that `cap` is a valid capability pointer in its own
/// CNode and that `method`/`args` form a legal invocation for the capability's
/// type. Misuse is contained by the kernel — it will return `InvalidCap` /
/// `BadOp` rather than crash — but the caller can still cause logical havoc.
#[inline]
pub unsafe fn invoke(_cap: CapPtr, _method: Method, _args: &[u64]) -> i64 {
    // Phase 2 — replace with arch-specific syscall instruction.
    -1
}
