#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Sleeper wakeup bonus benchmark.
///
/// Scenario: 6 CPU-bound hogs saturate the system while 1 "sleeper"
/// process repeatedly sleeps and measures its wakeup delay. This directly
/// tests whether the scheduler gives priority to recently-woken processes.
///
/// Output: BENCH:sleepw:avg_delay=<N>:max_delay=<M>
fn main() {
    let num_workers: usize = 6;
    let num_iterations: usize = 30;
    let sleep_ticks: usize = 1;
    let total_duration: usize = num_iterations * (sleep_ticks + 2) + 20;

    let t_start = sys::uptime().unwrap();

    // Fork CPU-bound background workers
    let mut worker_pids: [usize; 6] = [0; 6];
    for i in 0..num_workers {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            loop {
                if sys::uptime().unwrap() - t_start >= total_duration {
                    break;
                }
            }
            sys::exit(0);
        }
        worker_pids[i] = pid;
    }

    // Fork the sleeper child
    let pid = sys::fork().unwrap();
    if pid == 0 {
        let mut total_delay: usize = 0;
        let mut max_delay: usize = 0;
        for _ in 0..num_iterations {
            let before = sys::uptime().unwrap();
            sys::sleep(sleep_ticks).unwrap();
            let after = sys::uptime().unwrap();
            let actual = after - before;
            if actual > sleep_ticks {
                let delay = actual - sleep_ticks;
                total_delay += delay;
                if delay > max_delay {
                    max_delay = delay;
                }
            }
        }
        // Pack avg_delay and max_delay into exit status
        // avg in upper bits, max in lower 8 bits
        let avg = total_delay / num_iterations.max(1);
        let code = ((avg & 0xFF) << 8) | (max_delay & 0xFF);
        sys::exit(code as i32);
    }

    // Wait for sleeper
    let mut status: i32 = 0;
    sys::wait(&mut status).unwrap();
    let avg_delay = (status as usize >> 8) & 0xFF;
    let max_delay = status as usize & 0xFF;

    println!(
        "BENCH:sleepw:avg_delay={}:max_delay={}",
        avg_delay, max_delay
    );

    // Kill workers and wait.
    for &wpid in worker_pids.iter() {
        let _ = sys::kill(wpid);
    }
    for _ in 0..num_workers {
        let _ = sys::wait(&mut status);
    }
}
