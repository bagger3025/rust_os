#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Max process acceptance benchmark.
///
/// Attempts to fork close to the process-table capacity. The benchmark checks
/// that many children can be accepted and that eventual fork failure, if any,
/// is graceful rather than fatal.
///
/// Output: BENCH:maxproc:accepted=<N>:completed=<N>:fork_failed=<0|1>
fn main() {
    let target_children: usize = 60;
    let mut accepted: usize = 0;
    let mut fork_failed: usize = 0;

    for _ in 0..target_children {
        match sys::fork() {
            Ok(0) => {
                sys::sleep(2).unwrap();
                sys::exit(1);
            }
            Ok(_) => {
                accepted += 1;
            }
            Err(_) => {
                fork_failed = 1;
                break;
            }
        }
    }

    let mut completed: usize = 0;
    let mut status: i32 = 0;
    for _ in 0..accepted {
        if sys::wait(&mut status).is_ok() {
            completed += 1;
        }
    }

    println!(
        "BENCH:maxproc:accepted={}:completed={}:fork_failed={}",
        accepted, completed, fork_failed
    );
}
