#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Bursty adversarial interactive workload.
///
/// 24 batch workers run CPU-bound while 16 bursty workers repeatedly sleep
/// and then perform variable CPU bursts. Some bursts are intentionally long,
/// testing whether interactive treatment can be abused.
///
/// Output: BENCH:burst:delay_p95=<N>:delay_max=<N>:batch_work=<N>:
/// bursty_work=<N>
fn main() {
    let num_batch: usize = 24;
    let num_bursty: usize = 16;
    let duration: usize = 70;
    let cycles: usize = 24;

    let t_start = sys::uptime().unwrap();
    let end = t_start + duration;

    let mut batch_pids: [usize; 24] = [0; 24];
    for i in 0..num_batch {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut counter: usize = 0;
            loop {
                counter += 1;
                if counter % 10000 == 0 && sys::uptime().unwrap() >= end {
                    break;
                }
            }
            sys::exit((counter / 1000) as i32);
        }
        batch_pids[i] = pid;
    }

    for child_idx in 0..num_bursty {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut total_delay: usize = 0;
            let mut max_delay: usize = 0;
            let mut work_total: usize = 0;
            let mut delays: [usize; 24] = [0; 24];

            for cycle in 0..cycles {
                if sys::uptime().unwrap() >= end {
                    break;
                }
                let sleep_ticks = if (cycle + child_idx) % 4 == 0 { 3 } else { 1 };
                let before = sys::uptime().unwrap();
                sys::sleep(sleep_ticks).unwrap();
                let after = sys::uptime().unwrap();
                let actual = after - before;
                if actual > sleep_ticks {
                    let delay = actual - sleep_ticks;
                    delays[cycle] = delay;
                    total_delay += delay;
                    if delay > max_delay {
                        max_delay = delay;
                    }
                }

                let burst = match (cycle + child_idx) % 5 {
                    0 => 250_000,
                    1 | 2 => 30_000,
                    _ => 80_000,
                };
                for _ in 0..burst {
                    work_total += 1;
                }
            }

            for i in 1..cycles {
                let mut j = i;
                while j > 0 && delays[j - 1] > delays[j] {
                    delays.swap(j - 1, j);
                    j -= 1;
                }
            }
            let p95 = delays[(cycles * 95 / 100).min(cycles - 1)];
            let code = ((p95.min(0x7F) & 0x7F) << 21)
                | ((max_delay.min(0x7F) & 0x7F) << 14)
                | ((work_total / 1000) & 0x3FFF);
            let _ = total_delay;
            sys::exit(code as i32);
        }
    }

    let mut status: i32 = 0;
    let mut batch_work: usize = 0;
    let mut bursty_work: usize = 0;
    let mut delay_p95: usize = 0;
    let mut delay_max: usize = 0;

    for _ in 0..(num_batch + num_bursty) {
        let pid = sys::wait(&mut status).unwrap();
        let is_batch = batch_pids.iter().any(|&bp| bp == pid);
        if is_batch {
            batch_work += status as usize * 1000;
        } else {
            let code = status as usize;
            let p95 = (code >> 21) & 0x7F;
            let mx = (code >> 14) & 0x7F;
            let work = (code & 0x3FFF) * 1000;
            if p95 > delay_p95 {
                delay_p95 = p95;
            }
            if mx > delay_max {
                delay_max = mx;
            }
            bursty_work += work;
        }
    }

    println!(
        "BENCH:burst:delay_p95={}:delay_max={}:batch_work={}:bursty_work={}",
        delay_p95, delay_max, batch_work, bursty_work
    );
}
