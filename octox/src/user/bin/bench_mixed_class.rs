#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Mixed-class workload for Hybrid D-Scheduler.
///
/// Command name is `benchmixclass` to fit the 14-byte directory entry limit.
/// It creates interactive, periodic, batch, and ambiguous bursty tasks at once.
///
/// Output: BENCH:mixedclass:interactive_p95=<N>:interactive_max=<N>:
/// periodic_miss=<N>:periodic_max=<N>:batch_work=<N>:batch_min=<N>:
/// batch_max=<N>:bursty_p95=<N>:bursty_max=<N>:bursty_work=<N>
fn main() {
    let num_interactive: usize = 8;
    let num_periodic: usize = 8;
    let num_batch: usize = 24;
    let num_bursty: usize = 8;
    let duration: usize = 70;
    let cycles: usize = 24;

    let start = sys::uptime().unwrap();
    let end = start + duration;

    let mut interactive_pids: [usize; 8] = [0; 8];
    for i in 0..num_interactive {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut delays: [usize; 24] = [0; 24];
            let mut max_delay: usize = 0;
            let mut work: usize = 0;
            for cycle in 0..cycles {
                if sys::uptime().unwrap() >= end {
                    break;
                }
                let before = sys::uptime().unwrap();
                sys::sleep(1).unwrap();
                let after = sys::uptime().unwrap();
                if after > before + 1 {
                    let delay = after - before - 1;
                    delays[cycle] = delay;
                    if delay > max_delay {
                        max_delay = delay;
                    }
                }
                for _ in 0..25_000 {
                    work += 1;
                }
            }
            sort(&mut delays);
            let p95 = delays[(cycles * 95 / 100).min(cycles - 1)];
            let code = ((p95.min(0x7F) & 0x7F) << 21)
                | ((max_delay.min(0x7F) & 0x7F) << 14)
                | ((work / 1000) & 0x3FFF);
            sys::exit(code as i32);
        }
        interactive_pids[i] = pid;
    }

    let mut periodic_pids: [usize; 8] = [0; 8];
    for i in 0..num_periodic {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut miss_count: usize = 0;
            let mut max_late: usize = 0;
            let mut work: usize = 0;
            for _ in 0..cycles {
                if sys::uptime().unwrap() >= end {
                    break;
                }
                let before = sys::uptime().unwrap();
                sys::sleep(5).unwrap();
                let after = sys::uptime().unwrap();
                if after > before + 5 {
                    let late = after - before - 5;
                    if late > 1 {
                        miss_count += 1;
                    }
                    if late > max_late {
                        max_late = late;
                    }
                }
                for _ in 0..90_000 {
                    work += 1;
                }
            }
            let code = ((miss_count.min(0x7F) & 0x7F) << 21)
                | ((max_late.min(0x7F) & 0x7F) << 14)
                | ((work / 1000) & 0x3FFF);
            sys::exit(code as i32);
        }
        periodic_pids[i] = pid;
    }

    let mut batch_pids: [usize; 24] = [0; 24];
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

    let mut bursty_pids: [usize; 8] = [0; 8];
    for child_idx in 0..num_bursty {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut delays: [usize; 24] = [0; 24];
            let mut max_delay: usize = 0;
            let mut work: usize = 0;
            for cycle in 0..cycles {
                if sys::uptime().unwrap() >= end {
                    break;
                }
                let sleep_ticks = if (cycle + child_idx) % 3 == 0 { 1 } else { 2 };
                let before = sys::uptime().unwrap();
                sys::sleep(sleep_ticks).unwrap();
                let after = sys::uptime().unwrap();
                if after > before + sleep_ticks {
                    let delay = after - before - sleep_ticks;
                    delays[cycle] = delay;
                    if delay > max_delay {
                        max_delay = delay;
                    }
                }
                let burst = if (cycle + child_idx) % 5 == 0 {
                    300_000
                } else {
                    45_000
                };
                for _ in 0..burst {
                    work += 1;
                }
            }
            sort(&mut delays);
            let p95 = delays[(cycles * 95 / 100).min(cycles - 1)];
            let code = ((p95.min(0x7F) & 0x7F) << 21)
                | ((max_delay.min(0x7F) & 0x7F) << 14)
                | ((work / 1000) & 0x3FFF);
            sys::exit(code as i32);
        }
        bursty_pids[child_idx] = pid;
    }

    let mut status: i32 = 0;
    let mut interactive_p95: usize = 0;
    let mut interactive_max: usize = 0;
    let mut periodic_miss: usize = 0;
    let mut periodic_max: usize = 0;
    let mut batch_work: usize = 0;
    let mut batch_min: usize = usize::MAX;
    let mut batch_max: usize = 0;
    let mut bursty_p95: usize = 0;
    let mut bursty_max: usize = 0;
    let mut bursty_work: usize = 0;

    for _ in 0..(num_interactive + num_periodic + num_batch + num_bursty) {
        let pid = sys::wait(&mut status).unwrap();
        let code = status as usize;
        if contains(&interactive_pids, pid) {
            interactive_p95 = interactive_p95.max((code >> 21) & 0x7F);
            interactive_max = interactive_max.max((code >> 14) & 0x7F);
        } else if contains(&periodic_pids, pid) {
            periodic_miss += (code >> 21) & 0x7F;
            periodic_max = periodic_max.max((code >> 14) & 0x7F);
        } else if contains(&batch_pids, pid) {
            let work = code * 1000;
            batch_work += work;
            batch_min = batch_min.min(work);
            batch_max = batch_max.max(work);
        } else if contains(&bursty_pids, pid) {
            bursty_p95 = bursty_p95.max((code >> 21) & 0x7F);
            bursty_max = bursty_max.max((code >> 14) & 0x7F);
            bursty_work += (code & 0x3FFF) * 1000;
        }
    }

    println!(
        "BENCH:mixedclass:interactive_p95={}:interactive_max={}:periodic_miss={}:periodic_max={}:batch_work={}:batch_min={}:batch_max={}:bursty_p95={}:bursty_max={}:bursty_work={}",
        interactive_p95,
        interactive_max,
        periodic_miss,
        periodic_max,
        batch_work,
        batch_min,
        batch_max,
        bursty_p95,
        bursty_max,
        bursty_work
    );
}

fn contains<const N: usize>(items: &[usize; N], value: usize) -> bool {
    items.iter().any(|&item| item == value)
}

fn sort<const N: usize>(items: &mut [usize; N]) {
    for i in 1..N {
        let mut j = i;
        while j > 0 && items[j - 1] > items[j] {
            items.swap(j - 1, j);
            j -= 1;
        }
    }
}
