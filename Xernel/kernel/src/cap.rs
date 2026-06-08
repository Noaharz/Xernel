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

    /// An `IoPort` capability authorizing the port range `[base, base + count)`.
    /// The range is packed into `object`: high 16 bits = base, low 16 = count.
    pub const fn io_port(base: u16, count: u16) -> Self {
        Self::new(CapType::IoPort, ((base as u64) << 16) | count as u64)
    }

    /// For an `IoPort` capability: does it authorize a `size`-byte access at
    /// `port`? A width-`size` access touches port addresses `[port, port+size)`,
    /// so the whole span must lie within the capability's range. Any non-IoPort
    /// capability authorizes nothing here.
    pub fn authorizes_port(&self, port: u16, size: u8) -> bool {
        if self.cap_type != CapType::IoPort {
            return false;
        }
        let base = u32::from((self.object >> 16) as u16);
        let count = u32::from(self.object as u16);
        let lo = u32::from(port);
        let hi = lo + u32::from(size.max(1));
        lo >= base && hi <= base + count
    }

    /// An `IoMem` capability authorizing the physical range `[base, base+len)`.
    /// Physical addresses can be 64-bit (high PCI BARs), so the range uses both
    /// fields: `object` = base, `badge` = length in bytes.
    pub const fn io_mem(base: u64, len: u64) -> Self {
        Self {
            cap_type: CapType::IoMem,
            object: base,
            badge: len,
        }
    }

    /// For an `IoMem` capability: does it authorize mapping the physical range
    /// `[phys, phys+len)`? The whole span must lie within the capability.
    pub fn authorizes_mmio(&self, phys: u64, len: u64) -> bool {
        if self.cap_type != CapType::IoMem {
            return false;
        }
        let (Some(end), Some(cap_end)) = (phys.checked_add(len), self.object.checked_add(self.badge))
        else {
            return false;
        };
        phys >= self.object && end <= cap_end
    }

    /// An `Untyped` capability with a `bytes`-large allocation budget. Unlike the
    /// range caps above, this one is CONSUMABLE: `object` holds the remaining
    /// budget and shrinks as memory is allocated against it. (A fuller seL4-style
    /// `Untyped` would name a specific physical region to retype; here it is a
    /// pure byte budget, which is what bounds a driver's DMA footprint.)
    pub const fn untyped(bytes: u64) -> Self {
        Self::new(CapType::Untyped, bytes)
    }

    /// An `Endpoint` capability naming kernel endpoint `id` — the rendezvous
    /// point for IPC. The holder may send to and receive from that endpoint;
    /// without the capability there is no way to name it.
    pub const fn endpoint(id: u64) -> Self {
        Self::new(CapType::Endpoint, id)
    }

    /// A normalized, userspace-facing view of this capability: `(type, a, b)`,
    /// where the meaning of `a`/`b` depends on the type — IoPort: (base, count);
    /// IoMem: (base, len); Untyped: (remaining bytes, 0); otherwise raw
    /// (object, badge). Lets a process enumerate its own authority via
    /// `SYS_CAP_IDENTIFY` without knowing the kernel's internal bit-packing.
    pub fn describe(&self) -> (u8, u64, u64) {
        let ty = self.cap_type as u8;
        match self.cap_type {
            CapType::IoPort => (
                ty,
                u64::from((self.object >> 16) as u16),
                u64::from(self.object as u16),
            ),
            CapType::Untyped => (ty, self.object, 0),
            _ => (ty, self.object, self.badge),
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

    /// Does this CNode hold any `IoPort` capability authorizing a `size`-byte
    /// access at `port`? This is the kernel's authority check for port-I/O
    /// syscalls: no ambient privilege, only what the holder was granted.
    pub fn authorizes_port(&self, port: u16, size: u8) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|cap| cap.authorizes_port(port, size))
    }

    /// Does this CNode hold any `IoMem` capability authorizing a mapping of the
    /// physical range `[phys, phys+len)`? Consulted by `SYS_IOMAP`.
    pub fn authorizes_mmio(&self, phys: u64, len: u64) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|cap| cap.authorizes_mmio(phys, len))
    }

    /// Charge `amount` bytes against the first `Untyped` capability that still
    /// has enough budget, decrementing it. Returns `true` if charged. This is
    /// how `SYS_DMA_ALLOC` is bounded: a process can pin only as much physical
    /// memory as its `Untyped` budget allows — no unbounded allocation.
    pub fn charge_untyped(&mut self, amount: u64) -> bool {
        for cap in self.slots.iter_mut().flatten() {
            if cap.cap_type == CapType::Untyped && cap.object >= amount {
                cap.object -= amount;
                return true;
            }
        }
        false
    }

    /// Return `amount` bytes to the first `Untyped` capability — used to undo a
    /// charge when the allocation it paid for later fails. Best-effort: a no-op
    /// if the holder has no `Untyped` cap.
    pub fn refund_untyped(&mut self, amount: u64) {
        if let Some(cap) = self
            .slots
            .iter_mut()
            .flatten()
            .find(|c| c.cap_type == CapType::Untyped)
        {
            cap.object = cap.object.saturating_add(amount);
        }
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
