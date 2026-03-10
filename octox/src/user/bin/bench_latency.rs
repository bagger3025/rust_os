#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// First-slice latency + sleep correctness benchmark.
///
/// Part 1: Idle latency — fork a child, measure ticks until it runs.
/// Part 2: Loaded latency — same but with background CPU workers.
/// Part 3: Sleep correctness — verify sleep(5) takes ~5 ticks.
fn main() {
    // Part 1: idle latency
    for _ in 0..10 {
        let t0 = sys::uptime().unwrap();
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let t1 = sys::uptime().unwrap();
            println!("BENCH:latency:delay={}", t1 - t0);
            sys::exit(0);
        }
        let mut status: i32 = 0;
        sys::wait(&mut status).unwrap();
    }

    // Part 2: loaded latency (4 background CPU workers)
    let mut worker_pids: [usize; 4] = [0; 4];
    for i in 0..4 {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            // CPU-bound worker: loop forever
            loop {
                // tight loop — will be killed by parent
                core::hint::spin_loop();
            }
        }
        worker_pids[i] = pid;
    }

    for _ in 0..10 {
        let t0 = sys::uptime().unwrap();
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let t1 = sys::uptime().unwrap();
            println!("BENCH:latency_loaded:delay={}", t1 - t0);
            sys::exit(0);
        }
        let mut status: i32 = 0;
        sys::wait(&mut status).unwrap();
    }

    // kill background workers
    for &wpid in worker_pids.iter() {
        let _ = sys::kill(wpid);
    }
    let mut status: i32 = 0;
    for _ in 0..4 {
        let _ = sys::wait(&mut status);
    }

    // Part 3: sleep correctness
    let t0 = sys::uptime().unwrap();
    sys::sleep(5).unwrap();
    let t1 = sys::uptime().unwrap();
    println!("BENCH:sleep:elapsed={}", t1 - t0);
}
