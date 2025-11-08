use alloc::sync::Arc;
use kernel::{
    proc::{ProcState, CPUS, PROCS},
    riscv::intr_on,
    swtch::swtch,
};

use super::InstanceScheduler;

pub struct RoundRobin;

impl InstanceScheduler for RoundRobin {
    /// Per-CPU process scheduler.
    /// Each CPU calls scheduler() after setting itself up.
    /// Scheduler never returns. It loops, doing:
    ///  - choose a process to run.
    ///  - swtch to start running thet process.
    ///  - eventually that process transfers control via swtch back to the
    ///    scheduler.
    fn instance_scheduler(&mut self) -> ! {
        let c = unsafe { CPUS.mycpu() };

        loop {
            // Avoid deadlock by ensuring thet devices can interrupt.
            intr_on();

            for p in PROCS.pool.iter() {
                let mut inner = p.inner.lock();
                if inner.state == ProcState::RUNNABLE {
                    // Switch to chosen process. It is the process's job
                    // to release its lock and then reacquire it
                    // before jumping back to us.
                    inner.state = ProcState::RUNNING;
                    unsafe {
                        (*c).proc.replace(Arc::clone(p));
                        swtch(&mut (*c).context, &p.data().context);
                        // Process is done running for now.
                        // It should have changed its p->state before coming back.
                        (*c).proc.take();
                    }
                }
            }
        }
    }
}

impl Default for RoundRobin {
    fn default() -> Self {
        Self {}
    }
}
