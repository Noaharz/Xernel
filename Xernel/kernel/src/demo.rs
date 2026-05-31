//! Phase-2 demonstration: two kernel threads exchanging messages over an
//! in-kernel channel under the cooperative scheduler. This is milestone 2.0
//! from the design doc, in-kernel form. It will be replaced by real user-space
//! services once capabilities and syscalls land.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::ipc::Channel;
use crate::{println, sched};

const MESSAGES: u64 = 10;
static CHANNEL: Channel = Channel::new();
static RECEIVED: AtomicU64 = AtomicU64::new(0);

extern "C" fn producer() -> ! {
    for i in 1..=MESSAGES {
        println!("[producer t{}] send {i}", sched::current_id());
        CHANNEL.send(i);
        sched::yield_now();
    }
    loop {
        sched::yield_now();
    }
}

extern "C" fn consumer() -> ! {
    let mut sum = 0;
    for _ in 0..MESSAGES {
        let msg = CHANNEL.recv();
        sum += msg;
        RECEIVED.fetch_add(1, Ordering::Relaxed);
        println!("[consumer t{}] recv {msg} (running sum {sum})", sched::current_id());
    }
    println!("[xernel] ipc demo done: {MESSAGES} messages, sum {sum}");

    #[cfg(feature = "boot-test")]
    {
        assert_eq!(sum, MESSAGES * (MESSAGES + 1) / 2, "ipc sum mismatch");
        println!("[xernel] boot-test: ok");
        crate::arch::exit(true);
    }

    #[cfg(not(feature = "boot-test"))]
    loop {
        sched::yield_now();
    }
}

/// Spawn the demo threads and enter the scheduler. Never returns.
pub fn run() -> ! {
    sched::spawn(producer);
    sched::spawn(consumer);
    sched::start();
}
