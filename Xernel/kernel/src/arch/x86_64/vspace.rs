//! Per-process address spaces.
//!
//! Each process gets its own top-level page table (PML4). The kernel lives in
//! the higher half (PML4 entries 256..512: the HHDM and the kernel image), so
//! those entries are *shared* — copied into every process's PML4 — and the
//! kernel stays mapped no matter which address space is active. The lower half
//! (entries 0..256) is private per process: that is where the program, its
//! stack and heap live, and that is what gives processes isolation.
//!
//! An address space is identified by the physical address of its PML4 (the
//! value we load into CR3). We manipulate any page table — active or not —
//! through the HHDM, so mapping into a not-yet-active process works.

use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::{
    Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

use super::paging::{self, GlobalFrames};
use crate::mm::frame;

/// Number of the first higher-half PML4 entry (kernel space). Entries
/// `KERNEL_PML4_START..512` are shared across all address spaces.
const KERNEL_PML4_START: usize = 256;

fn table_mut(l4_phys: u64) -> &'static mut PageTable {
    let virt = l4_phys + paging::hhdm_offset();
    // SAFETY: every physical frame is reachable through the HHDM; `l4_phys` is
    // a frame we own as a page table for the whole kernel lifetime.
    unsafe { &mut *(virt as *mut PageTable) }
}

/// Create a fresh address space: a new PML4 with the kernel's higher-half
/// entries copied in. Returns its PML4 physical address, or `None` if out of
/// frames.
pub fn new() -> Option<u64> {
    let l4_phys = frame::alloc()?;
    let new_table = table_mut(l4_phys);
    new_table.zero();

    // Share the kernel half by copying the active PML4's higher-half entries.
    let (active_frame, _) = Cr3::read();
    let active = table_mut(active_frame.start_address().as_u64());
    for i in KERNEL_PML4_START..512 {
        new_table[i] = active[i].clone();
    }
    Some(l4_phys)
}

fn mapper(l4_phys: u64) -> OffsetPageTable<'static> {
    // SAFETY: HHDM offset is correct and `l4_phys` points at a valid PML4.
    unsafe { OffsetPageTable::new(table_mut(l4_phys), VirtAddr::new(paging::hhdm_offset())) }
}

/// Map a user page into address space `l4_phys` (not necessarily the active
/// one). `writable`/`executable` control W and NX.
pub fn map_user(l4_phys: u64, virt: u64, phys: u64, writable: bool, executable: bool) -> bool {
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
    if !executable {
        flags |= PageTableFlags::NO_EXECUTE;
    }
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt));
    let frame_ = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(phys));
    // SAFETY: mapping into a private lower-half address; we `ignore` the TLB
    // flush because the target space may not be active (a CR3 reload on switch
    // flushes anyway).
    unsafe {
        match mapper(l4_phys).map_to(page, frame_, flags, &mut GlobalFrames) {
            Ok(flush) => {
                flush.ignore();
                true
            }
            Err(_) => false,
        }
    }
}

/// Make `l4_phys` the active address space.
///
/// # Safety
/// `l4_phys` must be a valid PML4 with the kernel higher-half mapped (i.e. one
/// produced by [`new`]), otherwise the next instruction fetch faults.
pub unsafe fn switch(l4_phys: u64) {
    let frame_ = PhysFrame::containing_address(PhysAddr::new(l4_phys));
    unsafe { Cr3::write(frame_, Cr3Flags::empty()) };
}

/// Physical address of the currently active PML4.
pub fn current() -> u64 {
    Cr3::read().0.start_address().as_u64()
}

/// Self-test: build a second address space, switch into it, write and read a
/// page mapped only there, then switch back. Proves the kernel half stays
/// mapped across a CR3 switch and that a fresh space's private mapping works.
pub fn selftest() -> bool {
    let original = current();
    let Some(space) = new() else {
        return false;
    };
    const TEST_VA: u64 = 0x4000_0000; // 1 GiB, lower half, otherwise unused
    const SENTINEL: u64 = 0x1234_5678_9abc_def0;
    if !alloc_map_user(space, TEST_VA, true, false) {
        return false;
    }
    let ok;
    // SAFETY: `space` shares the kernel higher half (so code/stack/HHDM stay
    // mapped), and TEST_VA is mapped read/write in it. We restore CR3 after.
    unsafe {
        switch(space);
        let p = TEST_VA as *mut u64;
        p.write_volatile(SENTINEL);
        ok = p.read_volatile() == SENTINEL;
        switch(original);
    }
    ok
}

/// Allocate, map (read/write) and zero one fresh user page in `l4_phys` at
/// `virt`. Returns `false` on failure.
pub fn alloc_map_user(l4_phys: u64, virt: u64, writable: bool, executable: bool) -> bool {
    let Some(phys) = frame::alloc() else {
        return false;
    };
    // SAFETY: fresh frame, reachable via HHDM for a full page.
    unsafe {
        core::ptr::write_bytes((phys + paging::hhdm_offset()) as *mut u8, 0, 4096);
    }
    map_user(l4_phys, virt, phys, writable, executable)
}
