//! Linear framebuffer access for user space.
//!
//! Limine sets up a framebuffer and maps it in the higher half (kernel-only).
//! To let a ring-3 program draw pixels, we map the same physical pages into the
//! low (user) half. The mapping is uncached (device memory), so writes reach the
//! scanout instead of sitting in a cache line.
//!
//! IMPORTANT: the mapping is **per address space**. Each process has its own
//! page tables, so the framebuffer must be mapped into the *current* process's
//! address space whenever it asks (`info`). The geometry is discovered once and
//! cached, but the page-table mapping is (re)done on every call — otherwise a
//! process other than the first caller would get the address but no mapping and
//! fault on the first pixel write.

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

#[derive(Clone, Copy)]
struct Geometry {
    phys_base: u64,
    offset: u64,
    pages: u64,
    width: u64,
    height: u64,
    pitch: u64,
    bpp: u64,
}

/// Framebuffer geometry, discovered once from the bootloader (independent of
/// any address space). `None` if there is no framebuffer.
static GEOMETRY: Once<Option<Geometry>> = Once::new();

fn geometry() -> Option<Geometry> {
    *GEOMETRY.call_once(|| {
        let fb = FRAMEBUFFER.response()?.framebuffers().first().copied()?;
        let phys = (fb.address() as u64).checked_sub(paging::hhdm_offset())?;
        let phys_base = phys & !(PAGE - 1);
        let offset = phys - phys_base;
        let size = fb.height * fb.pitch;
        Some(Geometry {
            phys_base,
            offset,
            pages: (offset + size).div_ceil(PAGE),
            width: fb.width,
            height: fb.height,
            pitch: fb.pitch,
            bpp: u64::from(fb.bpp),
        })
    })
}

/// Ensure the framebuffer is mapped into the CURRENT address space and return
/// `[user_addr, width, height, pitch, bpp]`, or `None` if there is none.
pub fn info() -> Option<[u64; 5]> {
    let g = geometry()?;
    // (Re)map into whatever address space is active now — once per process that
    // asks. Already-mapped pages are fine (idempotent).
    for i in 0..g.pages {
        if !paging::map_user_device_idempotent(FB_USER_VA + i * PAGE, g.phys_base + i * PAGE) {
            return None;
        }
    }
    Some([FB_USER_VA + g.offset, g.width, g.height, g.pitch, g.bpp])
}
