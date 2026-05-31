//! Virtual memory: page-table manipulation over the Limine higher-half direct
//! map (HHDM).
//!
//! Limine maps all physical memory at a fixed offset (`hhdm_offset`), so a
//! physical address `pa` is reachable at virtual address `pa + hhdm_offset`.
//! That lets us walk and edit page tables with an
//! [`OffsetPageTable`](x86_64::structures::paging::OffsetPageTable) without
//! recursive mapping.

use spin::Once;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::mapper::MapToError;
use x86_64::structures::paging::{
    FrameAllocator as X86FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags,
    PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

use crate::mm::frame;

static HHDM_OFFSET: Once<u64> = Once::new();

pub fn init(hhdm_offset: u64) {
    HHDM_OFFSET.call_once(|| hhdm_offset);
}

pub fn hhdm_offset() -> u64 {
    *HHDM_OFFSET.get().expect("paging::init not called")
}

/// Adapts the generic physical frame allocator to the x86_64 paging trait, so
/// page-table intermediate frames come from the same pool as everything else.
pub struct GlobalFrames;

// SAFETY: `frame::alloc` returns frames that are not handed out twice and are
// backed by real usable RAM, satisfying the trait's contract.
unsafe impl X86FrameAllocator<Size4KiB> for GlobalFrames {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        frame::alloc().map(|pa| PhysFrame::containing_address(PhysAddr::new(pa)))
    }
}

fn active_level_4_table() -> &'static mut PageTable {
    let (l4_frame, _) = Cr3::read();
    let virt = l4_frame.start_address().as_u64() + hhdm_offset();
    // SAFETY: the HHDM maps all physical memory, so the active L4 table is
    // reachable here. It lives for the whole kernel lifetime, hence 'static.
    unsafe { &mut *(virt as *mut PageTable) }
}

fn mapper() -> OffsetPageTable<'static> {
    // SAFETY: the offset is the bootloader-provided HHDM base and the L4 table
    // pointer is valid (see `active_level_4_table`).
    unsafe { OffsetPageTable::new(active_level_4_table(), VirtAddr::new(hhdm_offset())) }
}

fn map_with_flags(
    virt: u64,
    phys: u64,
    flags: PageTableFlags,
) -> Result<(), MapToError<Size4KiB>> {
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt));
    let phys = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(phys));
    // SAFETY: the caller picks the mapping; we flush the TLB on success. Mapping
    // an already-mapped page returns an error rather than corrupting state.
    unsafe {
        mapper().map_to(page, phys, flags, &mut GlobalFrames)?.flush();
    }
    Ok(())
}

/// Map `virt` to `phys` (both 4 KiB-aligned) with the given permissions.
pub fn map(virt: u64, phys: u64, writable: bool) -> Result<(), MapToError<Size4KiB>> {
    let mut flags = PageTableFlags::PRESENT;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
    map_with_flags(virt, phys, flags)
}

/// Map a device-memory (MMIO) page: present, writable, and uncached so writes
/// reach the device instead of sitting in a cache line.
pub fn map_mmio(virt: u64, phys: u64) -> Result<(), MapToError<Size4KiB>> {
    map_with_flags(
        virt,
        phys,
        PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_CACHE
            | PageTableFlags::WRITE_THROUGH,
    )
}

/// Map a user-accessible device-memory page (e.g. a framebuffer): user, RW,
/// uncached (so writes reach the device/scanout), never executable.
pub fn map_user_device(virt: u64, phys: u64) -> Result<(), MapToError<Size4KiB>> {
    map_with_flags(
        virt,
        phys,
        PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_CACHE
            | PageTableFlags::WRITE_THROUGH
            | PageTableFlags::NO_EXECUTE,
    )
}

/// Map a user-accessible page. `writable` controls W, `executable` controls
/// whether instructions may be fetched (when false, the page is marked NX).
///
/// Note: the intermediate page-table entries are created with USER_ACCESSIBLE
/// by `map_to` because we pass that flag here, so the whole walk is reachable
/// from ring 3.
pub fn map_user(
    virt: u64,
    phys: u64,
    writable: bool,
    executable: bool,
) -> Result<(), MapToError<Size4KiB>> {
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
    if !executable {
        flags |= PageTableFlags::NO_EXECUTE;
    }
    map_with_flags(virt, phys, flags)
}

/// Allocate a fresh frame, map it at a scratch virtual address, write and read
/// back a sentinel. Proves the frame allocator and the mapper agree. Returns
/// `false` on any failure.
pub fn selftest() -> bool {
    const SCRATCH: u64 = 0xffff_9000_0000_0000;
    const SENTINEL: u64 = 0x_dead_beef_cafe_f00d;

    let Some(phys) = frame::alloc() else {
        return false;
    };
    if map(SCRATCH, phys, true).is_err() {
        return false;
    }
    let ptr = SCRATCH as *mut u64;
    // SAFETY: SCRATCH was just mapped writable to a fresh frame.
    let ok = unsafe {
        ptr.write_volatile(SENTINEL);
        ptr.read_volatile() == SENTINEL
    };
    ok
}
