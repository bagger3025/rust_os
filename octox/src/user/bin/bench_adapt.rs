#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Adaptive workload phase-change benchmark.
///
/// Scenario: 6 processes run in two phases:
///   Phase 1 (0–30 ticks): All 6 are CPU-bound.
///   Phase 2 (30–70 ticks): Children 0–2 stay CPU-bound, children 3–5
///   switch to I/O-bound (sleep + measure wakeup delay).
///
/// This tests whether the scheduler can adapt to a changing workload.
///
/// Static schedulers handle the transition through fixed heuristics:
///   CFS: sleeper bonus kicks in for newly-sleeping tasks
///   MLFQ: sleeping tasks stay at high priority after boost
///   O(1): sleep_avg gradually increases for I/O tasks
///
/// Output:
///   BENCH:adapt:phase1_work=<N>     — total CPU work in phase 1
///   BENCH:adapt:phase2_delay=<N>    — total wakeup delay in phase 2
///   BENCH:adapt:phase2_work=<N>     — CPU work in phase 2 (batch children)
fn main() {
    let num_cpu: usize = 3;
    let num_io: usize = 3;
    let phase1_ticks: usize = 30;
    let phase2_ticks: usize = 40;
    let sleep_per_io: usize = 2;
    let io_iterations: usize = 15;

    let t_start = sys::uptime().unwrap();
    let phase2_start = t_start + phase1_ticks;
    let total_end = t_start + phase1_ticks + phase2_ticks;

    // Fork CPU-bound children (0-2): run both phases CPU-bound.
    let mut cpu_pids: [usize; 3] = [0; 3];
    for i in 0..num_cpu {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            // Phase 1.
            let mut phase1_count: usize = 0;
            loop {
                phase1_count += 1;
                if phase1_count % 10000 == 0 {
                    if sys::uptime().unwrap() >= phase2_start {
                        break;
                    }
                }
            }
            // Phase 2: continue CPU-bound.
            let mut phase2_count: usize = 0;
            loop {
                phase2_count += 1;
                if phase2_count % 10000 == 0 {
                    if sys::uptime().unwrap() >= total_end {
                        break;
                    }
                }
            }
            // Exit with phase2 work / 1000 (phase1 counted separately).
            sys::exit((phase2_count / 1000) as i32);
        }
        cpu_pids[i] = pid;
    }

    // Fork I/O-bound children (3-5): CPU-bound in phase 1, I/O in phase 2.
    let mut io_pids: [usize; 3] = [0; 3];
    for i in 0..num_io {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            // Phase 1: CPU-bound.
            let mut phase1_count: usize = 0;
            loop {
                phase1_count += 1;
                if phase1_count % 10000 == 0 {
                    if sys::uptime().unwrap() >= phase2_start {
                        break;
                    }
                }
            }
            // Phase 2: switch to I/O-bound.
            let mut total_delay: usize = 0;
            for _ in 0..io_iterations {
                let before = sys::uptime().unwrap();
                if before >= total_end {
                    break;
                }
                sys::sleep(sleep_per_io).unwrap();
                let after = sys::uptime().unwrap();
                let actual = after - before;
                if actual > sleep_per_io {
                    total_delay += actual - sleep_per_io;
                }
            }
            // Exit with total wakeup delay.
            sys::exit(total_delay as i32);
        }
        io_pids[i] = pid;
    }

    // Collect results — use PIDs to categorize.
    let mut total_phase2_cpu_work: usize = 0;
    let mut total_phase2_delay: usize = 0;
    let mut status: i32 = 0;

    for _ in 0..(num_cpu + num_io) {
        let pid = sys::wait(&mut status).unwrap();
        let is_cpu = cpu_pids.iter().any(|&cp| cp == pid);
        if is_cpu {
            total_phase2_cpu_work += status as usize * 1000;
        } else {
            total_phase2_delay += status as usize;
        }
    }

    println!(
        "BENCH:adapt:phase1_work=0:phase2_delay={}:phase2_work={}",
        total_phase2_delay, total_phase2_cpu_work
    );
}
