#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Fork/exit throughput benchmark.
/// Measures how many fork+exit+wait cycles complete in a fixed time window.
/// Output: BENCH:throughput:count=<N> BENCH:throughput:ticks=<T>
fn main() {
    let duration = 100; // ticks (~10 seconds)
    let t0 = sys::uptime().unwrap();
    let mut count: usize = 0;

    loop {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            // child: exit immediately
            sys::exit(0);
        }
        // parent: wait for child
        let mut status: i32 = 0;
        sys::wait(&mut status).unwrap();
        count += 1;

        if sys::uptime().unwrap() - t0 >= duration {
            break;
        }
    }

    let elapsed = sys::uptime().unwrap() - t0;
    println!("BENCH:throughput:count={}:ticks={}", count, elapsed);
}
