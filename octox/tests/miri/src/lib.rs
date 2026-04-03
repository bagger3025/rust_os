// ============================================================================
// Miri UB tests for Octox scheduler algorithms
// ============================================================================
//
// The kernel crate targets riscv64gc-unknown-none-elf and cannot be compiled
// for the host, so this standalone crate extracts the *pure algorithmic* parts
// of each scheduler and tests them under Miri to catch:
//
//   - Integer overflow / underflow (debug-mode panics → caught by Miri)
//   - Array out-of-bounds access
//   - Stacked Borrows / aliasing violations
//   - Data races in concurrent atomic patterns
//   - Use of uninitialized memory
//   - Violations of type invariants
//
// Run:  cd tests/miri && cargo +nightly miri test
//

// ============================================================================
// Extracted constants (matching src/kernel/param.rs and scheduler files)
// ============================================================================

const NPROC: usize = 64;

// ---------------------------------------------------------------------------
// O(1) scheduler helpers (from src/kernel/scheduler/o1.rs)
// ---------------------------------------------------------------------------

const NUM_PRIORITIES: usize = 40;
const BASE_PRIORITY: u8 = 20;
const MIN_TIMESLICE: u64 = 1;
const MAX_TIMESLICE: u64 = 6;
const MAX_SLEEP_AVG: u64 = 5;
const MAX_BONUS: u8 = 5;

fn o1_calc_priority(sleep_avg: u64) -> u8 {
    let bonus = (sleep_avg as u8).min(MAX_BONUS);
    BASE_PRIORITY.saturating_sub(bonus)
}

fn o1_calc_timeslice(prio: u8) -> u64 {
    let p = prio.min((NUM_PRIORITIES - 1) as u8) as u64;
    let max_p = (NUM_PRIORITIES - 1) as u64;
    MIN_TIMESLICE + (MAX_TIMESLICE - MIN_TIMESLICE) * (max_p - p) / max_p
}

// ---------------------------------------------------------------------------
// EEVDF helpers (from src/kernel/scheduler/eevdf.rs)
// ---------------------------------------------------------------------------

const NICE_0_WEIGHT: u64 = 1024;
const SCHED_SLICE: u64 = 3;

fn eevdf_calc_delta(delta_exec: u64, weight: u64) -> u64 {
    delta_exec * NICE_0_WEIGHT / weight.max(1)
}

fn eevdf_calc_slice_vruntime(weight: u64) -> u64 {
    SCHED_SLICE * NICE_0_WEIGHT / weight.max(1)
}

// ---------------------------------------------------------------------------
// CFS helpers (from src/kernel/scheduler/cfs.rs)
// ---------------------------------------------------------------------------

const SCHED_LATENCY: u64 = 8;
const SLEEPER_BONUS: u64 = SCHED_LATENCY / 2;

const WEIGHT_TABLE: [u64; 40] = [
    88761, 71755, 56483, 46273, 36291,
    29154, 23254, 18705, 14949, 11916,
     9548,  7620,  6100,  4904,  3906,
     3121,  2501,  1991,  1586,  1277,
     1024,   820,   655,   526,   423,
      335,   272,   215,   172,   137,
      110,    87,    70,    56,    45,
       36,    29,    23,    18,    15,
];

fn cfs_weight_of_nice(nice: i8) -> u64 {
    let idx = (nice as i32 + 20).clamp(0, 39) as usize;
    WEIGHT_TABLE[idx]
}

fn cfs_calc_delta(delta_exec: u64, weight: u64) -> u64 {
    delta_exec * NICE_0_WEIGHT / weight.max(1)
}

// ---------------------------------------------------------------------------
// MLFQ constants (from src/kernel/scheduler/mlfq.rs)
// ---------------------------------------------------------------------------

