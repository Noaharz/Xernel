//! Processes with isolated address spaces and cooperative scheduling.
//!
//! Each process owns:
//!   - a private address space (its own page table, via `arch::vspace_*`),
//!   - a user stack and heap mapped only in that space,
//!   - a **kernel thread**: a kernel stack plus a saved kernel stack pointer,
//!     so the process's in-kernel state survives while another process runs.
//!
//! A process runs in ring 3 until it makes a syscall. `SYS_YIELD` switches to
//! another process; `SYS_EXIT` ends it and runs the next. Switching means:
//! change CR3 (address space), repoint the per-process syscall kernel stack,
//! then context-switch the kernel thread (reusing `arch::switch_context`, the
//! same primitive the milestone-2.0 kernel threads used). This is cooperative —
//! a process yields voluntarily; timer-driven preemption is the next step.

use alloc::vec;
use alloc::vec::Vec;

use spin::Mutex;

use crate::{arch, elf, println};

const PAGE: u64 = 4096;
const USER_STACK_VA: u64 = 0x80_0000;
const USER_STACK_PAGES: u64 = 16;
const HEAP_START: u64 = 0x1000_0000;
const HEAP_MAX: u64 = 0x2000_0000;
const KSTACK_WORDS: usize = 4096; // 32 KiB kernel stack per process
const NUM_PROCESSES: u64 = 3;

#[derive(PartialEq, Eq)]
enum State {
    Ready,
    Done,
}

struct Process {
    pid: u64,
    space: u64,
    entry: u64,
    user_stack_top: u64,
    heap_break: u64,
    _kstack: Vec<u64>, // owns the kernel stack memory
    ksp: u64,          // saved kernel stack pointer (for context switch)
    kstack_top: u64,   // top of the kernel stack (for syscall entry)
    state: State,
}

struct Scheduler {
    procs: Vec<Process>,
    current: usize,
}

static SCHED: Mutex<Option<Scheduler>> = Mutex::new(None);

fn page_up(x: u64) -> u64 {
    (x + PAGE - 1) & !(PAGE - 1)
}

/// First-run entry of a process's kernel thread: the scheduler has already made
/// this process current (CR3, kernel stack set), so just drop into its user
/// space.
extern "C" fn trampoline() -> ! {
    let (entry, stack_top) = {
        let guard = SCHED.lock();
        let s = guard.as_ref().expect("no scheduler");
        let p = &s.procs[s.current];
        (p.entry, p.user_stack_top)
    };
    // SAFETY: CR3 is this process's space; entry/stack are user-mapped; syscall
    // MSRs initialised.
    unsafe { arch::enter_user(entry, stack_top) }
}

fn create(pid: u64, module: &[u8]) -> Option<Process> {
    let space = arch::vspace_new()?;
    let entry = elf::load(module, space).ok()?;
    for i in 0..USER_STACK_PAGES {
        if !arch::vspace_alloc_map(space, USER_STACK_VA + i * PAGE, true, false) {
            return None;
        }
    }
    let user_stack_top = (USER_STACK_VA + USER_STACK_PAGES * PAGE) - 8;
    let mut kstack = vec![0u64; KSTACK_WORDS];
    let ksp = arch::init_thread_stack(&mut kstack, trampoline);
    let kstack_top = kstack.as_ptr_range().end as u64 & !0xf;
    Some(Process {
        pid,
        space,
        entry,
        user_stack_top,
        heap_break: HEAP_START,
        _kstack: kstack,
        ksp,
        kstack_top,
        state: State::Ready,
    })
}

/// Make process at index `i` the active one: switch its address space and
/// syscall kernel stack. Returns its saved kernel stack pointer. Caller must
/// hold the scheduler lock; the actual context switch happens after releasing
/// it.
fn activate(s: &mut Scheduler, i: usize) -> u64 {
    s.current = i;
    let p = &s.procs[i];
    arch::set_kernel_stack(p.kstack_top);
    // SAFETY: `p.space` is a valid address space (kernel half shared). We are
    // running on the kernel stack in the shared higher half, so the CR3 change
    // keeps our code and stack mapped.
    unsafe { arch::vspace_switch(p.space) };
    p.ksp
}

