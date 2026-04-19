#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Reverse adaptive phase-change benchmark.
///
/// Scenario: 32 processes run in two phases:
///   Phase 1 (0-30 ticks): 16 CPU-bound workers and 16 I/O-like sleepers.
///   Phase 2 (30-70 ticks): all children become CPU-bound.
///
/// This tests whether a scheduler can recover throughput after a latency-
/// oriented phase ends.
///
/// Output:
///   BENCH:adaptrev:phase1_delay=<N> — total sleeper overshoot in phase 1
///   BENCH:adaptrev:phase2_work=<N>  — total CPU work in phase 2
///   BENCH:adaptrev:min_work=<N>     — minimum per-child phase-2 work
///   BENCH:adaptrev:max_work=<N>     — maximum per-child phase-2 work
fn main() {
    let num_cpu: usize = 16;
    let num_switch: usize = 16;
    let phase1_ticks: usize = 30;
    let phase2_ticks: usize = 40;
    let sleep_ticks: usize = 2;

    let t_start = sys::uptime().unwrap();
    let phase2_start = t_start + phase1_ticks;
    let total_end = phase2_start + phase2_ticks;

    // CPU-like children: run CPU-bound in both phases.
    for _ in 0..num_cpu {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            while sys::uptime().unwrap() < phase2_start {
                core::hint::spin_loop();
            }

            let mut phase2_count: usize = 0;
            loop {
                phase2_count += 1;
                if phase2_count % 10000 == 0 && sys::uptime().unwrap() >= total_end {
                    break;
                }
            }
            sys::exit((phase2_count / 1000) as i32);
        }
    }

    // Switching children: I/O-like in phase 1, CPU-bound in phase 2.
    for _ in 0..num_switch {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut phase1_delay: usize = 0;
            loop {
                let before = sys::uptime().unwrap();
                if before >= phase2_start {
                    break;
                }
                sys::sleep(sleep_ticks).unwrap();
                let after = sys::uptime().unwrap();
                let actual = after - before;
                if actual > sleep_ticks {
                    phase1_delay += actual - sleep_ticks;
                }
            }

            let mut phase2_count: usize = 0;
            loop {
                phase2_count += 1;
                if phase2_count % 10000 == 0 && sys::uptime().unwrap() >= total_end {
                    break;
                }
            }

            // High 7 bits carry bounded phase-1 delay; low 24 bits carry work.
            // Keep bit 31 clear so the exit status remains positive.
            let code =
                ((phase1_delay.min(0x7F) & 0x7F) << 24) | ((phase2_count / 1000) & 0x00FF_FFFF);
            sys::exit(code as i32);
        }
    }

    let mut status: i32 = 0;
    let mut total_delay: usize = 0;
    let mut total_work: usize = 0;
    let mut min_work: usize = usize::MAX;
    let mut max_work: usize = 0;

    for _ in 0..(num_cpu + num_switch) {
        sys::wait(&mut status).unwrap();
        let code = status as usize;
        let delay = (code >> 24) & 0xFF;
        let work = (code & 0x00FF_FFFF) * 1000;
        total_delay += delay;
        total_work += work;
        if work < min_work {
            min_work = work;
        }
        if work > max_work {
            max_work = work;
        }
    }

    println!(
        "BENCH:adaptrev:phase1_delay={}:phase2_work={}:min_work={}:max_work={}",
        total_delay, total_work, min_work, max_work
    );
}
