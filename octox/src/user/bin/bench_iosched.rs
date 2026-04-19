#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Heavy I/O scheduling benchmark.
///
/// 40 CPU workers create 10x oversubscription on 4 cores, while
/// 8 I/O workers repeatedly sleep(1) and measure wakeup overshoot.
///
/// Unlike the existing iobound benchmark (3 CPU workers = idle cores
/// available), this scenario guarantees all cores are always busy.
/// When an I/O worker wakes from sleep, it MUST preempt a CPU worker
/// to run. The scheduler's wakeup/preemption mechanism is the sole
/// path to low latency.
///
/// Output: BENCH:iosched:pid=<P>:wakeup_delay=<D>:avg_delay=<A>:
/// p95_delay=<P95>:max_delay=<M>
fn main() {
    let num_cpu: usize = 40;
    let num_io: usize = 8;
    let io_iterations: usize = 40;
    let sleep_ticks: usize = 1;

    let t_start = sys::uptime().unwrap();
    let total_duration: usize = (io_iterations + 5) * (sleep_ticks + 1) + 20;

    // Fork CPU-bound workers.
    let mut cpu_pids: [usize; 40] = [0; 40];
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

    // Fork I/O workers.
    for _ in 0..num_io {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut total_delay: usize = 0;
            let mut max_delay: usize = 0;
            let mut delays: [usize; 40] = [0; 40];
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

    // Collect I/O worker results.
    let mut status: i32 = 0;
    for _ in 0..num_io {
        let pid = sys::wait(&mut status).unwrap();
        let code = status as usize;
        let total_delay = (code >> 21) & 0x7F;
        let avg_delay = (code >> 14) & 0x7F;
        let p95_delay = (code >> 7) & 0x7F;
        let max_delay = code & 0x7F;
        println!(
            "BENCH:iosched:pid={}:wakeup_delay={}:avg_delay={}:p95_delay={}:max_delay={}",
            pid, total_delay, avg_delay, p95_delay, max_delay
        );
    }

    // Kill CPU workers.
    for &cpid in cpu_pids.iter() {
        let _ = sys::kill(cpid);
    }
    for _ in 0..num_cpu {
        let _ = sys::wait(&mut status);
    }
}
