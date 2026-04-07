#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Heavy fairness benchmark: 48 CPU-bound on 4 cores (12x oversubscription).
///
/// With only 4 children (existing benchfair), all schedulers look fair
/// because each child often gets its own core. At 48 children (12 per
/// core), the scheduling policy must actively distribute CPU time.
///
/// Output: BENCH:fairhvy:pid=<P>:count=<C>  (48 lines)
fn main() {
    let num_children: usize = 48;
    let duration: usize = 80;

    let t_start = sys::uptime().unwrap();

    for _ in 0..num_children {
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

    let mut status: i32 = 0;
    for _ in 0..num_children {
        let pid = sys::wait(&mut status).unwrap();
        let count = status as usize * 1000;
        println!("BENCH:fairhvy:pid={}:count={}", pid, count);
    }
}
