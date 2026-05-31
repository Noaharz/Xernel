//! Capability type definitions shared between kernel and userspace.

/// Index into a CNode. Opaque from userspace; the kernel interprets the bits.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CapPtr(pub u64);

impl CapPtr {
    pub const NULL: Self = Self(0);
}

/// Tag describing what kind of object a capability refers to. The numeric
/// values are part of the ABI — never reorder, only append.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CapType {
    Null = 0,
    Untyped = 1,
    CNode = 2,
    Frame = 3,
    PageTable = 4,
    Thread = 5,
    VSpace = 6,
    Endpoint = 7,
    Notification = 8,
    IrqHandler = 9,
    IoPort = 10,
}
