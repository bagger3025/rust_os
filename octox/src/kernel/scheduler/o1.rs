// ============================================================================
// O(1) Scheduler
// ============================================================================
//
// Modeled after the Linux 2.6 O(1) scheduler designed by Ingo Molnar (used
// from kernel 2.6.0 to 2.6.22, before CFS replaced it).
//
// Core mechanism — active/expired arrays:
//
//   Every process belongs to one of two sets: active or expired.
//   The scheduler always picks from the active set.  Each process has a
//   priority-based time slice; once exhausted, it moves to expired.
//   When the active set is empty, the two sets are swapped — this begins
//   a new **epoch**.  The swap is O(1): just flip a flag and recalculate
//   time slices.
//
//   In Linux, each set was a 140-entry priority bitmap + per-level linked
//   list, giving O(1) lookup via `find_first_bit()`.  With Octox's fixed
//   process table, a linear scan is simpler, but the
//   active/expired epoch mechanism is faithfully preserved.
//
// Priority and time slices:
//
//   40 priority levels (0–39, mapping to nice −20 to +19).  All
//   processes start at priority 20 (nice 0).  Higher priority (lower
//   number) -> longer time slice:
//
//       timeslice = 1 + 5 x (39 − priority) / 39     [range 1–6 ticks]
//
// Interactivity heuristic:
//
//   The original O(1) scheduler tracked a "sleep average" — processes
//   that frequently slept (I/O-bound) got a priority bonus, while
//   CPU-bound processes stayed at base priority.
//
//   We approximate this at epoch boundaries: if a process used its full
//   time slice (expired), it is CPU-bound -> decrease sleep_avg.  If it
//   still had time slice remaining (was sleeping or waiting), it is
//   I/O-bound -> increase sleep_avg.  The sleep_avg (range 0–5) maps
//   directly to a priority bonus.
//
// SMP safety:
//
//   Per-process metadata uses atomics (same pattern as CFS/MLFQ).
//   The epoch swap uses compare_exchange on a global counter so exactly
//   one CPU performs the swap.  Two-phase scan handles races where
//   another CPU grabs the chosen process between phases.
//
// ============================================================================

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
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

/// Number of distinct priority levels (mirrors Linux nice −20..+19).
const NUM_PRIORITIES: usize = 40;

/// Default priority for all processes (nice 0 = index 20).
const BASE_PRIORITY: u8 = 20;

/// Shortest time slice (ticks), assigned to the lowest priority.
const MIN_TIMESLICE: u64 = 1;

/// Longest time slice (ticks), assigned to the highest priority.
const MAX_TIMESLICE: u64 = 6;

/// Maximum sleep average value (I/O-bound ceiling).
const MAX_SLEEP_AVG: u64 = 5;

/// Maximum priority bonus derived from sleep average.
/// A fully I/O-bound process (sleep_avg = MAX_SLEEP_AVG) gets its priority
/// lowered (improved) by this many levels.
const MAX_BONUS: u8 = 5;

// ---------------------------------------------------------------------------
// Per-process scheduler state (indexed by PROCS.pool position)
// ---------------------------------------------------------------------------

const PRIO_INIT: AtomicU8 = AtomicU8::new(BASE_PRIORITY);
const U64_ZERO: AtomicU64 = AtomicU64::new(0);
const BOOL_FALSE: AtomicBool = AtomicBool::new(false);

/// Dynamic priority (0 = highest, 39 = lowest).
static DYN_PRIORITY: [AtomicU8; NPROC] = [PRIO_INIT; NPROC];

/// Remaining time slice in the current epoch (ticks).
/// Initialized to 0; assigned on first dispatch via `calc_timeslice`.
static TIMESLICE: [AtomicU64; NPROC] = [U64_ZERO; NPROC];

/// Interactivity estimator.  0 = CPU-bound, MAX_SLEEP_AVG = I/O-bound.
/// Updated at epoch boundaries based on whether the process expired.
static SLEEP_AVG: [AtomicU64; NPROC] = [U64_ZERO; NPROC];

/// `true` if the process has exhausted its time slice and is in the
/// "expired" array, waiting for the next epoch.
static IS_EXPIRED: [AtomicBool; NPROC] = [BOOL_FALSE; NPROC];

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Monotonically increasing epoch counter.  Incremented by exactly one CPU
/// when it detects that all active processes are exhausted.
static EPOCH: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute dynamic priority from sleep average.
/// Higher sleep_avg -> lower priority number -> higher actual priority.
#[inline]
fn calc_priority(sleep_avg: u64) -> u8 {
    let bonus = (sleep_avg as u8).min(MAX_BONUS);
    BASE_PRIORITY.saturating_sub(bonus)
}

