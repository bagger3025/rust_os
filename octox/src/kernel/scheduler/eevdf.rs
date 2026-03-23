// ============================================================================
// EEVDF — Earliest Eligible Virtual Deadline First Scheduler
// ============================================================================
//
// Implements the EEVDF algorithm by Ion Stoica and Hussein Abdel-Wahab (1995),
// which replaced CFS in Linux 6.6 (2023, by Peter Zijlstra).
//
// EEVDF builds on CFS's virtual runtime concept but adds two key mechanisms:
//
//   1. Virtual deadlines — each process has a deadline computed as:
//
//          deadline = vruntime + slice_length / weight
//
//      Processes requesting shorter slices get earlier deadlines and are
//      scheduled sooner. This naturally benefits latency-sensitive (I/O-bound)
//      workloads without requiring CFS's ad-hoc "sleeper bonus" heuristic.
//
//   2. Eligibility — a process is "eligible" to run only if it has not
//      received more than its fair share of CPU time:
//
//          eligible ==  vruntime <= avg_vruntime
//
//      This prevents a waking process from monopolising the CPU even if its
//      vruntime is far behind the pack. The avg_vruntime is the arithmetic
//      mean of all active (RUNNABLE + RUNNING) tasks' vruntimes.
//
// Scheduling rule:  Among all eligible RUNNABLE processes, pick the one
//                   with the earliest virtual deadline.
//
// Compared to CFS:
//   - CFS picks the task with the lowest vruntime (most "starved").
//   - EEVDF picks the eligible task closest to missing its deadline.
//   - EEVDF provides bounded lag guarantees (each task completes its fair
//     share within a known virtual-time window).
//   - No sleeper bonus hack — eligibility and deadlines handle wakeup
//     latency naturally.
//
// Adapted for fixed process table and SMP (4 CPUs).
// With 16 processes a two-pass linear scan replaces the red-black tree
// augmented with avg_vruntime that Linux uses.
//
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

/// Default weight corresponding to nice 0 (matches Linux NICE_0_LOAD).
const NICE_0_WEIGHT: u64 = 1024;

/// Size of a scheduling slice in virtual-time units.
/// With all tasks at nice 0, this equals 3 ticks (~30 ms at the default
/// RISC-V QEMU timer rate). After consuming this much vruntime a task's
/// deadline is renewed.
///
/// In Linux 6.6 the default base slice is 3 ms; we use a coarser unit
/// matching Octox's timer granularity.
const SCHED_SLICE: u64 = 3;

/// Minimum vruntime charge per scheduling round (one context-switch).
const SCHED_MIN_GRANULARITY: u64 = 1;

/// Maximum vruntime credit for waking tasks.
/// A task's vruntime is floored at (min_vruntime − WAKEUP_LAG_LIMIT)
/// so that tasks returning from long sleeps do not accumulate unbounded
/// lag that would let them monopolise the CPU.
///
/// Unlike CFS's sleeper bonus, this is not a reward but a clamp on
/// how far behind a task can be.
const WAKEUP_LAG_LIMIT: u64 = SCHED_SLICE;

// ---------------------------------------------------------------------------
// Per-process EEVDF state (indexed by PROCS.pool position)
// ---------------------------------------------------------------------------

const VRT_ZERO: AtomicU64 = AtomicU64::new(0);

/// Virtual runtime consumed by each process.
static VRUNTIME: [AtomicU64; NPROC] = [VRT_ZERO; NPROC];

/// Virtual deadline for each process.
/// deadline = vruntime_at_slice_start + slice_length.
/// Initialised to 0; assigned on first dispatch.
static DEADLINE: [AtomicU64; NPROC] = [VRT_ZERO; NPROC];

// ---------------------------------------------------------------------------
// Global shared state
// ---------------------------------------------------------------------------

/// Monotonically increasing vruntime floor across all active tasks.
/// Used to clamp stale vruntimes on wakeup / fork.
static MIN_VRUNTIME: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute the vruntime delta for one scheduling round:
///     delta_vruntime = delta_exec x (NICE_0_WEIGHT / weight)
#[inline]
fn calc_delta(delta_exec: u64, weight: u64) -> u64 {
    delta_exec * NICE_0_WEIGHT / weight.max(1)
}

