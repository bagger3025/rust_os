#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Active/expired batch separation benchmark (O(1) interactivity test).
///
/// Scenario: 4 CPU-bound "batch" workers coexist with 3 "interactive"
/// processes that repeatedly sleep(1) and measure wakeup overshoot.
/// The wakeup overshoot (actual_sleep - requested_sleep) measures how
/// quickly the scheduler dispatches an interactive process after it wakes.
///
/// Output:
///   BENCH:epoch:io_resp=<N>    — total wakeup overshoot across interactive tasks
///   BENCH:epoch:io_p95=<N>     — worst per-child p95 wakeup overshoot
///   BENCH:epoch:io_max=<N>     — worst per-child max wakeup overshoot
///   BENCH:epoch:batch_work=<N> — total work completed by batch tasks
fn main() {
    let num_batch: usize = 4;
    let num_interactive: usize = 3;
    let duration: usize = 60; // ticks
    let io_cycles: usize = 25;
    let sleep_per_cycle: usize = 1;

    let t_start = sys::uptime().unwrap();

    // Fork batch workers.
    let mut batch_pids: [usize; 4] = [0; 4];
    for i in 0..num_batch {
        let pid = sys::fork().unwrap();
        if pid == 0 {
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
        batch_pids[i] = pid;
    }

    // Fork interactive workers.
    for _ in 0..num_interactive {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut total_overshoot: usize = 0;
            let mut max_overshoot: usize = 0;
            let mut delays: [usize; 25] = [0; 25];
            for i in 0..io_cycles {
                let before = sys::uptime().unwrap();
                sys::sleep(sleep_per_cycle).unwrap();
                let after = sys::uptime().unwrap();
                let actual = after - before;
                if actual > sleep_per_cycle {
                    let delay = actual - sleep_per_cycle;
                    delays[i] = delay;
                    total_overshoot += delay;
                    if delay > max_overshoot {
                        max_overshoot = delay;
                    }
                }

                if sys::uptime().unwrap() - t_start >= duration {
                    break;
                }
            }
            for i in 1..io_cycles {
                let mut j = i;
                while j > 0 && delays[j - 1] > delays[j] {
                    delays.swap(j - 1, j);
                    j -= 1;
                }
            }
            let p95 = delays[(io_cycles * 95 / 100).min(io_cycles - 1)];
            let code = ((total_overshoot.min(0x3FF) & 0x3FF) << 14)
                | ((p95.min(0x7F) & 0x7F) << 7)
                | (max_overshoot.min(0x7F) & 0x7F);
            sys::exit(code as i32);
        }
    }

    // Collect results — use PIDs to categorize.
    let mut total_io_overshoot: usize = 0;
    let mut worst_p95: usize = 0;
    let mut worst_max: usize = 0;
    let mut io_collected: usize = 0;
    let mut total_batch_work: usize = 0;
    let mut status: i32 = 0;

    for _ in 0..(num_interactive + num_batch) {
        let pid = sys::wait(&mut status).unwrap();
        let is_batch = batch_pids.iter().any(|&bp| bp == pid);
        if is_batch {
            total_batch_work += status as usize * 1000;
        } else {
            let code = status as usize;
            let total = (code >> 14) & 0x3FF;
            let p95 = (code >> 7) & 0x7F;
            let mx = code & 0x7F;
            total_io_overshoot += total;
            if p95 > worst_p95 {
                worst_p95 = p95;
            }
            if mx > worst_max {
                worst_max = mx;
            }
            io_collected += 1;
        }
    }

    println!(
        "BENCH:epoch:io_resp={}:io_p95={}:io_max={}:batch_work={}",
        total_io_overshoot, worst_p95, worst_max, total_batch_work
    );
}
