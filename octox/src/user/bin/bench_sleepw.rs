#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Sleeper wakeup bonus benchmark.
///
/// Scenario: 6 CPU-bound hogs saturate the system while 1 "sleeper"
/// process repeatedly sleeps and measures its wakeup delay. This directly
/// tests whether the scheduler gives priority to recently-woken processes.
///
/// Output: BENCH:sleepw:avg_delay=<N>:p95_delay=<P>:p99_delay=<Q>:max_delay=<M>
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
        let mut delays: [usize; 30] = [0; 30];
        for i in 0..num_iterations {
            let before = sys::uptime().unwrap();
            sys::sleep(sleep_ticks).unwrap();
            let after = sys::uptime().unwrap();
            let actual = after - before;
            if actual > sleep_ticks {
                let delay = actual - sleep_ticks;
                delays[i] = delay;
                total_delay += delay;
                if delay > max_delay {
                    max_delay = delay;
                }
            }
        }
        for i in 1..num_iterations {
            let mut j = i;
            while j > 0 && delays[j - 1] > delays[j] {
                delays.swap(j - 1, j);
                j -= 1;
            }
        }
        let avg = total_delay / num_iterations.max(1);
        let p95 = delays[(num_iterations * 95 / 100).min(num_iterations - 1)];
        let p99 = delays[(num_iterations * 99 / 100).min(num_iterations - 1)];
        // Pack four 7-bit fields while keeping the sign bit clear.
        let code = ((avg.min(0x7F) & 0x7F) << 21)
            | ((p95.min(0x7F) & 0x7F) << 14)
            | ((p99.min(0x7F) & 0x7F) << 7)
            | (max_delay.min(0x7F) & 0x7F);
        sys::exit(code as i32);
    }

    // Wait for sleeper
    let mut status: i32 = 0;
    sys::wait(&mut status).unwrap();
    let code = status as usize;
    let avg_delay = (code >> 21) & 0x7F;
    let p95_delay = (code >> 14) & 0x7F;
    let p99_delay = (code >> 7) & 0x7F;
    let max_delay = code & 0x7F;

    println!(
        "BENCH:sleepw:avg_delay={}:p95_delay={}:p99_delay={}:max_delay={}",
        avg_delay, p95_delay, p99_delay, max_delay
    );

    // Kill workers and wait.
    for &wpid in worker_pids.iter() {
        let _ = sys::kill(wpid);
    }
    for _ in 0..num_workers {
        let _ = sys::wait(&mut status);
    }
}
