//! Notification objects — the asynchronous readiness primitive.
//!
//! A Notification is a kernel object carrying a single word of *signal bits*. A
//! process `signal`s it (the bits are OR-ed in, never lost) and another `wait`s
//! on it: wait blocks until the word is non-zero, then returns it and clears it.
//! This is the seL4-style async primitive — and exactly the shape of an
//! `epoll`/`kqueue` readiness signal: one `wait` can cover many sources, because
//! each source sets its own bit. A process names a notification by a
//! `Notification` capability whose `object` is the notification id; there is no
//! ambient way to reach one.
//!
//! Like endpoints, this first cut uses a busy-yield wait: the waiter polls and
//! yields the CPU until bits appear (the loop lives in the syscall handler so it
//! can drive the scheduler). Real blocking with wait queues comes later.

use spin::Mutex;

/// Number of system notifications. A small fixed table keeps ids simple.
pub const NUM_NOTIFICATIONS: usize = 4;

static NOTIFICATIONS: [Mutex<u64>; NUM_NOTIFICATIONS] =
    [const { Mutex::new(0) }; NUM_NOTIFICATIONS];

/// OR `bits` into notification `id`'s signal word. Returns false if `id` is
/// invalid. Never blocks and never loses bits — multiple signals accumulate
/// until a waiter takes them.
pub fn signal(id: usize, bits: u64) -> bool {
    let Some(n) = NOTIFICATIONS.get(id) else {
        return false;
    };
    *n.lock() |= bits;
    true
}

/// Atomically take notification `id`'s signal word: if non-zero, return it and
/// reset it to zero; otherwise return `None`. Returns `None` for an invalid id.
/// The blocking (busy-yield) loop lives in the syscall handler so the waiter can
/// yield the scheduler between polls.
pub fn poll_take(id: usize) -> Option<u64> {
    let n = NOTIFICATIONS.get(id)?;
    let mut word = n.lock();
    if *word == 0 {
        None
    } else {
        Some(core::mem::take(&mut *word))
    }
}
