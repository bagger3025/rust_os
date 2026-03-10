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
            for _ in 0..io_cycles {
                let before = sys::uptime().unwrap();
                sys::sleep(sleep_per_cycle).unwrap();
                let after = sys::uptime().unwrap();
                let actual = after - before;
                if actual > sleep_per_cycle {
                    total_overshoot += actual - sleep_per_cycle;
                }

                if sys::uptime().unwrap() - t_start >= duration {
                    break;
                }
            }
            // Exit with total wakeup overshoot.
            sys::exit(total_overshoot as i32);
        }
    }

    // Collect results — use PIDs to categorize.
    let mut total_io_overshoot: usize = 0;
    let mut io_collected: usize = 0;
    let mut total_batch_work: usize = 0;
    let mut status: i32 = 0;

    for _ in 0..(num_interactive + num_batch) {
        let pid = sys::wait(&mut status).unwrap();
        let is_batch = batch_pids.iter().any(|&bp| bp == pid);
        if is_batch {
            total_batch_work += status as usize * 1000;
        } else {
            total_io_overshoot += status as usize;
            io_collected += 1;
        }
    }

    println!(
        "BENCH:epoch:io_resp={}:batch_work={}",
        total_io_overshoot, total_batch_work
    );
}
