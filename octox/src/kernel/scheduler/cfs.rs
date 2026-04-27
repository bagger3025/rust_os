// ============================================================================
// CFS — Completely Fair Scheduler
// ============================================================================
//
// Modeled after Linux's CFS (introduced by kernel 2.6.23).
// Adapted for fixed process table (NPROC=64) and RISC-V timer-driven
// preemption model.
//
// Core idea: every process accumulates *virtual runtime* (vruntime) as it
// consumes CPU. The scheduler always picks the RUNNABLE process with the
// lowest vruntime, ensuring that every process gets its fair share of
// the processor over time.
//
// Linux uses a red-black tree for O(log n) leftmost extraction. With
// fixed NPROC table a linear scan is simple and predictable, so we use that.
//
// Key mechanisms:
//
//   vruntime — Weighted CPU consumption counter. Processes with higher
//              weight (lower nice) accumulate vruntime more slowly, so they
//              are scheduled more often. The formula is:
//                  delta_vruntime = delta_exec * (NICE_0_WEIGHT / weight)
//
//   min_vruntime — Monotonically increasing floor across all runnable/running
//                  tasks. New and waking tasks get their vruntime clamped to
//                  at least (min_vruntime - SLEEPER_BONUS) so they cannot
//                  starve long-running tasks.
//
//   weight table — Copied from Linux's sched_prio_to_weight[]. Each nice
//                  level shifts CPU share by ~10%. Currently all processes
//                  run at nice=0; the table is provided for future extension
//                  via a renice/setpriority syscall.
//
// Scheduling flow (per-CPU, never returns):
//
//   loop {
//       enable_interrupts();
//       Phase 1 — scan process table under per-process locks:
//                 find RUNNABLE with lowest vruntime,
//                 compute running min for min_vruntime update.
//       Advance min_vruntime (never backward).
//       Phase 2 — re-lock chosen process, verify still RUNNABLE:
//                 clamp its vruntime to floor,
//                 context-switch into it.
//       On return — charge vruntime for the scheduling round.
//   }
// ============================================================================

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use kernel::{
    param::NPROC,
    proc::{ProcState, CPUS, PROCS},
    riscv::intr_on,
    swtch::swtch,
};

