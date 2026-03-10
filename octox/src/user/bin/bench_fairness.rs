#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// CPU share fairness benchmark.
/// Forks N CPU-bound children that busy-loop for a fixed duration.
/// Children exit with (count / 1000) as status. Parent collects
/// and prints results sequentially to avoid interleaved output.
/// Output: BENCH:fairness:pid=<P>:count=<C>
fn main() {
    let num_children: usize = 4;
    let duration: usize = 50; // ticks (~5 seconds)

    let t_start = sys::uptime().unwrap();
    let mut child_pids: [usize; 4] = [0; 4];

    for i in 0..num_children {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            // child: CPU-bound busy loop
            let mut counter: usize = 0;
            loop {
                counter += 1;
                if counter % 10000 == 0 {
                    if sys::uptime().unwrap() - t_start >= duration {
                        break;
                    }
                }
            }
            // Exit with count/1000 (fits in exit status).
            // Parent reconstructs approximate count.
            sys::exit((counter / 1000) as i32);
        }
        child_pids[i] = pid;
    }

    // Parent: wait and print results sequentially (no interleaving)
    let mut status: i32 = 0;
    for _ in 0..num_children {
        let pid = sys::wait(&mut status).unwrap();
        let count = status as usize * 1000; // approximate
        println!("BENCH:fairness:pid={}:count={}", pid, count);
    }
}
