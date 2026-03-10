#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// I/O-bound responsiveness benchmark.
/// Mixes CPU-bound and I/O-bound (sleeping) processes.
/// I/O children accumulate total wakeup delay and exit with it.
/// Parent collects and prints results sequentially.
/// Output: BENCH:iobound:pid=<P>:wakeup_delay=<D>
fn main() {
    let num_cpu: usize = 3;
    let num_io: usize = 3;
    let sleep_ticks: usize = 2;
    let io_iterations: usize = 20;

    let t_start = sys::uptime().unwrap();
    let total_duration: usize = (io_iterations + 2) * sleep_ticks + 10;

    // Fork CPU-bound children
    let mut cpu_pids: [usize; 3] = [0; 3];
    for i in 0..num_cpu {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            loop {
                if sys::uptime().unwrap() - t_start >= total_duration {
                    break;
                }
            }
            sys::exit(0);
        }
        cpu_pids[i] = pid;
    }

    // Fork I/O-bound children — each accumulates total delay
    for _ in 0..num_io {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut total_delay: usize = 0;
            for _ in 0..io_iterations {
                let before = sys::uptime().unwrap();
                sys::sleep(sleep_ticks).unwrap();
                let after = sys::uptime().unwrap();
                let actual = after - before;
                if actual > sleep_ticks {
                    total_delay += actual - sleep_ticks;
                }
            }
            // Exit with total delay (fits in exit code for reasonable values)
            sys::exit(total_delay as i32);
        }
    }

    // Wait for I/O-bound children, print results
    let mut status: i32 = 0;
    for _ in 0..num_io {
        let pid = sys::wait(&mut status).unwrap();
        // Print per-iteration average delay
        let avg_delay = status as usize;
        // Report total delay across all iterations
        println!("BENCH:iobound:pid={}:wakeup_delay={}", pid, avg_delay);
    }

    // Kill CPU-bound workers
    for &cpid in cpu_pids.iter() {
        let _ = sys::kill(cpid);
    }
    for _ in 0..num_cpu {
        let _ = sys::wait(&mut status);
    }
}