use super::InstanceScheduler;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default weight corresponding to nice 0 (matches Linux's NICE_0_LOAD).
const NICE_0_WEIGHT: u64 = 1024;

/// Target scheduling latency — the ideal period (in abstract vruntime units,
/// where 1 unit ≈ one timer-tick at default weight) within which every
/// runnable process should receive at least one time slice.
const SCHED_LATENCY: u64 = 8;

/// Minimum vruntime charge per scheduling round. Prevents pathological
/// thrashing when many tasks have nearly equal vruntimes.
const SCHED_MIN_GRANULARITY: u64 = 1;

/// Maximum vruntime credit given to tasks that wake from sleep.
/// A waking task's vruntime is floored at (min_vruntime - SLEEPER_BONUS)
/// so it gets a small head-start (rewarding I/O-bound behavior) but cannot
/// monopolize the CPU.
const SLEEPER_BONUS: u64 = SCHED_LATENCY / 2;

/// Weight table for nice values -20 .. +19 (index = nice + 20).
/// Taken from Linux's `sched_prio_to_weight[]`.  Each adjacent pair has a
/// ratio of ~1.25, so one nice level ≈ 10 % CPU share difference.
///
/// Currently unused at runtime (all processes run at nice 0), but provided
/// for completeness and ready for a future `sys_setpriority` syscall.
#[allow(dead_code)]
const WEIGHT_TABLE: [u64; 40] = [
    /* nice -20 */ 88761, 71755, 56483, 46273, 36291,
    /* nice -15 */ 29154, 23254, 18705, 14949, 11916,
    /* nice -10 */  9548,  7620,  6100,  4904,  3906,
    /* nice  -5 */  3121,  2501,  1991,  1586,  1277,
    /* nice   0 */  1024,   820,   655,   526,   423,
    /* nice   5 */   335,   272,   215,   172,   137,
    /* nice  10 */   110,    87,    70,    56,    45,
    /* nice  15 */    36,    29,    23,    18,    15,
];

// ---------------------------------------------------------------------------
// Global shared CFS state (all CPUs share one view of virtual runtimes)
// ---------------------------------------------------------------------------

const VRUNTIME_ZERO: AtomicU64 = AtomicU64::new(0);

/// Per-process virtual runtime, indexed by position in `PROCS.pool`.
/// Atomics so that multiple CPUs can read/write without a global lock.
static VRUNTIME: [AtomicU64; NPROC] = [VRUNTIME_ZERO; NPROC];

/// Monotonically increasing floor — the smallest vruntime among all
/// active (RUNNABLE | RUNNING) tasks the last time we checked.  Only
/// moves forward; used to clamp stale vruntimes on wakeup / fork.
static MIN_VRUNTIME: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Look up the scheduling weight for a given nice value (-20 .. +19).
#[allow(dead_code)]
fn weight_of_nice(nice: i8) -> u64 {
    let idx = (nice as i32 + 20).clamp(0, 39) as usize;
    WEIGHT_TABLE[idx]
}

/// Compute the vruntime delta for one scheduling round:
///
///     delta_vruntime = delta_exec x (NICE_0_WEIGHT / weight)
///
/// Higher weight -> smaller delta -> process is scheduled more often.
#[inline]
fn calc_delta(delta_exec: u64, weight: u64) -> u64 {
    delta_exec * NICE_0_WEIGHT / weight.max(1)
}

/// Advance `MIN_VRUNTIME` to `candidate` if `candidate` is larger.
/// A compare-and-swap loop ensures the floor never goes backward even
/// under concurrent updates from multiple CPUs.
#[inline]
fn advance_min_vruntime(candidate: u64) {
    let _ = MIN_VRUNTIME.fetch_update(Ordering::Release, Ordering::Relaxed, |cur| {
        if candidate > cur {
            Some(candidate)
        } else {
            None
        }
    });
}

// ---------------------------------------------------------------------------
// Scheduler implementation
// ---------------------------------------------------------------------------

pub struct Cfs;

impl Default for Cfs {
    fn default() -> Self {
        Self
    }
}

impl InstanceScheduler for Cfs {
    /// Per-CPU CFS scheduling loop.  Never returns.
    ///
    /// Each iteration performs two phases:
    ///
    ///   Phase 1 (selection) — scan every slot in the process table,
    ///   locking each briefly to read its state.  Track:
    ///     - the RUNNABLE process with the lowest vruntime -> `best_idx`
    ///     - the minimum vruntime across all active tasks -> used to
    ///       advance `MIN_VRUNTIME`
    ///
    ///   Phase 2 (dispatch) — re-lock the chosen process, verify it
    ///   is still RUNNABLE (another CPU may have grabbed it in between),
    ///   clamp its vruntime, context-switch into it, and charge vruntime
    ///   when it yields back.
    fn instance_scheduler(&mut self) -> ! {
        let c = unsafe { CPUS.mycpu() };

        loop {
            // Avoid deadlock by ensuring devices can interrupt.
            intr_on();

            // Phase 1: find RUNNABLE with lowest vruntime
            let (best_idx, active_min_vrt) = {
                let mut best_idx: Option<usize> = None;
                let mut best_vrt: u64 = u64::MAX;
                let mut active_min_vrt: u64 = u64::MAX;

                for (i, p) in PROCS.pool.iter().enumerate() {
                    let inner = p.inner.lock();
                    match inner.state {
                        ProcState::RUNNABLE => {
                            let vrt = VRUNTIME[i].load(Ordering::Relaxed);
                            // Track global floor across all active tasks.
                            if vrt < active_min_vrt {
                                active_min_vrt = vrt;
                            }
                            // Track best candidate to schedule.
                            if vrt < best_vrt {
                                best_vrt = vrt;
                                best_idx = Some(i);
                            }
                        }
                        ProcState::RUNNING => {
                            // Running on another CPU — contributes to floor
                            // but is not a scheduling candidate.
                            let vrt = VRUNTIME[i].load(Ordering::Relaxed);
                            if vrt < active_min_vrt {
                                active_min_vrt = vrt;
                            }
                        }
                        _ => {}
                    }
                    // Lock dropped here; brief window where state may change,
                    // which Phase 2 re-checks.
                }

                (best_idx, active_min_vrt)
            };
            
            // Advance the global vruntime floor (never goes backward).
            if active_min_vrt < u64::MAX {
                advance_min_vruntime(active_min_vrt);
            }

            // Phase 2: dispatch the chosen process
            if let Some(idx) = best_idx {
                let p = &PROCS.pool[idx];
                let mut inner = p.inner.lock();

                // Re-check: another CPU may have taken it between phases.
                if inner.state == ProcState::RUNNABLE {
                    // Clamp vruntime for tasks that slept a long time (or
                    // just forked).  Without this, a task waking from a
                    // long sleep would have vruntime << min_vruntime and
                    // would monopolise the CPU until it catches up.
                    //
                    // We allow a small bonus (SLEEPER_BONUS) to reward
                    // I/O-bound behavior — matching Linux CFS's
                    // place_entity() logic.
                    let floor = MIN_VRUNTIME
                        .load(Ordering::Relaxed)
                        .saturating_sub(SLEEPER_BONUS);
                    let cur_vrt = VRUNTIME[idx].load(Ordering::Relaxed);
                    if cur_vrt < floor {
                        VRUNTIME[idx].store(floor, Ordering::Relaxed);
                    }

                    // Mark running and context-switch.
                    inner.state = ProcState::RUNNING;
                    unsafe {
                        (*c).proc.replace(Arc::clone(p));

                        // Switch to the process.  The process will
                        // eventually call yielding() / sleep() / exit()
                        // which re-acquires this lock and calls swtch()
                        // back here.
                        swtch(&mut (*c).context, &p.data().context);

                        // Back in the scheduler — process is done for now.
                        (*c).proc.take();
                    }

                    // Charge vruntime for the time slice just consumed.
                    // With all tasks at nice 0 the delta simplifies to
                    // SCHED_MIN_GRANULARITY; the generic formula is
                    // retained so that per-process weights work once a
                    // renice syscall is added.
                    let weight = NICE_0_WEIGHT;
                    let delta = calc_delta(SCHED_MIN_GRANULARITY, weight);
                    VRUNTIME[idx].fetch_add(delta, Ordering::Release);
                }
                // `inner` guard drops here, releasing the process lock
                // that was re-acquired by the process's yielding()/sched().
            }
        }
    }
}
