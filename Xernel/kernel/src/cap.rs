//! Capability tables — the kernel side of Xernel's security model.
//!
//! Every authority in Xernel is a capability: an unforgeable reference to a
//! kernel object plus the rights to act on it. Capabilities live in **CNodes**
//! (capability tables); user threads name a capability by an index into a CNode
//! and can only copy/move/delete the ones they already hold. There are no
//! ambient privileges — this is the seL4-style model from the design doc.
//!
//! This module provides the in-kernel data structures and the basic operations
//! (insert, lookup, copy, delete). Wiring them to the syscall surface as
//! `invoke(cap, method, args)` and to `Untyped` retyping comes next; the shapes
//! here are chosen so that step is additive.

use alloc::vec;
use alloc::vec::Vec;

use xabi::cap::CapType;
use xabi::errno::CapError;

/// One capability: what kind of object it points at, a reference to the object
/// (interpretation depends on the type — physical address, object id, …), and a
/// badge the holder cannot alter (used to identify senders on an Endpoint).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CapEntry {
    pub cap_type: CapType,
    pub object: u64,
    pub badge: u64,
}

impl CapEntry {
    pub const fn new(cap_type: CapType, object: u64) -> Self {
        Self {
            cap_type,
            object,
            badge: 0,
        }
    }
}

/// A capability table: a fixed number of slots, each empty or holding one
/// capability. Slots are addressed by index.
pub struct CNode {
    slots: Vec<Option<CapEntry>>,
}

impl CNode {
    /// Create a CNode with `len` empty slots.
    pub fn new(len: usize) -> Self {
        Self {
            slots: vec![None; len],
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    fn slot(&self, index: usize) -> Result<&Option<CapEntry>, CapError> {
        self.slots.get(index).ok_or(CapError::InvalidCap)
    }

    /// Read the capability in `index`, if any.
    pub fn get(&self, index: usize) -> Result<CapEntry, CapError> {
        (*self.slot(index)?).ok_or(CapError::InvalidCap)
    }

    /// Place `cap` into an empty slot. Fails if the slot is out of range or
    /// already occupied (capabilities are never silently overwritten).
    pub fn insert(&mut self, index: usize, cap: CapEntry) -> Result<(), CapError> {
        let slot = self.slots.get_mut(index).ok_or(CapError::InvalidCap)?;
        if slot.is_some() {
            return Err(CapError::NotPermitted);
        }
        *slot = Some(cap);
        Ok(())
    }

    /// Copy the capability in `src` into the empty slot `dst`.
    pub fn copy(&mut self, src: usize, dst: usize) -> Result<(), CapError> {
        let cap = self.get(src)?;
        self.insert(dst, cap)
    }

    /// Remove and return the capability in `index`.
    pub fn delete(&mut self, index: usize) -> Result<CapEntry, CapError> {
        let slot = self.slots.get_mut(index).ok_or(CapError::InvalidCap)?;
        slot.take().ok_or(CapError::InvalidCap)
    }
}

/// Exercise the CNode operations end to end. Returns `Err` on any deviation.
/// Run from the boot self-tests.
pub fn selftest() -> Result<(), CapError> {
    let mut cnode = CNode::new(8);

    // Empty slot reads as invalid.
    if cnode.get(0) != Err(CapError::InvalidCap) {
        return Err(CapError::BadOp);
    }

    // Insert an Endpoint cap, read it back.
    let ep = CapEntry::new(CapType::Endpoint, 0x1000);
    cnode.insert(0, ep)?;
    if cnode.get(0)? != ep {
        return Err(CapError::BadOp);
    }

    // Inserting into an occupied slot is refused.
    if cnode.insert(0, ep) != Err(CapError::NotPermitted) {
        return Err(CapError::BadOp);
    }

    // Copy to an empty slot, then both must match.
    cnode.copy(0, 1)?;
    if cnode.get(1)? != ep {
        return Err(CapError::BadOp);
    }

    // Delete the original; slot 0 empty again, slot 1 still holds the copy.
    let removed = cnode.delete(0)?;
    if removed != ep || cnode.get(0) != Err(CapError::InvalidCap) || cnode.get(1)? != ep {
        return Err(CapError::BadOp);
    }

    // Out-of-range index is invalid, not a panic.
    if cnode.get(999) != Err(CapError::InvalidCap) {
        return Err(CapError::BadOp);
    }

    Ok(())
}
