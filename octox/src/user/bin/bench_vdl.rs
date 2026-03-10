#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Virtual deadline latency bound benchmark.
///
/// Scenario: 8 CPU-bound processes contend for CPU. Each child tracks
/// the maximum gap (in ticks) between consecutive scheduling opportunities.
/// This measures worst-case tail latency — the longest any process has to
/// wait before getting the CPU again.
///
/// Output: BENCH:vdl:max_gap=<N>:avg_gap=<M>:spread=<S>
fn main() {
    let num_children: usize = 8;
    let duration: usize = 50; // ticks

    let t_start = sys::uptime().unwrap();

    for _ in 0..num_children {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            // Child: track max gap between scheduling rounds.
            let mut max_gap: usize = 0;
            let mut prev_time = sys::uptime().unwrap();
            let mut total_gap: usize = 0;
            let mut num_gaps: usize = 0;

            loop {
                // Do a short CPU burst.
                let mut work: usize = 0;
                for _ in 0..5000 {
                    work += 1;
                }
                // Prevent optimizer from removing the loop.
                let _ = work;

                let now = sys::uptime().unwrap();
                let gap = now - prev_time;
                if gap > 0 && num_gaps > 0 {
                    // Only count after first iteration (skip startup).
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

            // Pack max_gap (upper 16 bits) and avg_gap*100 (lower 16 bits).
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
    // spread = max_gap * 100 - avg_gap * 100  (in hundredths of a tick)
    let spread = (global_max_gap * 100).saturating_sub(avg_gap_100);

    println!(
        "BENCH:vdl:max_gap={}:avg_gap_100={}:spread={}",
        global_max_gap, avg_gap_100, spread
    );
}
