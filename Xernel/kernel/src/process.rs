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

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use spin::Mutex;

use xabi::cap::CapType;

use crate::cap::{CapEntry, CNode};
use crate::{arch, elf, println};

const PAGE: u64 = 4096;
/// Capability-table size for a process.
const CAP_SLOTS: usize = 64;
/// PCI I/O-BAR window on the QEMU q35 machine. The root driver task is granted
/// an `IoPort` capability over exactly this range — enough to reach virtio
/// devices' legacy registers, but not the low system ports (PIC, PIT, CMOS, …).
const PCI_IO_BASE: u16 = 0xc000;
const PCI_IO_COUNT: u16 = 0x4000; // [0xc000, 0x10000)
/// PCI memory-BAR window (the 32-bit MMIO hole on q35). The root driver task is
/// granted an `IoMem` capability over exactly this range — it covers device
/// BARs but NOT real RAM (which lives far below) or the kernel.
const PCI_MMIO_BASE: u64 = 0xc000_0000;
const PCI_MMIO_LEN: u64 = 0x4000_0000; // [0xc000_0000, 0x1_0000_0000)
/// DMA-allocation budget granted to the root driver task as an `Untyped`
/// capability. Generous enough for real virtqueue/request buffers (tens of KiB),
/// but bounded — a driver cannot pin unbounded physical memory for DMA.
const DMA_BUDGET: u64 = 256 * 1024;
/// CNode slots holding the two `Endpoint` capabilities every process is seeded
/// with: endpoint 0 (slot 3) carries requests from a client to a service, and
/// endpoint 1 (slot 4) carries the service's replies back. A request/reply pair
/// of unidirectional endpoints is what lets the file-service (pid 0) answer a
/// client (a spawned process with no device authority) purely over IPC.
const EP_SLOT: usize = 3;
const REPLY_EP_SLOT: usize = 4;
/// CNode slot holding the `Notification` capability every process is seeded with
/// (notification 0) — the async readiness object a service signals and a client
/// waits on.
const NOTIF_SLOT: usize = 5;
const USER_STACK_VA: u64 = 0x80_0000;
const USER_STACK_PAGES: u64 = 16;
const HEAP_START: u64 = 0x1000_0000;
const HEAP_MAX: u64 = 0x2000_0000;
const KSTACK_WORDS: usize = 4096; // 32 KiB kernel stack per process
/// How many processes the kernel starts at boot. Like a real system, the kernel
/// boots exactly ONE init (the root, pid 0); the root then `spawn`s every other
/// process itself (see [`spawn`] / `SYS_SPAWN`). The same init binary takes a
/// role by its PID — pid 0 is the root/driver host (does the device work), any
/// other pid is a minimal child that only participates in the IPC/delegation
/// demo and never touches the framebuffer, so they do not collide.
const NUM_PROCESSES: u64 = 1;

/// Why a process is blocked — i.e. which resource it is waiting on. A blocked
/// process is skipped by the scheduler until something `wake`s exactly this
/// reason (a `SEND` to that endpoint, a `SIGNAL` to that notification).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// Waiting for a message on endpoint `id` (a blocked `RECV`).
    Endpoint(usize),
    /// Waiting for bits on notification `id` (a blocked `WAIT`).
    Notification(usize),
}

#[derive(PartialEq, Eq)]
enum State {
    /// Runnable: the scheduler may switch to it.
    Ready,
    /// Parked inside a syscall, waiting on `BlockReason`. Not runnable until
    /// woken — the scheduler never picks it, so it burns no CPU (unlike the old
    /// busy-yield).
    Blocked(BlockReason),
    /// Exited; kept in the table (PIDs are never reused) but never run again.
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
    caps: CNode, // this process's capability space (its only authority)
}

struct Scheduler {
    /// Processes are **boxed** so a `Process` never moves once created: the
    /// scheduler keeps saved kernel-stack pointers and switches contexts into
    /// these structs, and `spawn` grows this vector at runtime — a reallocation
    /// must not relocate existing processes.
    procs: Vec<Box<Process>>,
    current: usize,
    /// Monotonic PID counter. The kernel boots pid 0; every `spawn` hands out the
    /// next id. Never reused (exited processes stay in `procs`, marked `Done`).
    next_pid: u64,
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
        caps: seed_caps(pid),
    })
}

