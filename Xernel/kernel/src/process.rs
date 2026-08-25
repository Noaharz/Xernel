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
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use spin::Mutex;

use xabi::cap::CapType;

use crate::cap::{CapEntry, CNode};
use crate::{arch, elf, println};

const PAGE: u64 = 4096;
/// Capability-table size for a process.
const CAP_SLOTS: usize = 64;
/// Highest user virtual address (exclusive). Used to validate user pointers.
const USER_ADDR_MAX: u64 = 0x0000_8000_0000_0000;
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
    /// Pointer to the environment block in the child's address space (HEAP_START).
    /// `0` means no environment was provided.
    envp_user: u64,
    /// Length of the environment block in bytes.
    envp_len: u64,
    /// Ring buffer capturing this process's stdout/stderr output. Readable by
    /// any process that knows this PID (via `SYS_LOG_READ`). 64 KiB per process.
    log_buf: VecDeque<u8>,
    /// Exit code passed to `exit()` or set by `kill()`. Only meaningful when
    /// `state == Done`. Convention: 0 = normal, 15 = SIGTERM, 9 = SIGKILL.
    exit_code: u64,
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
    let mut log_buf = VecDeque::new();
    log_buf.reserve_exact(65536);
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
        envp_user: 0,
        envp_len: 0,
        log_buf,
        exit_code: 0,
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
/// The PID named by the `Process` capability in slot `slot` of the current
/// process, or `None` if that slot holds no such capability. This is the single
/// gate for every operation one process performs on another: no capability, no
/// authority — not even to ask whether the target is alive.
pub fn current_process_pid(slot: usize) -> Option<u64> {
    let guard = SCHED.lock();
    let s = guard.as_ref()?;
    s.procs[s.current].caps.get(slot).ok()?.process_pid()
}

/// Copy `cap` into slot `dst_slot` of the process `target_pid`. Backs
/// `SYS_CAP_GRANT`: it is how a parent equips a child with authority it did not
/// get from `seed_caps` — before or after the child first runs, which is what
/// makes a freshly spawned program useful at all. Returns `false` if the PID is
/// unknown or the destination slot is occupied or out of range (capabilities
/// are never silently overwritten).
pub fn grant_to(target_pid: u64, cap: CapEntry, dst_slot: usize) -> bool {
    let mut guard = SCHED.lock();
    let Some(s) = guard.as_mut() else {
        return false;
    };
    let Some(p) = s.procs.iter_mut().find(|p| p.pid == target_pid) else {
        return false;
    };
    p.caps.insert(dst_slot, cap).is_ok()
}

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
pub fn spawn(_module_index: u64, cap_slot: u64) -> Option<u64> {
    let module = arch::init_module()?;
    let mut guard = SCHED.lock();
    let s = guard.as_mut()?;
    let pid = s.next_pid;
    let p = create(pid, module)?;
    install_process_cap(s, pid, cap_slot)?;
    s.procs.push(Box::new(p));
    s.next_pid += 1;
    Some(pid)
}

/// Hand the spawning process a `Process` capability for its new child. Called
/// with the scheduler already locked, before the child is pushed, so a bad
/// destination slot aborts the spawn instead of leaving a process nobody holds
/// a handle to. `cap_slot == u64::MAX` means the caller deliberately wants no
/// handle — a fire-and-forget child it can never touch again.
fn install_process_cap(s: &mut Scheduler, pid: u64, cap_slot: u64) -> Option<()> {
    if cap_slot == u64::MAX {
        return Some(());
    }
    let cur = s.current;
    s.procs[cur]
        .caps
        .insert(cap_slot as usize, CapEntry::process(pid))
        .ok()
}

