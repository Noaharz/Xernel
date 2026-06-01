//! Minimal ELF64 loader.
//!
//! Loads the `PT_LOAD` segments of a statically-linked ELF executable into the
//! current address space as user-accessible pages and returns the entry point.
//! Deliberately small: static, non-relocatable ELF64 only — a fuller loader
//! (dynamic linking, relocations, interpreter) is a Phase-3+ concern that
//! belongs in a root server, not the kernel.
//!
//! It loads in two passes so that segments which *share* a page (common in
//! small binaries whose linker does not page-align sections) load correctly:
//!   1. Walk all `PT_LOAD` segments and map every covered page exactly once,
//!      with the union of the segments' permissions, zero-filled.
//!   2. Copy each segment's file bytes into the now-mapped pages.
//! Mapping each page only once is what fixes the `MapFailed` that an early XOS
//! build hit (two segments in one page -> the second map failed).
//!
//! All fields are read with explicit little-endian byte reads, so the loader
//! makes no alignment assumptions about the module image.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::{arch, mm::frame};

const PAGE: u64 = 4096;

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;

#[derive(Debug)]
pub enum ElfError {
    TooSmall,
    BadMagic,
    NotElf64Le,
    NoFrame,
    MapFailed,
    Malformed,
}

fn rd_u16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}
fn rd_u32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}
fn rd_u64(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

struct Segment {
    offset: usize,
    vaddr: u64,
    filesz: usize,
    memsz: u64,
    writable: bool,
    executable: bool,
}

struct PageInfo {
    writable: bool,
    executable: bool,
    phys: u64,
}

/// Load `image` into address space `space` (a handle from
/// `arch::vspace_new`). Returns the entry virtual address on success.
pub fn load(image: &[u8], space: u64) -> Result<u64, ElfError> {
    if image.len() < 64 {
        return Err(ElfError::TooSmall);
    }
    if &image[0..4] != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if image[4] != ELFCLASS64 || image[5] != ELFDATA2LSB {
        return Err(ElfError::NotElf64Le);
    }

    let entry = rd_u64(image, 24).ok_or(ElfError::Malformed)?;
    let phoff = rd_u64(image, 32).ok_or(ElfError::Malformed)? as usize;
    let phentsize = rd_u16(image, 54).ok_or(ElfError::Malformed)? as usize;
    let phnum = rd_u16(image, 56).ok_or(ElfError::Malformed)? as usize;

    // Collect the PT_LOAD segments.
    let mut segments: Vec<Segment> = Vec::new();
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if rd_u32(image, ph).ok_or(ElfError::Malformed)? != PT_LOAD {
            continue;
        }
        let p_flags = rd_u32(image, ph + 4).ok_or(ElfError::Malformed)?;
        segments.push(Segment {
            offset: rd_u64(image, ph + 8).ok_or(ElfError::Malformed)? as usize,
            vaddr: rd_u64(image, ph + 16).ok_or(ElfError::Malformed)?,
            filesz: rd_u64(image, ph + 32).ok_or(ElfError::Malformed)? as usize,
            memsz: rd_u64(image, ph + 40).ok_or(ElfError::Malformed)?,
            writable: p_flags & PF_W != 0,
            executable: p_flags & PF_X != 0,
        });
    }

    // Pass 1: every covered page, mapped once, with the union of permissions.
    let mut pages: BTreeMap<u64, PageInfo> = BTreeMap::new();
    for seg in &segments {
        let start = seg.vaddr & !(PAGE - 1);
        let end = seg
            .vaddr
            .checked_add(seg.memsz)
            .ok_or(ElfError::Malformed)?
            .div_ceil(PAGE)
            * PAGE;
        let mut page = start;
        while page < end {
            let info = pages.entry(page).or_insert(PageInfo {
                writable: false,
                executable: false,
                phys: 0,
            });
            info.writable |= seg.writable;
            info.executable |= seg.executable;
            page += PAGE;
        }
    }

    // Pass 2: allocate, zero, and map each page.
    let hhdm = arch::hhdm_offset();
    for (&page, info) in &mut pages {
        let phys = frame::alloc().ok_or(ElfError::NoFrame)?;
        // SAFETY: freshly allocated frame, reachable through the HHDM for a full
        // page; nothing else aliases it yet.
        unsafe {
            core::ptr::write_bytes((phys + hhdm) as *mut u8, 0, PAGE as usize);
        }
        if !arch::vspace_map(space, page, phys, info.writable, info.executable) {
            return Err(ElfError::MapFailed);
        }
        info.phys = phys;
    }

    // Pass 3: copy each segment's file bytes into the mapped pages.
    for seg in &segments {
        let mut copied = 0usize;
        while copied < seg.filesz {
            let va = seg.vaddr + copied as u64;
            let page = va & !(PAGE - 1);
            let page_off = (va - page) as usize;
            let n = core::cmp::min(PAGE as usize - page_off, seg.filesz - copied);
            let phys = pages.get(&page).ok_or(ElfError::Malformed)?.phys;
            let src_off = seg.offset + copied;
            let src = image
                .get(src_off..src_off + n)
                .ok_or(ElfError::Malformed)?;
            // SAFETY: `phys` is a mapped frame; `page_off + n <= PAGE`.
            unsafe {
                let dst = ((phys + hhdm) as *mut u8).add(page_off);
                core::ptr::copy_nonoverlapping(src.as_ptr(), dst, n);
            }
            copied += n;
        }
    }

    Ok(entry)
}