/// Compute time slice from priority.
/// Higher priority (lower number) -> longer time slice.
#[inline]
fn calc_timeslice(prio: u8) -> u64 {
    let p = prio.min((NUM_PRIORITIES - 1) as u8) as u64;
    let max_p = (NUM_PRIORITIES - 1) as u64;
    MIN_TIMESLICE + (MAX_TIMESLICE - MIN_TIMESLICE) * (max_p - p) / max_p
}

/// Perform an epoch swap: recalculate sleep averages, priorities, and
/// time slices for all process slots, then clear expired flags.
fn epoch_swap() {
    for i in 0..NPROC {
        let was_expired = IS_EXPIRED[i].load(Ordering::Relaxed);
        let sa = SLEEP_AVG[i].load(Ordering::Relaxed);

        let new_sa = if was_expired {
            sa.saturating_sub(1)
        } else {
            (sa + 1).min(MAX_SLEEP_AVG)
        };
        SLEEP_AVG[i].store(new_sa, Ordering::Relaxed);

        // Recalculate priority and assign fresh time slice.
        let prio = calc_priority(new_sa);
        DYN_PRIORITY[i].store(prio, Ordering::Relaxed);
        TIMESLICE[i].store(calc_timeslice(prio), Ordering::Relaxed);

        // All processes move to the active set.
        IS_EXPIRED[i].store(false, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Scheduler implementation
// ---------------------------------------------------------------------------

pub struct O1 {
    /// Per-CPU scan offset for round-robin within the same priority level.
    scan_offset: usize,
}

impl Default for O1 {
    fn default() -> Self {
        Self { scan_offset: 0 }
    }
}

impl InstanceScheduler for O1 {
    fn instance_scheduler(&mut self) -> ! {
        let c = unsafe { CPUS.mycpu() };

        loop {
            // Avoid deadlock by ensuring devices can interrupt.
            intr_on();

            // Phase 1: find best active RUNNABLE process
            let mut best_idx: Option<usize> = None;
            let mut best_prio: u8 = u8::MAX;
            let mut any_active_runnable = false;
            let mut any_active_running = false;
            let mut any_expired = false;

            for j in 0..NPROC {
                let i = (self.scan_offset + j) % NPROC;
                let p = &PROCS.pool[i];
                let inner = p.inner.lock();
                let expired = IS_EXPIRED[i].load(Ordering::Relaxed);

                match inner.state {
                    ProcState::RUNNABLE => {
                        if expired {
                            any_expired = true;
                        } else {
                            any_active_runnable = true;
                            let prio = DYN_PRIORITY[i].load(Ordering::Relaxed);
                            if prio < best_prio {
                                best_prio = prio;
                                best_idx = Some(i);
                            }
                        }
                    }
                    ProcState::RUNNING => {
                        // Running on another CPU — counts as active if not
                        // expired, preventing a premature epoch swap.
                        if !expired {
                            any_active_running = true;
                        }
                    }
                    _ => {}
                }
                // Lock dropped here.
            }

            if !any_active_runnable {
                if any_expired && !any_active_running {
                    let cur = EPOCH.load(Ordering::Acquire);
                    if EPOCH
                        .compare_exchange(cur, cur + 1, Ordering::AcqRel, Ordering::Relaxed)
                        .is_ok()
                    {
                        epoch_swap();
                    }
                }
                continue;
            }

            // Phase 2: dispatch the chosen process
            if let Some(idx) = best_idx {
                let p = &PROCS.pool[idx];
                let mut inner = p.inner.lock();

                // Re-check: another CPU may have taken it, or it may have
                // been expired by a concurrent epoch swap race.
                if inner.state == ProcState::RUNNABLE
                    && !IS_EXPIRED[idx].load(Ordering::Relaxed)
                {
                    // Assign initial time slice for new/reused slots.
                    // TIMESLICE starts at 0; a non-expired process with 0
                    // remaining is either brand new or in a reused slot.
                    if TIMESLICE[idx].load(Ordering::Relaxed) == 0 {
                        let prio = DYN_PRIORITY[idx].load(Ordering::Relaxed);
                        TIMESLICE[idx].store(calc_timeslice(prio), Ordering::Relaxed);
                    }

                    // Context-switch into the process.
                    inner.state = ProcState::RUNNING;
                    unsafe {
                        (*c).proc.replace(Arc::clone(p));
                        swtch(&mut (*c).context, &p.data().context);
                        (*c).proc.take();
                    }

                    let remaining = TIMESLICE[idx].load(Ordering::Relaxed);
                    if remaining <= 1 {
                        // Time slice exhausted → move to expired array.
                        TIMESLICE[idx].store(0, Ordering::Relaxed);
                        IS_EXPIRED[idx].store(true, Ordering::Relaxed);
                    } else {
                        TIMESLICE[idx].store(remaining - 1, Ordering::Relaxed);
                    }
                }

                self.scan_offset = (idx + 1) % NPROC;
            }
        }
    }
}
