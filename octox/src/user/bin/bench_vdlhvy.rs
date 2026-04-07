#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Heavy virtual deadline latency benchmark.
///
/// 48 CPU-bound processes contend for 4 cores (12x oversubscription).
/// Each child tracks the maximum gap (in ticks) between consecutive
/// scheduling turns. Under heavy load, the gap distribution reveals
/// the scheduler's worst-case latency guarantees.
///
/// Output: BENCH:vdlhvy:max_gap=<N>:avg_gap_100=<M>:spread=<S>
fn main() {
    let num_children: usize = 48;
    let duration: usize = 60;

    let t_start = sys::uptime().unwrap();

    for _ in 0..num_children {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut max_gap: usize = 0;
            let mut prev_time = sys::uptime().unwrap();
            let mut total_gap: usize = 0;
            let mut num_gaps: usize = 0;

            loop {
                // Short CPU burst.
                let mut work: usize = 0;
                for _ in 0..5000 {
                    work += 1;
                }
                let _ = work;

                let now = sys::uptime().unwrap();
                let gap = now - prev_time;
                if gap > 0 && num_gaps > 0 {
                    total_gap += gap;
                    if gap > max_gap {
                        max_gap = gap;
                    }
                }
                num_gaps += 1;
                prev_time = now;

                if now - t_start >= duration {
                    break;
                }
            }

            let avg_gap_100 = if num_gaps > 1 {
                (total_gap * 100) / (num_gaps - 1)
            } else {
                0
            };
            let code = ((max_gap & 0xFFFF) << 16) | (avg_gap_100 & 0xFFFF);
            sys::exit(code as i32);
        }
    }

    // Collect results.
    let mut global_max_gap: usize = 0;
    let mut sum_avg_gap_100: usize = 0;
    let mut status: i32 = 0;

    for _ in 0..num_children {
        sys::wait(&mut status).unwrap();
        let code = status as usize;
        let child_max = (code >> 16) & 0xFFFF;
        let child_avg_100 = code & 0xFFFF;

        if child_max > global_max_gap {
            global_max_gap = child_max;
        }
        sum_avg_gap_100 += child_avg_100;
    }

    let avg_gap_100 = sum_avg_gap_100 / num_children.max(1);
    let spread = (global_max_gap * 100).saturating_sub(avg_gap_100);

    println!(
        "BENCH:vdlhvy:max_gap={}:avg_gap_100={}:spread={}",
        global_max_gap, avg_gap_100, spread
    );
}