/// Index of the next non-done process after `current` (round-robin), or `None`.
fn pick_next(s: &Scheduler) -> Option<usize> {
    let n = s.procs.len();
    (1..=n)
        .map(|off| (s.current + off) % n)
        .find(|&i| s.procs[i].state != State::Done)
}

/// Create the processes and start running them. Never returns.
pub fn run() -> ! {
    arch::init_syscalls();
    let module = arch::init_module().expect("init boot module missing");
    println!("[xernel] init module: {} bytes", module.len());

    let mut procs = Vec::new();
    for pid in 0..NUM_PROCESSES {
        let p = create(pid, module).expect("failed to create process");
        println!(
            "[xernel] process {} ready: cr3={:#x} entry={:#x}",
            pid, p.space, p.entry
        );
        procs.push(p);
    }

    let first_ksp = {
        let mut guard = SCHED.lock();
        *guard = Some(Scheduler { procs, current: 0 });
        activate(guard.as_mut().unwrap(), 0)
    };
    let mut discard = 0u64;
    // SAFETY: `first_ksp` was prepared by `init_thread_stack` to start at
    // `trampoline`; the boot context is abandoned.
    unsafe { arch::switch_context(&mut discard, first_ksp) };
    unreachable!("returned to abandoned boot context");
}

/// Yield the CPU to the next ready process.
pub fn yield_now() {
    let (save_ptr, next_ksp) = {
        let mut guard = SCHED.lock();
        let s = guard.as_mut().expect("no scheduler");
        let next = match pick_next(s) {
            Some(i) if i != s.current => i,
            _ => return, // nobody else to run
        };
        let prev = s.current;
        let save_ptr = core::ptr::addr_of_mut!(s.procs[prev].ksp);
        let next_ksp = activate(s, next);
        (save_ptr, next_ksp)
    };
    // SAFETY: both stack pointers belong to processes whose kernel stacks live
    // in the shared higher half; the table never reallocates after `run`.
    unsafe { arch::switch_context(save_ptr, next_ksp) };
}

/// Terminate the current process and run the next. Never returns.
pub fn exit(code: u64) -> ! {
    let next_ksp = {
        let mut guard = SCHED.lock();
        let s = guard.as_mut().expect("no scheduler");
        let pid = s.procs[s.current].pid;
        s.procs[s.current].state = State::Done;
        println!("[user pid {pid}] exit({code})");
        pick_next(s).map(|i| activate(s, i))
    };
    if let Some(ksp) = next_ksp {
        let mut discard = 0u64;
        // SAFETY: switching to a valid process kernel stack; the dying process
        // is abandoned (its context is not saved).
        unsafe { arch::switch_context(&mut discard, ksp) };
        unreachable!("returned to an exited process");
    }
    println!("[xernel] all processes exited.");
    #[cfg(feature = "boot-test")]
    {
        println!("[xernel] boot-test: ok");
        arch::exit(true);
    }
    #[cfg(not(feature = "boot-test"))]
    arch::halt_forever();
}

/// PID of the currently running process.
pub fn getpid() -> u64 {
    SCHED.lock().as_ref().map_or(0, |s| s.procs[s.current].pid)
}

/// Adjust the current process's heap break; new pages map into its own space.
pub fn sbrk(delta: i64) -> Option<u64> {
    let mut guard = SCHED.lock();
    let s = guard.as_mut()?;
    let cur = s.current;
    let old = s.procs[cur].heap_break;
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
        let space = s.procs[cur].space;
        let mut page = page_up(old);
        while page < page_up(new) {
            if !arch::vspace_alloc_map(space, page, true, false) {
                return None;
            }
            page += PAGE;
        }
    }
    s.procs[cur].heap_break = new;
    Some(old)
}
