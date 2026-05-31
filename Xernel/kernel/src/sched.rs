//! Cooperative round-robin scheduler for kernel threads.
//!
//! This is the bootstrap scheduler: threads run until they call [`yield_now`],
//! at which point control passes to the next ready thread. Preemption via the
//! LAPIC timer is a later refinement; cooperative scheduling is enough to bring
//! up the first multi-threaded services and validate context switching.
//!
//! Threads are created before [`start`] and never destroyed yet, so the thread
//! table never reallocates — raw stack-pointer references handed to the context
//! switch stay valid across the unlocked switch window.

use alloc::vec;
use alloc::vec::Vec;

use spin::Mutex;

use crate::arch;

const STACK_WORDS: usize = 8192; // 64 KiB per thread

struct Thread {
    id: u64,
    _stack: Vec<u64>,
    rsp: u64,
}

struct Scheduler {
    threads: Vec<Thread>,
    current: usize,
    next_id: u64,
}

impl Scheduler {
    const fn new() -> Self {
        Self {
            threads: Vec::new(),
            current: 0,
            next_id: 0,
        }
    }
}

static SCHED: Mutex<Scheduler> = Mutex::new(Scheduler::new());

/// Create a kernel thread that will begin at `entry`. Call before [`start`].
pub fn spawn(entry: extern "C" fn() -> !) -> u64 {
    let mut stack = vec![0u64; STACK_WORDS];
    let rsp = arch::init_thread_stack(&mut stack, entry);
    let mut sched = SCHED.lock();
    let id = sched.next_id;
    sched.next_id += 1;
    sched.threads.push(Thread {
        id,
        _stack: stack,
        rsp,
    });
    id
}

/// Id of the currently running thread.
pub fn current_id() -> u64 {
    let sched = SCHED.lock();
    sched.threads[sched.current].id
}

/// Begin scheduling. Switches into the first spawned thread and never returns
/// to the caller (the boot context is abandoned).
pub fn start() -> ! {
    let first_rsp = {
        let sched = SCHED.lock();
        assert!(!sched.threads.is_empty(), "scheduler: no threads to run");
        sched.threads[0].rsp
    };
    let mut discard: u64 = 0;
    // SAFETY: `first_rsp` was produced by `init_thread_stack`; `discard` is
    // valid scratch storage for the abandoned boot context.
    unsafe { arch::switch_context(&mut discard, first_rsp) };
    unreachable!("returned to abandoned boot context");
}

/// Yield the CPU to the next ready thread.
pub fn yield_now() {
    let (save_ptr, next_rsp) = {
        let mut sched = SCHED.lock();
        let n = sched.threads.len();
        if n < 2 {
            return;
        }
        let prev = sched.current;
        let next = (prev + 1) % n;
        sched.current = next;
        let save_ptr = core::ptr::addr_of_mut!(sched.threads[prev].rsp);
        let next_rsp = sched.threads[next].rsp;
        (save_ptr, next_rsp)
    };
    // Lock released before switching: the thread we resume will lock again.
    // SAFETY: the thread table never reallocates (no spawning after start), so
    // `save_ptr` stays valid; `next_rsp` is a saved stack pointer.
    unsafe { arch::switch_context(save_ptr, next_rsp) };
}
