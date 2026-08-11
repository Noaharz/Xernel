//! Reference counting for shared (delegated) frames — the lifetime half of
//! capability revocation.
//!
//! A `Frame` capability can be copied to other processes over IPC (the file-
//! service hands clients a page). The physical memory must stay alive while ANY
//! holder still references it and be reclaimed the moment the last reference
//! goes away. We track that with one counter per physical base address.
//!
//! The count is maintained at exactly the points a Frame-cap reference is
//! created or destroyed:
//!   - `SYS_FRAME_ALLOC` installs the first cap            -> [`inc`] (0 -> 1)
//!   - `SYS_SEND` copies a Frame cap into a message        -> [`inc`] (in-flight)
//!   - `SYS_RECV` discards an arriving Frame cap           -> [`dec`]
//!     (installing it instead just relocates the same reference — no change)
//!   - `SYS_FRAME_DROP` removes an installed cap           -> [`dec`]
//! When [`dec`] returns `true`, the caller frees the underlying frames.
//!
//! Limitation (honest): a process that exits while still holding a Frame cap is
//! not swept, so its reference — and the frame — leaks. A full seL4-style
//! capability-derivation tree (revoke a child's authority from the parent) is a
//! larger, later piece; this closes the common alloc → share → drop → free path.

use alloc::collections::BTreeMap;

use spin::Mutex;

/// Physical base address -> number of live Frame-cap references to it.
static REFS: Mutex<BTreeMap<u64, u32>> = Mutex::new(BTreeMap::new());

/// Record one more reference to the frame at physical base `phys`.
pub fn inc(phys: u64) {
    *REFS.lock().entry(phys).or_insert(0) += 1;
}

/// Drop one reference to the frame at `phys`. Returns `true` if that was the
/// last reference (the count hit zero and the entry was removed) — the caller
/// must then free the physical frames.
pub fn dec(phys: u64) -> bool {
    let mut refs = REFS.lock();
    if let Some(count) = refs.get_mut(&phys) {
        *count -= 1;
        if *count == 0 {
            refs.remove(&phys);
            return true;
        }
    }
    false
}
