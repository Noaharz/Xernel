//! In-kernel message passing.
//!
//! A minimal synchronous-ish channel used to validate that two kernel threads
//! can communicate while the cooperative scheduler round-robins between them.
//! This is a stepping stone toward the real capability-based IPC (Endpoint /
//! Notification objects); the *shape* — send, blocking receive — is the same,
//! but here there are no capabilities and no user/kernel boundary yet.

use alloc::collections::VecDeque;

use spin::Mutex;

use crate::sched;

/// A FIFO byte/word channel between kernel threads.
pub struct Channel {
    queue: Mutex<VecDeque<u64>>,
}

impl Channel {
    pub const fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
        }
    }

    /// Enqueue a message. Never blocks.
    pub fn send(&self, msg: u64) {
        self.queue.lock().push_back(msg);
    }

    /// Dequeue a message, yielding the CPU until one is available.
    pub fn recv(&self) -> u64 {
        loop {
            if let Some(msg) = self.queue.lock().pop_front() {
                return msg;
            }
            sched::yield_now();
        }
    }

    /// Non-blocking dequeue.
    pub fn try_recv(&self) -> Option<u64> {
        self.queue.lock().pop_front()
    }
}

impl Default for Channel {
    fn default() -> Self {
        Self::new()
    }
}
