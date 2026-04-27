// ============================================================================
// MLFQ — Multi-Level Feedback Queue Scheduler
// ============================================================================
//
// Implements the classic MLFQ algorithm described in OSTEP (Operating Systems:
// Three Easy Pieces, Chapter 8).  Adapted for fixed process table
// (NPROC=64), SMP (4 CPUs), and RISC-V timer-driven preemption.
//
// Core rules (from OSTEP):
//
//   Rule 1: If Priority(A) > Priority(B), A runs (B doesn't).
//   Rule 2: If Priority(A) == Priority(B), A & B run in round-robin.
//   Rule 3: When a job enters the system, it starts at the highest priority.
//   Rule 4: Once a job uses up its time allotment at a given level
//           (regardless of how many times it has given up the CPU),
//           its priority is reduced (i.e., it moves down one queue).
//   Rule 5: After some time period S, move all jobs to the topmost queue
//           (priority boost — prevents starvation).
//
// Design choices:
//
//   - 3 priority levels: 0 (highest), 1, 2 (lowest).
//   - Time allotments: level 0 = 1 round, level 1 = 2 rounds, level 2 = 4.
//     Higher-priority queues get shorter allotments, so CPU-bound processes
//     are identified and demoted quickly.
//   - Priority boost every BOOST_INTERVAL dispatches (~100 context switches
//     across all CPUs).  Exactly one CPU triggers the boost via fetch_add.
//   - Per-CPU scan offset provides round-robin within each priority level.
//   - Two-phase scan (like CFS): Phase 1 finds the best candidate across
//     all process slots; Phase 2 re-locks and verifies before dispatch.
//     This handles the SMP race where another CPU grabs our candidate.
//   - Accounting-based demotion (Rule 4 with gaming prevention): we track
//     cumulative ticks at each level, so a process that repeatedly sleeps
//     just before its allotment expires will still be demoted eventually.
//
// ============================================================================

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};
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

/// Number of priority levels.  Level 0 is highest priority.
const NUM_LEVELS: usize = 3;

/// Time allotment (in scheduling rounds) before demotion at each level.
/// Once a process accumulates this many rounds at a level, it is demoted
/// to the next lower level.  Level 2 (lowest) has no demotion.
const TIME_ALLOTMENT: [u64; NUM_LEVELS] = [1, 2, 4];

/// Number of global dispatches between priority boosts.
/// When a boost fires, ALL processes are moved back to level 0 (Rule 5).
/// With 4 CPUs and typical workloads this fires roughly every few hundred
/// milliseconds — frequent enough to prevent starvation, infrequent enough
/// to let the feedback mechanism work.
const BOOST_INTERVAL: u64 = 100;

// ---------------------------------------------------------------------------
// Global MLFQ state (shared across all CPUs)
// ---------------------------------------------------------------------------

const LEVEL_ZERO: AtomicU8 = AtomicU8::new(0);
const TICKS_ZERO: AtomicU64 = AtomicU64::new(0);

/// Per-process priority level (0 = highest, NUM_LEVELS-1 = lowest).
/// Indexed by position in PROCS.pool.
static QUEUE_LEVEL: [AtomicU8; NPROC] = [LEVEL_ZERO; NPROC];

/// Per-process cumulative ticks consumed at the current priority level.
/// Reset to 0 on demotion or priority boost.
static TICKS_AT_LEVEL: [AtomicU64; NPROC] = [TICKS_ZERO; NPROC];

/// Global dispatch counter.  Incremented by each CPU after every successful
/// context switch.  Used to trigger periodic priority boosts — exactly one
/// CPU will see `(old_count + 1) % BOOST_INTERVAL == 0` and perform the
/// boost.
static DISPATCH_COUNT: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Scheduler implementation
// ---------------------------------------------------------------------------

pub struct Mlfq {
    /// Per-CPU scan offset for round-robin within priority levels.
    /// Each CPU starts scanning from a different position to reduce
    /// contention and distribute work fairly.
    scan_offset: usize,
}

