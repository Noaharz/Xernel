//! Linear framebuffer access for user space.
//!
//! Limine sets up a framebuffer and maps it in the higher half (kernel-only).
//! To let a ring-3 program draw pixels, we map the same physical pages into the
//! low (user) half on first request and hand back the user virtual address plus
//! the geometry. The mapping is uncached (device memory), so writes reach the
//! scanout instead of sitting in a cache line.

use limine::request::FramebufferRequest;
use spin::Once;

use super::paging;

#[used]
#[link_section = ".requests"]
static FRAMEBUFFER: FramebufferRequest = FramebufferRequest::new();

/// User virtual base for the framebuffer mapping. 768 MiB — clear of the user
/// image (4 MiB), stack (8 MiB) and heap region (256–512 MiB).
const FB_USER_VA: u64 = 0x3000_0000;
const PAGE: u64 = 4096;

/// `[user_addr, width, height, pitch_bytes, bpp]`, computed and mapped once.
static INFO: Once<Option<[u64; 5]>> = Once::new();

/// Geometry of the framebuffer and the user virtual address it is mapped at, or
/// `None` if the bootloader provided no framebuffer. Maps it on first call.
pub fn info() -> Option<[u64; 5]> {
    *INFO.call_once(map_framebuffer)
}

fn map_framebuffer() -> Option<[u64; 5]> {
    let response = FRAMEBUFFER.response()?;
    let fb = response.framebuffers().first()?;

    let virt = fb.address() as u64;
    let phys = virt.checked_sub(paging::hhdm_offset())?;
    let width = fb.width;
    let height = fb.height;
    let pitch = fb.pitch;
    let bpp = u64::from(fb.bpp);
    let size = height * pitch;

    // Map every covered physical page into the user half (handle a non-page
    // aligned base defensively, though Limine framebuffers are page-aligned).
    let phys_base = phys & !(PAGE - 1);
    let offset = phys - phys_base;
    let pages = (offset + size).div_ceil(PAGE);
    for i in 0..pages {
        paging::map_user_device(FB_USER_VA + i * PAGE, phys_base + i * PAGE).ok()?;
    }

    Some([FB_USER_VA + offset, width, height, pitch, bpp])
}
