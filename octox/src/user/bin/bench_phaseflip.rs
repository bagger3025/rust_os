#![no_std]
extern crate alloc;
use ulib::{print, println, sys};

/// Phase-flapping workload.
///
/// 16 stable CPU workers run beside 16 flapping workers. Flappers alternate
/// every 10 ticks between I/O-like sleep/wakeup behavior and CPU-heavy work.
///
/// Output: BENCH:phaseflip:delay_p95=<N>:delay_max=<N>:cpu_work=<N>:
/// flip_work=<N>
fn main() {
    let num_cpu: usize = 16;
    let num_flip: usize = 16;
    let duration: usize = 80;
    let phase_len: usize = 10;
    let cycles: usize = 24;

    let start = sys::uptime().unwrap();
    let end = start + duration;

    let mut cpu_pids: [usize; 16] = [0; 16];
    for i in 0..num_cpu {
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
        cpu_pids[i] = pid;
    }

    for _ in 0..num_flip {
        let pid = sys::fork().unwrap();
        if pid == 0 {
            let mut delays: [usize; 24] = [0; 24];
            let mut max_delay: usize = 0;
            let mut work: usize = 0;
            for i in 0..cycles {
                let now = sys::uptime().unwrap();
                if now >= end {
                    break;
                }
                let phase = ((now - start) / phase_len) % 2;
                if phase == 0 {
                    let before = sys::uptime().unwrap();
                    sys::sleep(1).unwrap();
                    let after = sys::uptime().unwrap();
                    if after > before + 1 {
                        let delay = after - before - 1;
                        delays[i] = delay;
                        if delay > max_delay {
                            max_delay = delay;
                        }
                    }
                    for _ in 0..30_000 {
                        work += 1;
                    }
                } else {
                    for _ in 0..250_000 {
                        work += 1;
                    }
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
                | ((work / 1000) & 0x3FFF);
            sys::exit(code as i32);
        }
    }

    let mut status: i32 = 0;
    let mut cpu_work: usize = 0;
    let mut flip_work: usize = 0;
    let mut delay_p95: usize = 0;
    let mut delay_max: usize = 0;

    for _ in 0..(num_cpu + num_flip) {
        let pid = sys::wait(&mut status).unwrap();
        let is_cpu = cpu_pids.iter().any(|&cp| cp == pid);
        if is_cpu {
            cpu_work += status as usize * 1000;
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
            flip_work += work;
        }
    }

    println!(
        "BENCH:phaseflip:delay_p95={}:delay_max={}:cpu_work={}:flip_work={}",
        delay_p95, delay_max, cpu_work, flip_work
    );
}
