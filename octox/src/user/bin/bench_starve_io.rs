#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// I/O-starvation adversarial workload.
///
/// 40 frequent sleepers compete with 8 CPU-bound batch workers. This catches
/// schedulers that reduce sleeper latency by starving batch work.
///
/// Output: BENCH:starveio:sleeper_p95=<N>:sleeper_max=<N>:batch_min=<N>:batch_max=<N>
fn main() {
    let num_sleepers: usize = 40;
    let num_batch: usize = 8;
    let duration: usize = 70;
    let cycles: usize = 24;

    let start = sys::uptime().unwrap();
    let end = start + duration;

    let mut batch_pids: [usize; 8] = [0; 8];
    for i in 0..num_batch {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut work: usize = 0;
            loop {
                work += 1;
                if work % 10000 == 0 && sys::uptime().unwrap() >= end {
                    break;
                }
            }
            sys::exit((work / 1000) as i32);
        }
        batch_pids[i] = pid;
    }

    for _ in 0..num_sleepers {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut delays: [usize; 24] = [0; 24];
            let mut max_delay: usize = 0;
            for i in 0..cycles {
                if sys::uptime().unwrap() >= end {
                    break;
                }
                let before = sys::uptime().unwrap();
                sys::sleep(1).unwrap();
                let after = sys::uptime().unwrap();
                if after > before + 1 {
                    let delay = after - before - 1;
                    delays[i] = delay;
                    if delay > max_delay {
                        max_delay = delay;
                    }
                }
                let mut small_work: usize = 0;
                for _ in 0..20_000 {
                    small_work += 1;
                }
                let _ = small_work;
            }
            for i in 1..cycles {
                let mut j = i;
                while j > 0 && delays[j - 1] > delays[j] {
                    delays.swap(j - 1, j);
                    j -= 1;
                }
            }
            let p95 = delays[(cycles * 95 / 100).min(cycles - 1)];
            sys::exit((((p95.min(0x7F) & 0x7F) << 7) | (max_delay.min(0x7F) & 0x7F)) as i32);
        }
    }

    let mut status: i32 = 0;
    let mut sleeper_p95: usize = 0;
    let mut sleeper_max: usize = 0;
    let mut batch_min: usize = usize::MAX;
    let mut batch_max: usize = 0;

    for _ in 0..(num_sleepers + num_batch) {
        let pid = sys::wait(&mut status).unwrap();
        let is_batch = batch_pids.iter().any(|&bp| bp == pid);
        if is_batch {
            let work = status as usize * 1000;
            if work < batch_min {
                batch_min = work;
            }
            if work > batch_max {
                batch_max = work;
            }
        } else {
            let code = status as usize;
            let p95 = (code >> 7) & 0x7F;
            let mx = code & 0x7F;
            if p95 > sleeper_p95 {
                sleeper_p95 = p95;
            }
            if mx > sleeper_max {
                sleeper_max = mx;
            }
        }
    }

    println!(
        "BENCH:starveio:sleeper_p95={}:sleeper_max={}:batch_min={}:batch_max={}",
        sleeper_p95, sleeper_max, batch_min, batch_max
    );
}