/// Compute the virtual-time length of one scheduling slice:
///     slice_vruntime = SCHED_SLICE x (NICE_0_WEIGHT / weight)
/// Higher weight -> shorter virtual slice -> earlier deadlines -> more frequent
/// scheduling.
#[inline]
fn calc_slice_vruntime(weight: u64) -> u64 {
    SCHED_SLICE * NICE_0_WEIGHT / weight.max(1)
}

/// Advance MIN_VRUNTIME to `candidate` if it is larger (never goes backward).
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

pub struct Eevdf;

impl Default for Eevdf {
    fn default() -> Self {
        Self
    }
}

impl InstanceScheduler for Eevdf {
    fn instance_scheduler(&mut self) -> ! {
        let c = unsafe { CPUS.mycpu() };

        loop {
            // Avoid deadlock by ensuring devices can interrupt.
            intr_on();

            // Pass 1: compute avg_vruntime and floor
            let mut sum_vrt: u64 = 0;
            let mut count: u64 = 0;
            let mut active_min_vrt: u64 = u64::MAX;

            for (i, p) in PROCS.pool.iter().enumerate() {
                let inner = p.inner.lock();
                match inner.state {
                    ProcState::RUNNABLE | ProcState::RUNNING => {
                        let vrt = VRUNTIME[i].load(Ordering::Relaxed);
                        sum_vrt = sum_vrt.saturating_add(vrt);
                        count += 1;
                        if vrt < active_min_vrt {
                            active_min_vrt = vrt;
                        }
                    }
                    _ => {}
                }
                // Lock dropped here.
            }

            // Advance global vruntime floor (never backward).
            if active_min_vrt < u64::MAX {
                advance_min_vruntime(active_min_vrt);
            }

            // Average vruntime across all active tasks.
            let avg_vrt = if count > 0 { sum_vrt / count } else { 0 };

            // Pass 2: find eligible RUNNABLE with earliest deadline
            let mut best_idx: Option<usize> = None;
            let mut best_dl: u64 = u64::MAX;
            let mut fallback_idx: Option<usize> = None;
            let mut fallback_vrt: u64 = u64::MAX;

            for (i, p) in PROCS.pool.iter().enumerate() {
                let inner = p.inner.lock();
                if inner.state == ProcState::RUNNABLE {
                    let vrt = VRUNTIME[i].load(Ordering::Relaxed);
                    let dl = DEADLINE[i].load(Ordering::Relaxed);

                    // Eligibility: task has not received more than its share.
                    if vrt <= avg_vrt {
                        if dl < best_dl {
                            best_dl = dl;
                            best_idx = Some(i);
                        }
                    }

                    if vrt < fallback_vrt {
                        fallback_vrt = vrt;
                        fallback_idx = Some(i);
                    }
                }
                // Lock dropped here.
            }

            let chosen = best_idx.or(fallback_idx);

            // Phase 3: dispatch the chosen process
            if let Some(idx) = chosen {
                let p = &PROCS.pool[idx];
                let mut inner = p.inner.lock();

                // Re-check: another CPU may have taken it between passes.
                if inner.state == ProcState::RUNNABLE {
                    let floor = MIN_VRUNTIME
                        .load(Ordering::Relaxed)
                        .saturating_sub(WAKEUP_LAG_LIMIT);
                    let cur_vrt = VRUNTIME[idx].load(Ordering::Relaxed);
                    if cur_vrt < floor {
                        VRUNTIME[idx].store(floor, Ordering::Relaxed);
                    }

                    let vrt = VRUNTIME[idx].load(Ordering::Relaxed);
                    let dl = DEADLINE[idx].load(Ordering::Relaxed);
                    if dl <= vrt {
                        let weight = NICE_0_WEIGHT;
                        DEADLINE[idx]
                            .store(vrt + calc_slice_vruntime(weight), Ordering::Relaxed);
                    }

                    // Context-switch into the process.
                    inner.state = ProcState::RUNNING;
                    unsafe {
                        (*c).proc.replace(Arc::clone(p));
                        swtch(&mut (*c).context, &p.data().context);
                        (*c).proc.take();
                    }

                    let weight = NICE_0_WEIGHT;
                    let delta = calc_delta(SCHED_MIN_GRANULARITY, weight);
                    let new_vrt =
                        VRUNTIME[idx].fetch_add(delta, Ordering::Release) + delta;

                    let dl = DEADLINE[idx].load(Ordering::Relaxed);
                    if new_vrt >= dl {
                        DEADLINE[idx]
                            .store(new_vrt + calc_slice_vruntime(weight), Ordering::Relaxed);
                    }
                }
            }
        }
    }
}
