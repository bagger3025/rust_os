#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// I/O-bound responsiveness benchmark.
/// Mixes CPU-bound and I/O-bound (sleeping) processes.
/// I/O children accumulate total wakeup delay and exit with it.
/// Parent collects and prints results sequentially.
/// Output: BENCH:iobound:pid=<P>:wakeup_delay=<D>:avg_delay=<A>:
/// p95_delay=<P95>:max_delay=<M>
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
            let mut max_delay: usize = 0;
            let mut delays: [usize; 20] = [0; 20];
            for i in 0..io_iterations {
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
            for i in 1..io_iterations {
                let mut j = i;
                while j > 0 && delays[j - 1] > delays[j] {
                    delays.swap(j - 1, j);
                    j -= 1;
                }
            }
            let avg = total_delay / io_iterations.max(1);
            let p95 = delays[(io_iterations * 95 / 100).min(io_iterations - 1)];
            let code = ((total_delay.min(0x7F) & 0x7F) << 21)
                | ((avg.min(0x7F) & 0x7F) << 14)
                | ((p95.min(0x7F) & 0x7F) << 7)
                | (max_delay.min(0x7F) & 0x7F);
            sys::exit(code as i32);
        }
    }

    // Wait for I/O-bound children, print results
    let mut status: i32 = 0;
    for _ in 0..num_io {
        let pid = sys::wait(&mut status).unwrap();
        let code = status as usize;
        let total_delay = (code >> 21) & 0x7F;
        let avg_delay = (code >> 14) & 0x7F;
        let p95_delay = (code >> 7) & 0x7F;
        let max_delay = code & 0x7F;
        println!(
            "BENCH:iobound:pid={}:wakeup_delay={}:avg_delay={}:p95_delay={}:max_delay={}",
            pid, total_delay, avg_delay, p95_delay, max_delay
        );
    }

    // Kill CPU-bound workers
    for &cpid in cpu_pids.iter() {
        let _ = sys::kill(cpid);
    }
    for _ in 0..num_cpu {
        let _ = sys::wait(&mut status);
    }
}
