#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Starvation resistance benchmark.
///
/// 40 CPU-bound children saturate the system while 8 "probe" children
/// alternate between CPU work and brief sleeps. After 80 ticks, we
/// check that ALL 48 children completed meaningful work. The min/max
/// work ratio reveals whether any process was starved.
///
/// Output: BENCH:starve:pid=<P>:count=<C>  (48 lines)
fn main() {
    let num_cpu: usize = 40;
    let num_probes: usize = 8;
    let duration: usize = 80;

    let t_start = sys::uptime().unwrap();

    // Fork CPU-bound children.
    for _ in 0..num_cpu {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut counter: usize = 0;
            loop {
                counter += 1;
                if counter % 10000 == 0 {
                    if sys::uptime().unwrap() - t_start >= duration {
                        break;
                    }
                }
            }
            sys::exit((counter / 1000) as i32);
        }
    }

    // Fork probe children: alternate CPU work and sleep.
    for _ in 0..num_probes {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut counter: usize = 0;
            loop {
                // CPU work phase.
                for _ in 0..50000 {
                    counter += 1;
                }
                // Brief sleep.
                sys::sleep(1).unwrap();
                if sys::uptime().unwrap() - t_start >= duration {
                    break;
                }
            }
            sys::exit((counter / 1000) as i32);
        }
    }

    // Collect results.
    let mut status: i32 = 0;
    for _ in 0..(num_cpu + num_probes) {
        let pid = sys::wait(&mut status).unwrap();
        let count = status as usize * 1000;
        println!("BENCH:starve:pid={}:count={}", pid, count);
    }
}
