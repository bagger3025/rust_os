#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Symmetric CPU-bound throughput benchmark.
///
/// Scenario: N identical CPU-bound processes compete for CPU. There are no
/// I/O, no sleeping, no priority differences — every process does exactly
/// the same work. This is the purest test of scheduling overhead.
///
/// Other schedulers spend cycles on per-dispatch accounting:
///   - CFS/EEVDF: vruntime charging (fetch_add + fetch_update for min_vruntime)
///   - MLFQ: demotion checks, dispatch counter, periodic boost scan
///   - O(1): timeslice charging, epoch swap detection
///   - DRL: neural network forward pass on every scheduling decision
///
/// Output: BENCH:cpusym:total_work=<N>:min_work=<M>
fn main() {
    let num_children: usize = 8;
    let duration: usize = 50; // ticks (~5 seconds)

    let t_start = sys::uptime().unwrap();

    for _ in 0..num_children {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            // Child: CPU-bound busy loop.
            let mut counter: usize = 0;
            loop {
                counter += 1;
                if counter % 10000 == 0 {
                    if sys::uptime().unwrap() - t_start >= duration {
                        break;
                    }
                }
            }
            sys::exit((counter / 1000) as i32);
        }
    }

    // Parent: collect results.
    let mut total: usize = 0;
    let mut min_work: usize = usize::MAX;
    let mut status: i32 = 0;
    for _ in 0..num_children {
        sys::wait(&mut status).unwrap();
        let count = status as usize * 1000;
        total += count;
        if count < min_work {
            min_work = count;
        }
    }

    println!("BENCH:cpusym:total_work={}:min_work={}", total, min_work);
}
