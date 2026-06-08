//! Endpoint objects — the rendezvous points for inter-process IPC.
//!
//! An Endpoint is a kernel object two user processes meet on: one SENDs a
//! message word (and, from stage 2, a capability), another RECVs it. A process
//! names an endpoint by an `Endpoint` capability whose `object` is the endpoint
//! id — there is no ambient way to reach an endpoint.
//!
//! This first cut is a FIFO queue with a busy-yield receive: the receiver polls
//! and yields the CPU until a message arrives (the loop lives in the syscall
//! handler so it can drive the process scheduler). Same shape as the
//! milestone-2.0 in-kernel channel, but between Ring-3 processes and able to
//! carry a capability. Real blocking IPC with wait queues comes later.

use alloc::collections::VecDeque;

use spin::Mutex;

use crate::cap::CapEntry;

/// Number of system endpoints. A small fixed table keeps endpoint ids simple;
/// the delegation demo uses endpoint 0.
pub const NUM_ENDPOINTS: usize = 4;

struct Endpoint {
    queue: VecDeque<(u64, Option<CapEntry>)>,
}

impl Endpoint {
    const fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }
}

static ENDPOINTS: [Mutex<Endpoint>; NUM_ENDPOINTS] =
    [const { Mutex::new(Endpoint::new()) }; NUM_ENDPOINTS];

/// Enqueue `(word, cap)` on endpoint `id`. Returns false if `id` is invalid.
pub fn send(id: usize, word: u64, cap: Option<CapEntry>) -> bool {
    let Some(ep) = ENDPOINTS.get(id) else {
        return false;
    };
    ep.lock().queue.push_back((word, cap));
    true
}

/// Try to dequeue one message from endpoint `id`. Returns `None` if the queue is
/// empty or `id` is invalid. The blocking (busy-yield) loop lives in the syscall
/// handler so the receiver can yield the process scheduler between polls.
pub fn try_recv(id: usize) -> Option<(u64, Option<CapEntry>)> {
    ENDPOINTS.get(id)?.lock().queue.pop_front()
}