impl Default for Mlfq {
    fn default() -> Self {
        Self { scan_offset: 0 }
    }
}

impl InstanceScheduler for Mlfq {
    /// Per-CPU MLFQ scheduling loop.  Never returns.
    ///
    /// Each iteration performs two phases:
    ///
    ///   Phase 1 (selection) — scan every slot in the process table
    ///   starting from `scan_offset`, locking each briefly to read its
    ///   state.  Select the RUNNABLE process at the highest priority level
    ///   (lowest level number).  Within the same level, the scan order
    ///   provides round-robin fairness.
    ///
    ///   Phase 2 (dispatch) — re-lock the chosen process, verify it
    ///   is still RUNNABLE (another CPU may have grabbed it), context-
    ///   switch into it, and on return charge time and check demotion.
    ///
    ///   Boost check — after dispatch, if the global dispatch count
    ///   crosses a BOOST_INTERVAL boundary, reset all processes to level 0.
    fn instance_scheduler(&mut self) -> ! {
        let c = unsafe { CPUS.mycpu() };

        loop {
            // Avoid deadlock by ensuring devices can interrupt.
            intr_on();

            // Phase 1: find highest-priority RUNNABLE process
            let best_idx = {
                let mut best_idx: Option<usize> = None;
                let mut best_level: u8 = u8::MAX;
                
                for j in 0..NPROC {
                    let i = (self.scan_offset + j) % NPROC;
                    let p = &PROCS.pool[i];
                    let inner = p.inner.lock();
                    
                    if inner.state == ProcState::RUNNABLE {
                        let level = QUEUE_LEVEL[i].load(Ordering::Relaxed);
                        // Pick the process at the highest priority (lowest level
                        // number).  Within the same level, the first one we
                        // encounter wins (round-robin via scan_offset).
                        if level < best_level {
                            best_level = level;
                            best_idx = Some(i);
                        }
                    }
                    // Lock dropped here — brief window before Phase 2.
                }

                best_idx
            };
                
            // Phase 2: dispatch the chosen process
            if let Some(idx) = best_idx {
                let p = &PROCS.pool[idx];
                let mut inner = p.inner.lock();

                // Re-check: another CPU may have taken it between phases.
                if inner.state == ProcState::RUNNABLE {
                    // Mark running and context-switch.
                    inner.state = ProcState::RUNNING;
                    unsafe {
                        (*c).proc.replace(Arc::clone(p));
                        swtch(&mut (*c).context, &p.data().context);
                        // Process yielded back to us.
                        (*c).proc.take();
                    }

                    // Charge time and check demotion (Rule 4)
                    let level = QUEUE_LEVEL[idx].load(Ordering::Relaxed) as usize;
                    let ticks = TICKS_AT_LEVEL[idx].fetch_add(1, Ordering::Relaxed) + 1;

                    if level < NUM_LEVELS - 1 && ticks >= TIME_ALLOTMENT[level] {
                        // Demote to next lower priority level.
                        QUEUE_LEVEL[idx].store((level + 1) as u8, Ordering::Relaxed);
                        TICKS_AT_LEVEL[idx].store(0, Ordering::Relaxed);
                    }

                    // Priority boost check (Rule 5)
                    let count = DISPATCH_COUNT.fetch_add(1, Ordering::Relaxed);
                    if (count + 1) % BOOST_INTERVAL == 0 {
                        // Exactly one CPU hits this for each interval.
                        // Move all processes back to highest priority.
                        for i in 0..NPROC {
                            QUEUE_LEVEL[i].store(0, Ordering::Relaxed);
                            TICKS_AT_LEVEL[i].store(0, Ordering::Relaxed);
                        }
                    }
                }

                // Advance scan offset for round-robin within levels.
                self.scan_offset = (idx + 1) % NPROC;
            }
        }
    }
}