/// Like [`spawn`], but also copies an environment block from the parent's
/// address space into the child. `envp_ptr`/`envp_len` point into the
/// **caller's** (parent's) address space; the data is copied page-by-page into
/// the child's heap at `HEAP_START`. The child can later retrieve the pointer
/// via `SYS_GETENVP`. Returns the new PID, or `u64::MAX` on failure.
pub fn spawn_env(_module_index: u64, envp_ptr: u64, envp_len: u64, cap_slot: u64) -> Option<u64> {
    if envp_len == 0 || envp_ptr == 0 {
        return spawn(_module_index, cap_slot);
    }
    let module = arch::init_module()?;
    let mut guard = SCHED.lock();
    let s = guard.as_mut()?;
    let pid = s.next_pid;
    let mut p = create(pid, module)?;

    // The source is a user pointer in the PARENT's (active) address space, so it
    // gets the same treatment as any other: in range AND actually mapped.
    let parent_end = envp_ptr.checked_add(envp_len)?;
    if parent_end >= USER_ADDR_MAX || !arch::user_range_ok(envp_ptr, envp_len, false) {
        return None;
    }

    // Copy the environment into the child's freshly created heap. The child is
    // not running yet, so its heap address is meaningless as a CPU address here:
    // each page has to be translated through the CHILD's tables and written
    // through the HHDM. (Treating the child's virtual address as physical is
    // what 0.26.0 did — it scribbled over whatever RAM sat at that physical
    // address and the child read zeros.)
    let env_pages = envp_len.div_ceil(PAGE);
    for i in 0..env_pages {
        let child_va = HEAP_START + i * PAGE;
        if !arch::vspace_alloc_map(p.space, child_va, true, false) {
            return None; // couldn't map child page
        }
        // `vspace_alloc_map` hands back a zeroed frame, so only the bytes we
        // actually have need copying; the rest of the page stays zero.
        let dst = arch::vspace_phys(p.space, child_va)? + arch::hhdm_offset();
        let start = i * PAGE;
        let n = (envp_len - start).min(PAGE);
        for off in 0..n {
            // SAFETY: the source page was verified mapped in the active address
            // space; the destination is the child's own frame, reached through
            // the HHDM, which maps all physical memory.
            unsafe {
                let byte = core::ptr::read_volatile((envp_ptr + start + off) as *const u8);
                core::ptr::write_volatile((dst + off) as *mut u8, byte);
            }
        }
    }
    p.envp_user = HEAP_START;
    p.envp_len = envp_len;
    // Adjust heap break past the environment block so SBRK starts after it.
    p.heap_break = HEAP_START + env_pages * PAGE;

    install_process_cap(s, pid, cap_slot)?;
    s.procs.push(Box::new(p));
    s.next_pid += 1;
    Some(pid)
}

/// Query the state of the process identified by `target_pid`. Returns:
///   0 = running (Ready or Blocked)
///   1 = exited (Done)
///   2 = unknown PID
pub fn get_status(target_pid: u64) -> u64 {
    let guard = SCHED.lock();
    let s = match guard.as_ref() {
        Some(s) => s,
        None => return 2,
    };
    for p in s.procs.iter() {
        if p.pid == target_pid {
            return match p.state {
                State::Ready | State::Blocked(_) => 0,
                State::Done => 1,
            };
        }
    }
    2 // PID not found
}