/// Build a process's initial capability space. The root task (pid 0) is the
/// system's first driver host, so it is granted device authority directly —
/// here, an `IoPort` capability over the PCI I/O window. Every other process
/// starts with an empty CNode and receives authority only by delegation. A more
/// mature system would derive even the root's caps from firmware/a manifest
/// rather than hardcoding them.
fn seed_caps(pid: u64) -> CNode {
    let mut caps = CNode::new(CAP_SLOTS);
    // Every process shares the request/reply endpoint pair so a client and a
    // service can rendezvous; these are the only capabilities a spawned client
    // starts with. Everything else it would gain by delegation over an endpoint.
    let _ = caps.insert(EP_SLOT, CapEntry::endpoint(0));
    let _ = caps.insert(REPLY_EP_SLOT, CapEntry::endpoint(1));
    let _ = caps.insert(NOTIF_SLOT, CapEntry::notification(0));
    if pid == 0 {
        let _ = caps.insert(0, CapEntry::io_port(PCI_IO_BASE, PCI_IO_COUNT));
        let _ = caps.insert(1, CapEntry::io_mem(PCI_MMIO_BASE, PCI_MMIO_LEN));
        let _ = caps.insert(2, CapEntry::untyped(DMA_BUDGET));
    }
    caps
}

/// Does the currently running process hold a capability authorizing a
/// `size`-byte I/O-port access at `port`? The port-I/O syscalls consult this —
/// there is no ambient permission to touch hardware ports.
pub fn current_authorizes_port(port: u16, size: u8) -> bool {
    let guard = SCHED.lock();
    guard
        .as_ref()
        .is_some_and(|s| s.procs[s.current].caps.authorizes_port(port, size))
}

/// Does the currently running process hold a capability authorizing a mapping
/// of the physical range `[phys, phys+len)`? Consulted by `SYS_IOMAP`.
pub fn current_authorizes_mmio(phys: u64, len: u64) -> bool {
    let guard = SCHED.lock();
    guard
        .as_ref()
        .is_some_and(|s| s.procs[s.current].caps.authorizes_mmio(phys, len))
}

/// Charge `amount` bytes against the current process's `Untyped` budget,
/// returning `true` if it had enough. Consulted by `SYS_DMA_ALLOC` — a driver
/// can pin only as much DMA memory as its budget allows.
pub fn current_charge_untyped(amount: u64) -> bool {
    let mut guard = SCHED.lock();
    guard.as_mut().is_some_and(|s| {
        let cur = s.current;
        s.procs[cur].caps.charge_untyped(amount)
    })
}

/// Refund `amount` bytes to the current process's `Untyped` budget, undoing a
/// charge whose allocation later failed.
pub fn current_refund_untyped(amount: u64) {
    let mut guard = SCHED.lock();
    if let Some(s) = guard.as_mut() {
        let cur = s.current;
        s.procs[cur].caps.refund_untyped(amount);
    }
}

/// A normalized description of the capability in slot `slot` of the current
/// process, or `None` if the slot is empty/out of range. Backs
/// `SYS_CAP_IDENTIFY`, letting a process enumerate its own authority.
pub fn current_cap_describe(slot: usize) -> Option<(u8, u64, u64)> {
    let guard = SCHED.lock();
    let s = guard.as_ref()?;
    s.procs[s.current].caps.get(slot).ok().map(|c| c.describe())
}

/// If the current process holds an `Endpoint` capability in slot `slot`, return
/// the endpoint id it names. Backs `SYS_SEND`/`SYS_RECV` — a process can only
/// reach an endpoint it has a capability for.
pub fn current_endpoint_id(slot: usize) -> Option<u64> {
    let guard = SCHED.lock();
    let s = guard.as_ref()?;
    let cap = s.procs[s.current].caps.get(slot).ok()?;
    (cap.cap_type == CapType::Endpoint).then_some(cap.object)
}

/// If the current process holds a `Notification` capability in slot `slot`,
/// return the notification id it names. Backs `SYS_SIGNAL`/`SYS_WAIT` — a process
/// can only reach a notification it has a capability for.
pub fn current_notification_id(slot: usize) -> Option<u64> {
    let guard = SCHED.lock();
    let s = guard.as_ref()?;
    let cap = s.procs[s.current].caps.get(slot).ok()?;
    (cap.cap_type == CapType::Notification).then_some(cap.object)
}

/// If the current process holds a `Frame` capability in slot `slot`, return the
/// physical base and page count it names (`(phys, pages)`). Backs
/// `SYS_MAP_FRAME` — a process can only map a frame it has a capability for.
pub fn current_frame_cap(slot: usize) -> Option<(u64, u64)> {
    let guard = SCHED.lock();
    let s = guard.as_ref()?;
    let cap = s.procs[s.current].caps.get(slot).ok()?;
    (cap.cap_type == CapType::Frame).then_some((cap.object, cap.badge))
}

/// Read (a copy of) the capability in slot `slot` of the current process, for
/// granting it over an endpoint. `None` if the slot is empty/out of range.
pub fn current_cap_get(slot: usize) -> Option<CapEntry> {
    let guard = SCHED.lock();
    let s = guard.as_ref()?;
    s.procs[s.current].caps.get(slot).ok()
}

/// Remove and return the capability in slot `slot` of the current process, or
/// `None` if the slot is empty/out of range. The destroying half of a cap's
/// lifetime — backs `SYS_FRAME_DROP`.
pub fn current_cap_delete(slot: usize) -> Option<CapEntry> {
    let mut guard = SCHED.lock();
    let s = guard.as_mut()?;
    let cur = s.current;
    s.procs[cur].caps.delete(slot).ok()
}