const NUM_LEVELS: usize = 3;
const TIME_ALLOTMENT: [u64; NUM_LEVELS] = [1, 2, 4];
const BOOST_INTERVAL: u64 = 100;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    // ====================================================================
    //  O(1) Scheduler Tests
    // ====================================================================

    #[test]
    fn o1_calc_priority_boundaries() {
        // sleep_avg = 0 (CPU-bound) → base priority
        assert_eq!(o1_calc_priority(0), BASE_PRIORITY);
        // sleep_avg = MAX_SLEEP_AVG (I/O-bound) → highest priority boost
        assert_eq!(o1_calc_priority(MAX_SLEEP_AVG), BASE_PRIORITY - MAX_BONUS);
        // sleep_avg > MAX_SLEEP_AVG → clamped to MAX_BONUS
        assert_eq!(o1_calc_priority(100), BASE_PRIORITY - MAX_BONUS);
        assert_eq!(o1_calc_priority(u64::MAX), BASE_PRIORITY - MAX_BONUS);
    }

    #[test]
    fn o1_calc_priority_monotonic() {
        // Higher sleep_avg should give lower (better) priority number.
        let mut prev = o1_calc_priority(0);
        for sa in 1..=MAX_SLEEP_AVG {
            let prio = o1_calc_priority(sa);
            assert!(prio <= prev, "priority not monotonically decreasing");
            prev = prio;
        }
    }

    #[test]
    fn o1_calc_timeslice_range() {
        // Every priority should produce a timeslice in [MIN, MAX].
        for prio in 0..=39u8 {
            let ts = o1_calc_timeslice(prio);
            assert!(
                ts >= MIN_TIMESLICE && ts <= MAX_TIMESLICE,
                "prio={prio}: timeslice={ts} not in [{MIN_TIMESLICE}, {MAX_TIMESLICE}]"
            );
        }
    }

    #[test]
    fn o1_calc_timeslice_monotonic() {
        // Lower priority number (higher priority) → longer timeslice.
        let mut prev = o1_calc_timeslice(0);
        for prio in 1..=39u8 {
            let ts = o1_calc_timeslice(prio);
            assert!(ts <= prev, "timeslice not monotonically decreasing with priority");
            prev = ts;
        }
    }

    #[test]
    fn o1_calc_timeslice_extremes() {
        assert_eq!(o1_calc_timeslice(0), MAX_TIMESLICE);
        assert_eq!(o1_calc_timeslice(39), MIN_TIMESLICE);
        // Out-of-range priority should be clamped.
        assert_eq!(o1_calc_timeslice(255), MIN_TIMESLICE);
    }

    #[test]
    fn o1_epoch_swap_simulation() {
        // Simulate the epoch swap logic with arrays of atomics.
        use std::sync::atomic::AtomicBool;

        let sleep_avg: Vec<AtomicU64> = (0..NPROC).map(|_| AtomicU64::new(0)).collect();
        let is_expired: Vec<AtomicBool> = (0..NPROC).map(|_| AtomicBool::new(false)).collect();
        let dyn_priority: Vec<AtomicU8> =
            (0..NPROC).map(|_| AtomicU8::new(BASE_PRIORITY)).collect();
        let timeslice: Vec<AtomicU64> = (0..NPROC).map(|_| AtomicU64::new(0)).collect();

        use std::sync::atomic::AtomicU8;

        // Mark some processes as expired.
        is_expired[0].store(true, Ordering::Relaxed);
        is_expired[3].store(true, Ordering::Relaxed);
        sleep_avg[0].store(2, Ordering::Relaxed);
        sleep_avg[3].store(0, Ordering::Relaxed);
        sleep_avg[5].store(3, Ordering::Relaxed);

        // Simulate epoch_swap.
        for i in 0..NPROC {
            let was_expired = is_expired[i].load(Ordering::Relaxed);
            let sa = sleep_avg[i].load(Ordering::Relaxed);
            let new_sa = if was_expired {
                sa.saturating_sub(1)
            } else {
                (sa + 1).min(MAX_SLEEP_AVG)
            };
            sleep_avg[i].store(new_sa, Ordering::Relaxed);
            let prio = o1_calc_priority(new_sa);
            dyn_priority[i].store(prio, Ordering::Relaxed);
            timeslice[i].store(o1_calc_timeslice(prio), Ordering::Relaxed);
            is_expired[i].store(false, Ordering::Relaxed);
        }

        // Verify all expired flags cleared.
        for i in 0..NPROC {
            assert!(!is_expired[i].load(Ordering::Relaxed));
        }

        // Process 0 was expired with sa=2 → sa=1.
        assert_eq!(sleep_avg[0].load(Ordering::Relaxed), 1);
        // Process 3 was expired with sa=0 → sa=0 (saturating_sub).
        assert_eq!(sleep_avg[3].load(Ordering::Relaxed), 0);
        // Process 5 was NOT expired with sa=3 → sa=4.
        assert_eq!(sleep_avg[5].load(Ordering::Relaxed), 4);
    }

    // ====================================================================
    //  EEVDF Tests
    // ====================================================================

    #[test]
    fn eevdf_calc_delta_identity() {
        // With nice-0 weight, delta_exec passes through unchanged.
        assert_eq!(eevdf_calc_delta(1, NICE_0_WEIGHT), 1);
        assert_eq!(eevdf_calc_delta(5, NICE_0_WEIGHT), 5);
    }

    #[test]
    fn eevdf_calc_delta_higher_weight() {
        // Higher weight → smaller vruntime delta (more CPU share).
        let delta_high = eevdf_calc_delta(10, NICE_0_WEIGHT * 2);
        let delta_norm = eevdf_calc_delta(10, NICE_0_WEIGHT);
        assert!(delta_high < delta_norm);
    }

    #[test]
    fn eevdf_calc_delta_lower_weight() {
        // Lower weight → larger vruntime delta (less CPU share).
        let delta_low = eevdf_calc_delta(10, NICE_0_WEIGHT / 2);
        let delta_norm = eevdf_calc_delta(10, NICE_0_WEIGHT);
        assert!(delta_low > delta_norm);
    }

    #[test]
    fn eevdf_calc_delta_zero_weight() {
        // Weight 0 should not divide by zero (clamped to 1).
        let delta = eevdf_calc_delta(10, 0);
        assert_eq!(delta, 10 * NICE_0_WEIGHT);
    }

    #[test]
    fn eevdf_slice_vruntime_consistency() {
        let slice_vrt = eevdf_calc_slice_vruntime(NICE_0_WEIGHT);
        assert_eq!(slice_vrt, SCHED_SLICE);
    }

    #[test]
    fn eevdf_eligibility_simulation() {
        // Simulate the two-pass EEVDF selection.
        let vruntimes = [10u64, 15, 20, 25];
        let deadlines = [13u64, 16, 22, 30];
        let n = vruntimes.len();

        // Pass 1: compute avg_vruntime.
        let sum_vrt: u64 = vruntimes.iter().sum();
        let avg_vrt = sum_vrt / n as u64;

        // Pass 2: find eligible with earliest deadline.
        let mut best_idx: Option<usize> = None;
        let mut best_dl: u64 = u64::MAX;

        for i in 0..n {
            if vruntimes[i] <= avg_vrt {
                if deadlines[i] < best_dl {
                    best_dl = deadlines[i];
                    best_idx = Some(i);
                }
            }
        }

        // avg_vrt = (10+15+20+25)/4 = 17.  Eligible: indices 0 (vrt=10) and 1 (vrt=15).
        // Earliest deadline among eligible: index 0 (dl=13).
        assert_eq!(best_idx, Some(0));
    }

    #[test]
    fn eevdf_deadline_renewal() {
        // Simulate deadline exhaustion and renewal.
        let mut vruntime = 10u64;
        let mut deadline = 13u64; // slice_vrt = 3

        // Charge vruntime: 10 → 11, deadline 13 not reached.
        vruntime += eevdf_calc_delta(1, NICE_0_WEIGHT);
        assert!(vruntime < deadline);

        // Charge more: 11 → 12, still below.
        vruntime += eevdf_calc_delta(1, NICE_0_WEIGHT);
        assert!(vruntime < deadline);

        // Charge: 12 → 13, reached deadline → renew.
        vruntime += eevdf_calc_delta(1, NICE_0_WEIGHT);
        assert!(vruntime >= deadline);
        deadline = vruntime + eevdf_calc_slice_vruntime(NICE_0_WEIGHT);
        assert_eq!(deadline, 16); // 13 + 3
    }

    // ====================================================================
    //  CFS Tests
    // ====================================================================

    #[test]
    fn cfs_weight_of_nice_boundaries() {
        // nice -20 → highest weight
        assert_eq!(cfs_weight_of_nice(-20), 88761);
        // nice 0 → 1024
        assert_eq!(cfs_weight_of_nice(0), 1024);
        // nice +19 → lowest weight
        assert_eq!(cfs_weight_of_nice(19), 15);
    }

    #[test]
    fn cfs_weight_of_nice_clamping() {
        // Out-of-range nice values should clamp to valid range.
        assert_eq!(cfs_weight_of_nice(-128), cfs_weight_of_nice(-20));
        assert_eq!(cfs_weight_of_nice(127), cfs_weight_of_nice(19));
    }

    #[test]
    fn cfs_weight_monotonically_decreasing() {
        for nice in -19..=19i8 {
            let w_lower = cfs_weight_of_nice(nice - 1);
            let w_higher = cfs_weight_of_nice(nice);
            assert!(
                w_lower > w_higher,
                "weight not monotonically decreasing: nice {} ({}) >= nice {} ({})",
                nice - 1, w_lower, nice, w_higher
            );
        }
    }

    #[test]
    fn cfs_calc_delta_zero_weight_safe() {
        // Should not divide by zero.
        let delta = cfs_calc_delta(10, 0);
        assert_eq!(delta, 10 * NICE_0_WEIGHT);
    }

    #[test]
    fn cfs_sleeper_bonus_value() {
        assert_eq!(SLEEPER_BONUS, 4);
    }

    #[test]
    fn cfs_vruntime_clamping_simulation() {
        // Simulate the wakeup vruntime clamping.
        let min_vruntime = 1000u64;
        let floor = min_vruntime.saturating_sub(SLEEPER_BONUS);

        // Process that slept a long time (vruntime far behind).
        let stale_vrt = 100u64;
        let clamped = stale_vrt.max(floor);
        assert_eq!(clamped, floor);
        assert_eq!(clamped, 996);

        // Process that was recently active (vruntime close to min).
        let fresh_vrt = 999u64;
        let clamped = fresh_vrt.max(floor);
        assert_eq!(clamped, 999); // not clamped since >= floor
    }

    // ====================================================================
    //  MLFQ Tests
    // ====================================================================

    #[test]
    fn mlfq_demotion_logic() {
        // Simulate Rule 4: demotion after time allotment exhausted.
        for level in 0..NUM_LEVELS - 1 {
            let mut ticks_at_level = 0u64;
            let mut demoted = false;

            for _ in 0..10 {
                ticks_at_level += 1;
                if level < NUM_LEVELS - 1 && ticks_at_level >= TIME_ALLOTMENT[level] {
                    demoted = true;
                    break;
                }
            }
            assert!(
                demoted,
                "level {level} should demote after {} ticks",
                TIME_ALLOTMENT[level]
            );
        }
    }

    #[test]
    fn mlfq_lowest_level_no_demotion() {
        // Level NUM_LEVELS-1 (lowest) should never trigger demotion.
        let level = NUM_LEVELS - 1;
        let ticks_at_level = 1000u64;
        let should_demote = level < NUM_LEVELS - 1 && ticks_at_level >= TIME_ALLOTMENT[level];
        assert!(!should_demote);
    }

    #[test]
    fn mlfq_boost_trigger() {
        // Exactly one CPU should trigger boost per interval.
        use std::sync::atomic::AtomicU64;
        let dispatch_count = AtomicU64::new(0);
        let mut boost_count = 0;

        for _ in 0..BOOST_INTERVAL * 3 {
            let count = dispatch_count.fetch_add(1, Ordering::Relaxed);
            if (count + 1) % BOOST_INTERVAL == 0 {
                boost_count += 1;
            }
        }
        assert_eq!(boost_count, 3);
    }

    #[test]
    fn mlfq_time_allotments_increasing() {
        // Higher levels (lower priority) should have longer allotments.
        for i in 1..NUM_LEVELS {
            assert!(
                TIME_ALLOTMENT[i] >= TIME_ALLOTMENT[i - 1],
                "allotment at level {} ({}) < level {} ({})",
                i, TIME_ALLOTMENT[i], i - 1, TIME_ALLOTMENT[i - 1]
            );
        }
    }

    // ====================================================================
    //  Concurrent Atomics Tests (Miri data-race detection)
    // ====================================================================

    #[test]
    fn concurrent_advance_min_vruntime() {
        // Test the CAS loop under concurrent access — Miri detects data races.
        let min_vruntime = Arc::new(AtomicU64::new(0));
        let mut handles = vec![];

        for t in 0..4 {
            let mv = Arc::clone(&min_vruntime);
            handles.push(std::thread::spawn(move || {
                for i in 0..100 {
                    let candidate = (t * 100 + i) as u64;
                    let _ = mv.fetch_update(Ordering::Release, Ordering::Relaxed, |cur| {
                        if candidate > cur {
                            Some(candidate)
                        } else {
                            None
                        }
                    });
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Final value should be the maximum across all threads.
        let final_val = min_vruntime.load(Ordering::Relaxed);
        assert_eq!(final_val, 399);
    }

    #[test]
    fn concurrent_epoch_swap_cas() {
        // Simulate the O(1) epoch swap CAS — threads race to increment
        // a shared epoch counter.  Each thread does multiple attempts;
        // verify that the final value equals the total number of successful
        // CAS operations (i.e., no lost updates).
        let epoch = Arc::new(AtomicU64::new(0));
        let winner_count = Arc::new(AtomicU64::new(0));
        let mut handles = vec![];

        for _ in 0..4 {
            let ep = Arc::clone(&epoch);
            let wc = Arc::clone(&winner_count);
            handles.push(std::thread::spawn(move || {
                for _ in 0..25 {
                    let cur = ep.load(Ordering::Acquire);
                    if ep
                        .compare_exchange(cur, cur + 1, Ordering::AcqRel, Ordering::Relaxed)
                        .is_ok()
                    {
                        wc.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // The epoch value must equal the number of successful CAS ops.
        let final_epoch = epoch.load(Ordering::Relaxed);
        let total_wins = winner_count.load(Ordering::Relaxed);
        assert_eq!(final_epoch, total_wins, "epoch != winner_count: lost update?");
        // With 4 threads × 25 attempts, the total should be up to 100.
        assert!(total_wins > 0 && total_wins <= 100);
    }

    #[test]
    fn concurrent_dispatch_counter() {
        // Simulate MLFQ dispatch counter with boost detection.
        let dispatch_count = Arc::new(AtomicU64::new(0));
        let boost_count = Arc::new(AtomicU64::new(0));
        let mut handles = vec![];

        for _ in 0..4 {
            let dc = Arc::clone(&dispatch_count);
            let bc = Arc::clone(&boost_count);
            handles.push(std::thread::spawn(move || {
                for _ in 0..25 {
                    let count = dc.fetch_add(1, Ordering::Relaxed);
                    if (count + 1) % BOOST_INTERVAL == 0 {
                        bc.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(dispatch_count.load(Ordering::Relaxed), 100);
        assert_eq!(boost_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn concurrent_vruntime_fetch_add() {
        // Simulate CFS vruntime charging across CPUs.
        let vruntime = Arc::new(AtomicU64::new(0));
        let mut handles = vec![];
        let delta = cfs_calc_delta(1, NICE_0_WEIGHT); // = 1

        for _ in 0..4 {
            let vrt = Arc::clone(&vruntime);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    vrt.fetch_add(delta, Ordering::Release);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(vruntime.load(Ordering::Relaxed), 400);
    }

    #[test]
    fn concurrent_per_process_atomics() {
        // Simulate concurrent access to per-process atomic arrays
        // (as done by multiple CPUs scanning the process table).
        let queue_levels: Arc<Vec<AtomicU8>> =
            Arc::new((0..NPROC).map(|_| AtomicU8::new(0)).collect());
        let ticks: Arc<Vec<AtomicU64>> =
            Arc::new((0..NPROC).map(|_| AtomicU64::new(0)).collect());
        let mut handles = vec![];

        use std::sync::atomic::AtomicU8;

        for cpu in 0..4u8 {
            let ql = Arc::clone(&queue_levels);
            let tk = Arc::clone(&ticks);
            handles.push(std::thread::spawn(move || {
                // Each CPU scans and potentially modifies different slots.
                for j in 0..NPROC {
                    let i = (cpu as usize * 4 + j) % NPROC;
                    let level = ql[i].load(Ordering::Relaxed);
                    let t = tk[i].fetch_add(1, Ordering::Relaxed) + 1;

                    if level < (NUM_LEVELS - 1) as u8
                        && t >= TIME_ALLOTMENT[level as usize]
                    {
                        ql[i].store(level + 1, Ordering::Relaxed);
                        tk[i].store(0, Ordering::Relaxed);
                    }
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // No crash or data race detected = pass.
    }

    // ====================================================================
    //  Arithmetic Edge Cases (integer overflow checks)
    // ====================================================================

    #[test]
    fn saturating_arithmetic_no_overflow() {
        // Verify saturating operations handle extremes.
        assert_eq!(i64::MAX.saturating_add(1), i64::MAX);
        assert_eq!(i64::MIN.saturating_sub(1), i64::MIN);
        assert_eq!(0i64.saturating_add(0), 0);
    }

    #[test]
    fn o1_timeslice_division_no_remainder_issue() {
        // Verify integer division rounding doesn't produce 0.
        for prio in 0..=255u8 {
            let ts = o1_calc_timeslice(prio);
            assert!(ts >= MIN_TIMESLICE, "timeslice for prio {prio} is 0");
        }
    }

    #[test]
    fn eevdf_large_vruntime_no_overflow() {
        // Simulate very long-running system where vruntimes are large.
        let vrt = u64::MAX - 100;
        let delta = eevdf_calc_delta(1, NICE_0_WEIGHT);
        let new_vrt = vrt.wrapping_add(delta); // wrapping is fine for vruntime
        // In the real scheduler, fetch_add wraps. Check slice_vruntime is still valid.
        let slice = eevdf_calc_slice_vruntime(NICE_0_WEIGHT);
        assert_eq!(slice, SCHED_SLICE);
        let _ = new_vrt; // just ensure no panic
    }

    #[test]
    fn mlfq_scan_offset_wraparound() {
        // Verify modular arithmetic for scan offset.
        let mut scan_offset = 0usize;
        for _ in 0..NPROC * 3 {
            for j in 0..NPROC {
                let i = (scan_offset + j) % NPROC;
                assert!(i < NPROC);
            }
            scan_offset = (scan_offset + 1) % NPROC;
        }
    }
    //
    //  In the kernel code, `self.w1[j][k] * f[k] / SCALE` does a plain
    //  multiply.  If weights drift far from their initial range after many
    //  perturbation epochs, the multiplication could overflow in debug mode.
    //  This test verifies that the warm-start weights plus a realistic
    //  number of perturbations stay within safe bounds.
    // ====================================================================
    //  Forward pass with perturbed weights (integration-style)
    // ====================================================================
    //  Scenario Benchmark Logic Tests
    //  Test the arithmetic and data encoding used in each new benchmark.
    // ====================================================================

    // ---- cpusym: symmetric CPU throughput ----

    #[test]
    fn cpusym_exit_status_encoding() {
        // Children exit with counter/1000. Verify no overflow for realistic counts.
        for counter in [0usize, 1000, 100_000, 1_000_000, 10_000_000] {
            let code = (counter / 1000) as i32;
            let reconstructed = code as usize * 1000;
            // Reconstruction should be close (within 1000 of original).
            assert!(
                counter.abs_diff(reconstructed) < 1000,
                "counter={counter} reconstructed={reconstructed}"
            );
        }
    }

    #[test]
    fn cpusym_min_tracking() {
        let counts = [5000usize, 3000, 7000, 4000, 6000, 2000, 8000, 1000];
        let mut total = 0usize;
        let mut min_work = usize::MAX;
        for &c in &counts {
            total += c;
            if c < min_work {
                min_work = c;
            }
        }
        assert_eq!(total, 36000);
        assert_eq!(min_work, 1000);
    }

    // ---- sleepw: sleeper wakeup bonus ----

    #[test]
    fn sleepw_exit_status_packing() {
        // avg in upper 8 bits, max in lower 8 bits.
        for avg in 0..=255u8 {
            for max_d in [0u8, 1, 5, 100, 255] {
                let code = ((avg as usize & 0xFF) << 8) | (max_d as usize & 0xFF);
                let decoded_avg = (code >> 8) & 0xFF;
                let decoded_max = code & 0xFF;
                assert_eq!(decoded_avg, avg as usize);
                assert_eq!(decoded_max, max_d as usize);
                // Ensure fits in i32.
                assert!(code <= i32::MAX as usize);
            }
        }
    }

    #[test]
    fn sleepw_delay_calculation() {
        // Wakeup delay = actual - requested.
        let sleep_ticks = 1;
        for actual in [1usize, 2, 3, 5, 10] {
            let delay = if actual > sleep_ticks {
                actual - sleep_ticks
            } else {
                0
            };
            assert!(delay < actual);
        }
    }

    // ---- newproc: new process priority ----

    #[test]
    fn newproc_latency_measurement() {
        // Simulated latencies for 10 measurements.
        let latencies = [0usize, 1, 0, 0, 1, 0, 0, 2, 0, 1];
        let total: usize = latencies.iter().sum();
        let max = *latencies.iter().max().unwrap();
        let avg = total / latencies.len();
        assert_eq!(total, 5);
        assert_eq!(max, 2);
        assert_eq!(avg, 0); // integer division: 5/10 = 0
    }

    #[test]
    fn newproc_mlfq_demotion_timing() {
        // Verify that 20 ticks is enough for full MLFQ demotion.
        // Level 0: allotment 1 → demoted after 1 dispatch.
        // Level 1: allotment 2 → demoted after 2 dispatches.
        // Level 2: lowest (no further demotion).
        // Total dispatches for full demotion: 1 + 2 = 3 per process.
        // With 6 workers and 4 CPUs, each worker gets dispatched roughly
        // every 1.5 rounds. 3 dispatches × 1.5 = ~5 rounds minimum.
        // 20 ticks gives plenty of margin.
        let total_allotment: u64 = TIME_ALLOTMENT[0] + TIME_ALLOTMENT[1];
        assert_eq!(total_allotment, 3);
        assert!(20 > total_allotment as usize * 6 / 4);
    }

    // ---- vdl: virtual deadline tail-latency ----

    #[test]
    fn vdl_exit_status_packing() {
        // max_gap in upper 16 bits, avg_gap*100 in lower 16 bits.
        for max_gap in [0u16, 1, 5, 50, 1000] {
            for avg_gap_100 in [0u16, 50, 100, 500, 5000] {
                let code = ((max_gap as usize & 0xFFFF) << 16)
                    | (avg_gap_100 as usize & 0xFFFF);
                let decoded_max = (code >> 16) & 0xFFFF;
                let decoded_avg = code & 0xFFFF;
                assert_eq!(decoded_max, max_gap as usize);
                assert_eq!(decoded_avg, avg_gap_100 as usize);
                assert!(code <= i32::MAX as usize);
            }
        }
    }

    #[test]
    fn vdl_spread_calculation() {
        // spread = max_gap * 100 - avg_gap * 100.
        let max_gap = 5u64;
        let avg_gap_100 = 250u64; // avg = 2.5 ticks
        let spread = (max_gap * 100).saturating_sub(avg_gap_100);
        assert_eq!(spread, 250); // 500 - 250 = 250 hundredths = 2.5 ticks
    }

    #[test]
    fn vdl_eevdf_deadline_guarantees() {
        // Simulate EEVDF scheduling for 8 processes.
        // Each has vruntime and deadline. Verify that the selected process
        // (eligible + earliest deadline) rotates fairly.
        let n = 8;
        let mut vruntimes = vec![0u64; n];
        let mut deadlines: Vec<u64> = (0..n).map(|i| i as u64 * 3 + 3).collect();
        let mut dispatch_counts = vec![0u32; n];

        for round in 0..100 {
            // Compute avg_vrt.
            let sum: u64 = vruntimes.iter().sum();
            let avg = sum / n as u64;

            // Find eligible with earliest deadline.
            let mut best: Option<usize> = None;
            let mut best_dl = u64::MAX;
            for i in 0..n {
                if vruntimes[i] <= avg && deadlines[i] < best_dl {
                    best_dl = deadlines[i];
                    best = Some(i);
                }
            }
            // Fallback to lowest vruntime.
            let chosen = best.unwrap_or_else(|| {
                vruntimes
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, &v)| v)
                    .unwrap()
                    .0
            });

            // Dispatch.
            vruntimes[chosen] += 1; // charge 1 unit
            dispatch_counts[chosen] += 1;
            // Renew deadline if exhausted.
            if vruntimes[chosen] >= deadlines[chosen] {
                deadlines[chosen] = vruntimes[chosen] + 3;
            }
        }

        // With EEVDF, all processes should get roughly equal dispatches.
        let min_d = *dispatch_counts.iter().min().unwrap();
        let max_d = *dispatch_counts.iter().max().unwrap();
        assert!(
            max_d - min_d <= 3,
            "EEVDF dispatch spread too large: min={min_d} max={max_d}"
        );
    }

    // ---- epoch: interactive/batch separation ----

    #[test]
    fn epoch_o1_interactivity_heuristic() {
        // Simulate O(1) epoch swaps. Interactive processes (not expired)
        // should accumulate sleep_avg and get better priority.
        let mut sleep_avg = [0u64; 8];
        let mut is_expired = [false; 8];

        // Processes 0-3 are batch (always expire).
        // Processes 4-6 are interactive (never expire — they sleep).
        for _epoch in 0..10 {
            // Mark batch as expired.
            for i in 0..4 {
                is_expired[i] = true;
            }

            // Epoch swap.
            for i in 0..7 {
                let sa = sleep_avg[i];
                sleep_avg[i] = if is_expired[i] {
                    sa.saturating_sub(1)
                } else {
                    (sa + 1).min(MAX_SLEEP_AVG)
                };
                is_expired[i] = false;
            }
        }

        // Batch processes should have sleep_avg = 0.
        for i in 0..4 {
            assert_eq!(sleep_avg[i], 0);
        }
        // Interactive processes should have sleep_avg = MAX_SLEEP_AVG.
        for i in 4..7 {
            assert_eq!(sleep_avg[i], MAX_SLEEP_AVG);
        }

        // Priority difference.
        let batch_prio = o1_calc_priority(sleep_avg[0]);
        let interactive_prio = o1_calc_priority(sleep_avg[4]);
        assert!(interactive_prio < batch_prio, "interactive should have better (lower) priority");
        assert_eq!(batch_prio, BASE_PRIORITY);
        assert_eq!(interactive_prio, BASE_PRIORITY - MAX_BONUS);

        // Timeslice difference.
        let batch_ts = o1_calc_timeslice(batch_prio);
        let interactive_ts = o1_calc_timeslice(interactive_prio);
        assert!(interactive_ts > batch_ts, "interactive should get longer timeslice");
    }

    #[test]
    fn epoch_response_time_encoding() {
        // Interactive children exit with avg_resp * 100.
        for total_resp in [0usize, 10, 100, 500] {
            let io_cycles = 25;
            let avg_resp_100 = (total_resp * 100) / io_cycles.max(1);
            assert!(avg_resp_100 <= i32::MAX as usize);
            let decoded = avg_resp_100 as f64 / 100.0;
            let expected = total_resp as f64 / io_cycles as f64;
            assert!((decoded - expected).abs() < 0.1);
        }
    }

    // ---- adapt: adaptive workload phase change ----

    #[test]
    fn adapt_exit_status_encoding() {
        // CPU children exit with phase2_count / 1000.
        // I/O children exit with total_delay (small number).
        // Parent uses PIDs for categorization (not exit status format).
        for phase2_count in [0usize, 1_000_000, 10_000_000, 100_000_000] {
            let code = (phase2_count / 1000) as i32;
            let reconstructed = code as usize * 1000;
            assert!(phase2_count.abs_diff(reconstructed) < 1000);
        }
        for total_delay in [0usize, 1, 5, 15, 50] {
            let code = total_delay as i32;
            assert_eq!(code as usize, total_delay);
        }
    }

    #[test]
    fn adapt_pid_categorization() {
        // Parent tracks PIDs to distinguish CPU vs I/O children.
        let cpu_pids: [usize; 3] = [3, 4, 5];
        let io_pids: [usize; 3] = [6, 7, 8];
        for &pid in &cpu_pids {
            assert!(cpu_pids.iter().any(|&cp| cp == pid));
            assert!(!io_pids.iter().any(|&ip| ip == pid));
        }
        for &pid in &io_pids {
            assert!(!cpu_pids.iter().any(|&cp| cp == pid));
            assert!(io_pids.iter().any(|&ip| ip == pid));
        }
    }
    // ---- Concurrent scenario: simulate scheduler contention patterns ----

    #[test]
    fn concurrent_mlfq_boost_with_demotions() {
        // Simulate MLFQ with concurrent demotions and periodic boosts.
        use std::sync::atomic::AtomicU8;

        let queue_levels: Arc<Vec<AtomicU8>> =
            Arc::new((0..NPROC).map(|_| AtomicU8::new(0)).collect());
        let ticks: Arc<Vec<AtomicU64>> =
            Arc::new((0..NPROC).map(|_| AtomicU64::new(0)).collect());
        let dispatch_count = Arc::new(AtomicU64::new(0));
        let mut handles = vec![];

        for _ in 0..4 {
            let ql = Arc::clone(&queue_levels);
            let tk = Arc::clone(&ticks);
            let dc = Arc::clone(&dispatch_count);
            handles.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    // Simulate dispatch + demotion check.
                    let idx = (dc.load(Ordering::Relaxed) as usize) % NPROC;
                    let level = ql[idx].load(Ordering::Relaxed) as usize;
                    let t = tk[idx].fetch_add(1, Ordering::Relaxed) + 1;
                    if level < NUM_LEVELS - 1 && t >= TIME_ALLOTMENT[level] {
                        ql[idx].store((level + 1) as u8, Ordering::Relaxed);
                        tk[idx].store(0, Ordering::Relaxed);
                    }

                    // Boost check.
                    let count = dc.fetch_add(1, Ordering::Relaxed);
                    if (count + 1) % BOOST_INTERVAL == 0 {
                        for i in 0..NPROC {
                            ql[i].store(0, Ordering::Relaxed);
                            tk[i].store(0, Ordering::Relaxed);
                        }
                    }
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
        // No data race or panic = pass.
    }

    #[test]
    fn concurrent_cfs_sleeper_bonus() {
        // Simulate CFS vruntime clamping (sleeper bonus) under contention.
        let min_vruntime = Arc::new(AtomicU64::new(100));
        let vruntimes: Arc<Vec<AtomicU64>> =
            Arc::new((0..8).map(|i| AtomicU64::new(100 + i as u64)).collect());
        let mut handles = vec![];

        for cpu in 0..4u8 {
            let mv = Arc::clone(&min_vruntime);
            let vrts = Arc::clone(&vruntimes);
            handles.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    let idx = (cpu as usize * 13 + 7) % 8; // pseudo-random index
                    let floor = mv.load(Ordering::Relaxed).saturating_sub(SLEEPER_BONUS);
                    let cur_vrt = vrts[idx].load(Ordering::Relaxed);
                    if cur_vrt < floor {
                        vrts[idx].store(floor, Ordering::Relaxed);
                    }
                    // Charge vruntime.
                    let delta = cfs_calc_delta(1, NICE_0_WEIGHT);
                    let new_vrt = vrts[idx].fetch_add(delta, Ordering::Release) + delta;
                    // Advance min_vruntime.
                    let _ = mv.fetch_update(
                        Ordering::Release,
                        Ordering::Relaxed,
                        |cur| if new_vrt > cur { Some(new_vrt) } else { None },
                    );
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
        // min_vruntime should have advanced.
        assert!(min_vruntime.load(Ordering::Relaxed) > 100);
    }
}