/// Return the (user virtual address, length) of the environment block for the
/// current process, or `(0, 0)` if none was set.
pub fn current_getenvp() -> (u64, u64) {
    let guard = SCHED.lock();
    guard
        .as_ref()
        .map_or((0, 0), |s| (s.procs[s.current].envp_user, s.procs[s.current].envp_len))
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

/// Exit code recorded for a process the kernel terminated after a fatal
/// user-mode CPU exception. Outside the range a program can pass to `exit()`,
/// so `WAIT_PID` can tell "crashed" from "chose to exit with this code".
pub const EXIT_FAULT: u64 = 0x100;

/// Terminate the current process and run the next. Never returns.
pub fn exit(code: u64) -> ! {
    exit_inner(code, None)
}

/// Terminate the current process because it took a fatal CPU exception **in
/// ring 3**, and keep the kernel running. This is what makes "a driver crash is
/// not a kernel panic" true rather than aspirational: the faulting process is
/// the only casualty, its parent observes [`EXIT_FAULT`] through `WAIT_PID`.
///
/// Only ever called for a fault whose saved `CS` says ring 3 — a ring-0 fault
/// is a kernel bug and must still panic, because the kernel's own invariants
/// are already broken at that point.
pub fn fault_exit(what: &str) -> ! {
    exit_inner(EXIT_FAULT, Some(what))
}

fn exit_inner(code: u64, fault: Option<&str>) -> ! {
    let next_ksp = {
        let mut guard = SCHED.lock();
        let s = guard.as_mut().expect("no scheduler");
        let pid = s.procs[s.current].pid;
        s.procs[s.current].exit_code = code;
        s.procs[s.current].state = State::Done;
        match fault {
            Some(what) => println!("[user pid {pid}] killed by {what}"),
            None => println!("[user pid {pid}] exit({code})"),
        }
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

/// Maximum bytes kept in a process's log ring buffer (64 KiB).
const LOG_BUF_CAP: usize = 65536;

/// Append `data` to the current process's log ring buffer, dropping the oldest
/// bytes when the buffer is full. Called from `sys_write` when `fd` is 1 or 2.
pub fn log_write(data: &[u8]) {
    let mut guard = SCHED.lock();
    if let Some(s) = guard.as_mut() {
        let p = &mut s.procs[s.current];
        for &b in data {
            if p.log_buf.len() >= LOG_BUF_CAP {
                p.log_buf.pop_front();
            }
            p.log_buf.push_back(b);
        }
    }
}

/// Copy up to `max` bytes from the log ring buffer of the process identified by
/// `target_pid` into `out`, returning how many bytes were copied and removing
/// them from the buffer. Reaching this function at all requires a `Process`
/// capability for the target — a process's console output is its own.
pub fn log_read(target_pid: u64, out: &mut [u8], max: usize) -> u64 {
    let mut guard = SCHED.lock();
    let s = match guard.as_mut() {
        Some(s) => s,
        None => return 0,
    };
    let p = match s.procs.iter_mut().find(|p| p.pid == target_pid) {
        Some(p) => p,
        None => return 0,
    };
    let n = core::cmp::min(max, p.log_buf.len());
    for i in 0..n {
        if let Some(b) = p.log_buf.pop_front() {
            out[i] = b;
        }
    }
    n as u64
}

/// Send a signal to a process identified by `target_pid`.
/// - signal 15 (SIGTERM): mark the process as Done with exit_code 15.
/// - signal 9  (SIGKILL): same, but exit_code 9.
/// Returns 0 on success, `u64::MAX` if the PID is unknown or already exited.
/// The caller's authority was already checked by `sys_kill`, which resolves a
/// `Process` capability to this PID — there is no path here from a bare number.
pub fn kill(target_pid: u64, signal: u64) -> u64 {
    let mut guard = SCHED.lock();
    let s = match guard.as_mut() {
        Some(s) => s,
        None => return u64::MAX,
    };
    // Don't kill PID 0 (the root/init) — the system depends on it.
    if target_pid == 0 {
        return u64::MAX;
    }
    let p = match s.procs.iter_mut().find(|p| p.pid == target_pid) {
        Some(p) => p,
        None => return u64::MAX,
    };
    match p.state {
        State::Done => return u64::MAX, // already exited
        _ => {}
    }
    let code = match signal {
        9 => 9,   // SIGKILL
        15 => 15, // SIGTERM
        other => other, // arbitrary exit code
    };
    p.exit_code = code;
    p.state = State::Done;
    println!("[kernel] kill(pid={}, sig={}) → exit_code={}", target_pid, signal, code);
    0
}

/// Block until the process identified by `target_pid` has exited, then return
/// its exit code. Returns `u64::MAX` if the PID is unknown or is the caller
/// itself. If the target has already exited, returns immediately.
pub fn wait_pid(target_pid: u64) -> u64 {
    if target_pid == 0 {
        return u64::MAX;
    }
    // Busy-wait with yield: check, yield, repeat. Not elegant, but correct for
    // single-tenant where children exit quickly.
    loop {
        {
            let guard = SCHED.lock();
            let s = match guard.as_ref() {
                Some(s) => s,
                None => return u64::MAX,
            };
            // Don't wait for yourself.
            if s.procs[s.current].pid == target_pid {
                return u64::MAX;
            }
            if let Some(p) = s.procs.iter().find(|p| p.pid == target_pid) {
                if p.state == State::Done {
                    return p.exit_code;
                }
            } else {
                return u64::MAX; // PID unknown
            }
        }
        // Release lock, yield, then re-check.
        crate::process::yield_now();
    }
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
