//! Physical frame allocator.
//!
//! Fed with the usable physical-memory regions discovered by the architecture
//! layer. Hands out 4 KiB frames by linear bump within the regions, with a
//! free list so reclaimed frames are reused. This is deliberately simple: it is
//! enough to bootstrap paging, the kernel heap, and (later) the capability
//! `Untyped` allocator, which will take over bulk physical-memory policy.

use alloc::vec::Vec;

use spin::Mutex;

pub const FRAME_SIZE: u64 = 4096;

/// A half-open physical range `[start, end)`, in bytes.
#[derive(Copy, Clone, Debug)]
pub struct Region {
    pub start: u64,
    pub end: u64,
}

impl Region {
    pub const fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }
}

pub struct PhysFrameAllocator {
    regions: Vec<Region>,
    region_idx: usize,
    cursor: u64,
    free: Vec<u64>,
    total_frames: u64,
    in_use: u64,
}

impl PhysFrameAllocator {
    fn new(mut regions: Vec<Region>) -> Self {
        for r in &mut regions {
            r.start = (r.start + FRAME_SIZE - 1) & !(FRAME_SIZE - 1);
            r.end &= !(FRAME_SIZE - 1);
        }
        regions.retain(|r| r.end > r.start);
        let total_frames = regions.iter().map(|r| (r.end - r.start) / FRAME_SIZE).sum();
        let cursor = regions.first().map_or(0, |r| r.start);
        Self {
            regions,
            region_idx: 0,
            cursor,
            free: Vec::new(),
            total_frames,
            in_use: 0,
        }
    }

    /// Allocate one frame, returning its physical base address.
    pub fn alloc(&mut self) -> Option<u64> {
        if let Some(pa) = self.free.pop() {
            self.in_use += 1;
            return Some(pa);
        }
        while self.region_idx < self.regions.len() {
            let region = self.regions[self.region_idx];
            if self.cursor + FRAME_SIZE <= region.end {
                let pa = self.cursor;
                self.cursor += FRAME_SIZE;
                self.in_use += 1;
                return Some(pa);
            }
            self.region_idx += 1;
            if let Some(next) = self.regions.get(self.region_idx) {
                self.cursor = next.start;
            }
        }
        None
    }

    /// Allocate `n` physically *contiguous* frames, returning the base address.
    /// Taken only from the bump cursor (never the free list), so the run is
    /// guaranteed contiguous. Needed for DMA buffers / virtqueues.
    pub fn alloc_contiguous(&mut self, n: u64) -> Option<u64> {
        if n == 0 {
            return None;
        }
        while self.region_idx < self.regions.len() {
            let region = self.regions[self.region_idx];
            if self.cursor + n * FRAME_SIZE <= region.end {
                let pa = self.cursor;
                self.cursor += n * FRAME_SIZE;
                self.in_use += n;
                return Some(pa);
            }
            self.region_idx += 1;
            if let Some(next) = self.regions.get(self.region_idx) {
                self.cursor = next.start;
            }
        }
        None
    }

    /// Return a previously allocated frame to the pool.
    pub fn free(&mut self, pa: u64) {
        self.free.push(pa);
        self.in_use = self.in_use.saturating_sub(1);
    }

    /// `(frames_in_use, total_frames)`.
    pub fn stats(&self) -> (u64, u64) {
        (self.in_use, self.total_frames)
    }
}

static ALLOCATOR: Mutex<Option<PhysFrameAllocator>> = Mutex::new(None);

/// Initialise the global frame allocator from the usable regions. Requires the
/// kernel heap to be live (it allocates `Vec`s internally).
pub fn init(regions: impl Iterator<Item = Region>) {
    let regions: Vec<Region> = regions.collect();
    *ALLOCATOR.lock() = Some(PhysFrameAllocator::new(regions));
}

pub fn alloc() -> Option<u64> {
    ALLOCATOR.lock().as_mut().and_then(PhysFrameAllocator::alloc)
}

/// Allocate `n` physically contiguous frames; returns the base physical address.
pub fn alloc_contiguous(n: u64) -> Option<u64> {
    ALLOCATOR
        .lock()
        .as_mut()
        .and_then(|a| a.alloc_contiguous(n))
}

pub fn free(pa: u64) {
    if let Some(a) = ALLOCATOR.lock().as_mut() {
        a.free(pa);
    }
}

pub fn stats() -> (u64, u64) {
    ALLOCATOR
        .lock()
        .as_ref()
        .map_or((0, 0), PhysFrameAllocator::stats)
}
