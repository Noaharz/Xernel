//! Memory management.
//!
//! Layering (built up across phase 1):
//!   1. **Early heap** — a fixed region in `.bss`, mapped by the bootloader as
//!      part of the kernel image. Available immediately so the rest of the
//!      kernel can use `alloc` before we own the page tables. Set up by
//!      [`init_early_heap`].
//!   2. **Frame allocator** — hands out physical frames from the Limine memory
//!      map (`mm::frame`).
//!   3. **Paging** — `OffsetPageTable` over the bootloader's higher-half direct
//!      map (`mm::paging`).
//!
//! Once (2) and (3) are up, the heap can be grown with freshly mapped frames;
//! the early region stays as the bootstrap arena.

pub mod frame;
pub mod paging;

use core::ptr::addr_of_mut;

use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Size of the bootstrap heap carved out of `.bss`.
const EARLY_HEAP_SIZE: usize = 1024 * 1024; // 1 MiB

static mut EARLY_HEAP: [u8; EARLY_HEAP_SIZE] = [0; EARLY_HEAP_SIZE];

/// Initialise the bootstrap heap. Must be called exactly once, early in boot,
/// before any allocation occurs.
pub fn init_early_heap() {
    // SAFETY: `EARLY_HEAP` is a private static touched only here, before any
    // other code can allocate, so there is no aliasing. The region lives in
    // `.bss` and is therefore mapped and zeroed by the bootloader.
    unsafe {
        let ptr = addr_of_mut!(EARLY_HEAP).cast::<u8>();
        ALLOCATOR.lock().init(ptr, EARLY_HEAP_SIZE);
    }
}

/// Build the physical frame allocator from the regions the architecture layer
/// reports as usable. Requires [`init_early_heap`] to have run.
pub fn init_frames() {
    frame::init(crate::arch::usable_regions().map(|(start, end)| frame::Region::new(start, end)));
}

/// Log a one-line summary of physical memory.
pub fn report() {
    let (in_use, total) = frame::stats();
    crate::println!(
        "[xernel] phys: {} MiB usable ({} frames, {} in use)",
        total * frame::FRAME_SIZE / (1024 * 1024),
        total,
        in_use,
    );
}
