#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Context switch overhead benchmark.
/// For each concurrency level N in [1, 2, 4, 8], forks N CPU-bound
/// children that count iterations for a fixed duration. Children exit
/// with (count / 1000) as status. Parent prints results sequentially.
/// Output: BENCH:overhead:n=<N>:pid=<P>:count=<C>
fn main() {
    let levels: [usize; 4] = [1, 2, 4, 8];
    let duration: usize = 50;

    for &n in levels.iter() {
        let t_start = sys::uptime().unwrap();

        for _ in 0..n {
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

        // Wait for all children at this level, print sequentially
        let mut status: i32 = 0;
        for _ in 0..n {
            let pid = sys::wait(&mut status).unwrap();
            let count = status as usize * 1000;
            println!("BENCH:overhead:n={}:pid={}:count={}", n, pid, count);
        }
    }
}