/// Install a delegated capability into slot `slot` of the current process.
/// Returns false if the slot is occupied or out of range (capabilities are
/// never silently overwritten). This is the receiving half of delegation.
pub fn current_cap_install(slot: usize, cap: CapEntry) -> bool {
    let mut guard = SCHED.lock();
    guard.as_mut().is_some_and(|s| {
        let cur = s.current;
        s.procs[cur].caps.insert(slot, cap).is_ok()
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

/// Index of the next **ready** process after `current` (round-robin), or `None`
/// if nobody is runnable. Blocked and Done processes are skipped — this is what
/// makes blocking real: a parked waiter is simply not a candidate.
fn pick_next(s: &Scheduler) -> Option<usize> {
    let n = s.procs.len();
    (1..=n)
        .map(|off| (s.current + off) % n)
        .find(|&i| s.procs[i].state == State::Ready)
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
        procs.push(Box::new(p));
    }

    let first_ksp = {
        let mut guard = SCHED.lock();
        *guard = Some(Scheduler {
            procs,
            current: 0,
            next_pid: NUM_PROCESSES,
        });
        activate(guard.as_mut().unwrap(), 0)
    };
    let mut discard = 0u64;
    // SAFETY: `first_ksp` was prepared by `init_thread_stack` to start at
    // `trampoline`; the boot context is abandoned.
    unsafe { arch::switch_context(&mut discard, first_ksp) };
    unreachable!("returned to abandoned boot context");
}

/// Create a new process at runtime and add it to the scheduler as `Ready`,
/// returning its PID. This is how userland grows the process table: the kernel
/// boots only the root, which `spawn`s every other process. The newcomer runs
/// in its own fresh address space with a freshly seeded capability space
/// (`seed_caps`); it is picked up by the round-robin scheduler the next time the
/// caller yields, blocks, or exits.
///
/// `_module_index` selects which program to launch. Today only the boot init
/// image (index 0) exists, so any value loads it — but the parameter is already
/// part of the ABI so a future root-server can resolve a name to one of several
/// programs. Returns `None` if the image is missing or process creation fails.
pub fn spawn(_module_index: u64) -> Option<u64> {
    let module = arch::init_module()?;
    let mut guard = SCHED.lock();
    let s = guard.as_mut()?;
    let pid = s.next_pid;
    let p = create(pid, module)?;
    s.procs.push(Box::new(p));
    s.next_pid += 1;
    Some(pid)
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
    // in the shared higher half; processes are boxed, so growing the table via
    // `spawn` never relocates them and `save_ptr` stays valid.
    unsafe { arch::switch_context(save_ptr, next_ksp) };
}

/// Block the current process on `reason` and switch to another ready process —
/// the heart of real (non-spinning) blocking. The caller (`sys_recv`/`sys_wait`)
/// must re-check its condition after this returns, because being woken only means
/// the resource *might* now be available (several waiters can race for one
/// message). The process resumes here once a matching [`wake`] makes it `Ready`
/// again and the scheduler picks it.
///
/// If nobody else is runnable, we cannot switch away; we re-mark ourselves
/// `Ready` and return so the caller re-checks. That degrades to the old spin in
/// the degenerate "only process, waiting forever" case (a program deadlock either
/// way) but never parks the CPU with no one to wake it.
pub fn block_on(reason: BlockReason) {
    let (save_ptr, next_ksp) = {
        let mut guard = SCHED.lock();
        let s = guard.as_mut().expect("no scheduler");
        let cur = s.current;
        s.procs[cur].state = State::Blocked(reason);
        let Some(next) = pick_next(s) else {
            s.procs[cur].state = State::Ready;
            return;
        };
        let save_ptr = core::ptr::addr_of_mut!(s.procs[cur].ksp);
        let next_ksp = activate(s, next);
        (save_ptr, next_ksp)
    };
    // SAFETY: see `yield_now` — both kernel stacks live in the shared higher
    // half; boxed processes never relocate, so `save_ptr` stays valid.
    unsafe { arch::switch_context(save_ptr, next_ksp) };
}

/// Wake every process blocked on exactly `reason`, marking it `Ready` so the
/// scheduler may pick it again. Called right after a `SEND` (wakes a blocked
/// `RECV` on that endpoint) or a `SIGNAL` (wakes a blocked `WAIT` on that
/// notification). Waking more than one waiter is fine — each re-checks and the
/// loser simply blocks again.
pub fn wake(reason: BlockReason) {
    let mut guard = SCHED.lock();
    if let Some(s) = guard.as_mut() {
        for p in s.procs.iter_mut() {
            if p.state == State::Blocked(reason) {
                p.state = State::Ready;
            }
        }
    }
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
