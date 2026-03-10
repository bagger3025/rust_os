#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// New process priority benchmark (MLFQ feedback test).
///
/// Scenario: 6 long-running CPU-bound workers have been running for 20+
/// ticks (enough to be demoted in MLFQ). Then the parent rapidly forks
/// short-lived measurement children and records how quickly they get
/// their first time slice.
///
/// Output: BENCH:newproc:avg_latency=<N>:max_latency=<M>
fn main() {
    let num_workers: usize = 6;
    let warmup_ticks: usize = 20;
    let num_measurements: usize = 10;

    let t_start = sys::uptime().unwrap();

    // Fork background workers.
    let mut worker_pids: [usize; 6] = [0; 6];
    for i in 0..num_workers {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            // CPU-bound: busy loop until killed.
            loop {
                if sys::uptime().unwrap() - t_start >= 200 {
                    break;
                }
            }
            sys::exit(0);
        }
        worker_pids[i] = pid;
    }

    // Wait for workers to get demoted.
    while sys::uptime().unwrap() - t_start < warmup_ticks {
        // spin — parent yields via uptime syscall
    }

    // Measure first-slice latency for new processes.
    let mut total_latency: usize = 0;
    let mut max_latency: usize = 0;
    let mut status: i32 = 0;

    for _ in 0..num_measurements {
        let t0 = sys::uptime().unwrap();
        let pid = sys::fork().unwrap();
        if pid == 0 {
            // Child: immediately report and exit.
            let t1 = sys::uptime().unwrap();
            sys::exit((t1 - t0) as i32);
        }
        sys::wait(&mut status).unwrap();
        let latency = status as usize;
        total_latency += latency;
        if latency > max_latency {
            max_latency = latency;
        }
    }

    let avg_latency = total_latency / num_measurements.max(1);
    println!(
        "BENCH:newproc:avg_latency={}:max_latency={}",
        avg_latency, max_latency
    );

    // Kill workers.
    for &wpid in worker_pids.iter() {
        let _ = sys::kill(wpid);
    }
    for _ in 0..num_workers {
        let _ = sys::wait(&mut status);
    }
}
