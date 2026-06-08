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
/// CNode slot in which every process holds its `Endpoint` capability for the
/// delegation demo. Both the root and its child are seeded with endpoint 0 here.
const EP_SLOT: usize = 3;
const USER_STACK_VA: u64 = 0x80_0000;
const USER_STACK_PAGES: u64 = 16;
const HEAP_START: u64 = 0x1000_0000;
const HEAP_MAX: u64 = 0x2000_0000;
const KSTACK_WORDS: usize = 4096; // 32 KiB kernel stack per process
/// How many processes to start from the init module at boot. Real systems boot
/// exactly ONE init, which then spawns its own children; launching several
/// copies that fight over the framebuffer breaks real userland. We boot TWO for
/// the delegation demo: the same init binary takes a role by its PID — pid 0 is
/// the root/driver host (does the device work), pid 1 is a minimal child that
/// only participates in the IPC/delegation demo and never touches the
/// framebuffer, so they do not collide. A real system would have the root
/// *spawn* the child; that (and a spawn syscall) comes later.
const NUM_PROCESSES: u64 = 2;

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
    caps: CNode, // this process's capability space (its only authority)
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
    // Both the root and its child share endpoint 0 so they can rendezvous; this
    // is the one capability the child starts with. Everything else it gains only
    // by delegation over that endpoint.
    let _ = caps.insert(EP_SLOT, CapEntry::endpoint(0));
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

/// Read (a copy of) the capability in slot `slot` of the current process, for
/// granting it over an endpoint. `None` if the slot is empty/out of range.
pub fn current_cap_get(slot: usize) -> Option<CapEntry> {
    let guard = SCHED.lock();
    let s = guard.as_ref()?;
    s.procs[s.current].caps.get(slot).ok()
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
