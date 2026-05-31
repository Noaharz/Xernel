//! Loading and entering the first user-space program.
//!
//! The program (`init`) is a separately compiled ELF executable, delivered as a
//! Limine boot module. The kernel reads the module bytes, hands them to the ELF
//! loader (which maps the PT_LOAD segments into ring-3-accessible pages), maps a
//! user stack, and drops to ring 3 at the ELF entry point.
//!
//! This replaces the earlier hand-assembled byte blob: from here on, user
//! programs are real, compiled artifacts. A proper loader living in a root
//! server (rather than the kernel) is the Phase-3 evolution of this.

use spin::Mutex;

use crate::{arch, elf, mm::frame, println};

const USER_STACK_VA: u64 = 0x80_0000; // 8 MiB, clear of the init image at 4 MiB
const PAGE: u64 = 4096;
const USER_STACK_PAGES: u64 = 16; // 64 KiB — room for a real program's call stack

// User heap region, grown on demand by `sbrk`. Placed well above the init image
// (4 MiB) and stack (8 MiB) so the regions never collide.
const HEAP_START: u64 = 0x1000_0000; // 256 MiB
const HEAP_MAX: u64 = 0x2000_0000; // 512 MiB — address ceiling (frames cap real size)

/// Current program break (top of the user heap). Single process for now, so one
/// global value suffices; this becomes per-process state once we have many.
static HEAP_BREAK: Mutex<u64> = Mutex::new(HEAP_START);

fn page_up(addr: u64) -> u64 {
    (addr + PAGE - 1) & !(PAGE - 1)
}

/// Adjust the program break by `delta` bytes (Unix `sbrk` semantics) and return
/// the PREVIOUS break, or `None` on failure. Growing maps fresh, zeroed,
/// user-writable pages on demand; `delta == 0` just queries the current break.
/// Shrinking lowers the break without unmapping (kept simple and safe).
pub fn sbrk(delta: i64) -> Option<u64> {
    let mut brk = HEAP_BREAK.lock();
    let old = *brk;
    if delta == 0 {
        return Some(old);
    }
    let new = if delta > 0 {
        old.checked_add(delta as u64)?
    } else {
        old.checked_sub(delta.unsigned_abs())?
    };
    if new < HEAP_START || new > HEAP_MAX {
        return None;
    }
    if delta > 0 {
        // Map the pages newly covered by the grown break: [page_up(old), page_up(new)).
        let hhdm = arch::hhdm_offset();
        let mut page = page_up(old);
        while page < page_up(new) {
            let phys = frame::alloc()?;
            // SAFETY: freshly allocated frame, reachable through the HHDM; zero
            // it so userland never sees stale data.
            unsafe {
                core::ptr::write_bytes((phys + hhdm) as *mut u8, 0, PAGE as usize);
            }
            if !arch::map_user(page, phys, true, false) {
                return None;
            }
            page += PAGE;
        }
    }
    *brk = new;
    Some(old)
}

/// Load `init` from its boot module and enter ring 3. Never returns.
pub fn run() -> ! {
    arch::init_syscalls();

    let module = arch::init_module().expect("init boot module missing");
    println!("[xernel] init module: {} bytes", module.len());

    let entry = match elf::load(module) {
        Ok(entry) => entry,
        Err(e) => panic!("failed to load init ELF: {e:?}"),
    };

    // User stack: a run of readable/writable, never-executable pages.
    for i in 0..USER_STACK_PAGES {
        let phys = frame::alloc().expect("no frame for user stack");
        assert!(
            arch::map_user(USER_STACK_VA + i * PAGE, phys, true, false),
            "mapping user stack failed"
        );
    }
    // SysV AMD64 ABI: at a function's entry `rsp % 16 == 8` (as if reached via
    // `call`, which pushed an 8-byte return address). We jump straight into
    // `_start`, so bias the 16-byte-aligned stack top by 8 to match. Without
    // this, SSE code that uses aligned moves (`movaps`) on 16-byte-aligned stack
    // locals faults with #GP — invisible with a soft-float userland, fatal once
    // SSE is in play.
    let user_stack_top = (USER_STACK_VA + USER_STACK_PAGES * PAGE) - 8;

    println!("[xernel] entering ring 3: entry={entry:#x}");
    // SAFETY: the ELF segments and the stack are mapped user-accessible above,
    // and the syscall MSRs were initialised by `arch::init_syscalls`.
    unsafe { arch::enter_user(entry, user_stack_top) }
}
