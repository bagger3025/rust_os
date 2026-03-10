#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Convoy effect benchmark.
///
/// 6 CPU hogs saturate all 4 cores. 6 "burst" workers repeatedly
/// sleep(1), wake, do a short CPU burst, and sleep again. This tests
/// whether the scheduler can quickly preempt hogs in favor of freshly-
/// woken burst workers.
///
/// Output: BENCH:convoy:hog_work=<N>:burst_delay=<D>
fn main() {
    let num_hogs: usize = 6;
    let num_burst: usize = 6;
    let duration: usize = 60;
    let burst_cycles: usize = 25;
    let sleep_per_burst: usize = 1;

    let t_start = sys::uptime().unwrap();

    // Fork CPU hogs.
    let mut hog_pids: [usize; 6] = [0; 6];
    for i in 0..num_hogs {
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
        hog_pids[i] = pid;
    }

    // Fork burst workers.
    for _ in 0..num_burst {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut total_delay: usize = 0;
            for _ in 0..burst_cycles {
                let before = sys::uptime().unwrap();
                if before - t_start >= duration {
                    break;
                }
                sys::sleep(sleep_per_burst).unwrap();
                let after = sys::uptime().unwrap();
                let actual = after - before;
                if actual > sleep_per_burst {
                    total_delay += actual - sleep_per_burst;
                }
                // Short CPU burst after waking (prevents immediate re-sleep).
                let mut work: usize = 0;
                for _ in 0..5000 {
                    work += 1;
                }
                let _ = work;
            }
            sys::exit(total_delay as i32);
        }
    }

    // Collect results.
    let mut total_hog_work: usize = 0;
    let mut total_burst_delay: usize = 0;
    let mut status: i32 = 0;

    for _ in 0..(num_hogs + num_burst) {
        let pid = sys::wait(&mut status).unwrap();
        let is_hog = hog_pids.iter().any(|&hp| hp == pid);
        if is_hog {
            total_hog_work += status as usize * 1000;
        } else {
            total_burst_delay += status as usize;
        }
    }

    println!(
        "BENCH:convoy:hog_work={}:burst_delay={}",
        total_hog_work, total_burst_delay
    );
}
