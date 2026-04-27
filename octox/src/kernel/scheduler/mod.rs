#[cfg(not(any(
    sched_rr,
    sched_mlfq,
    sched_o1,
    sched_eevdf,
    sched_drl,
    sched_hybrid_drl
)))]
pub mod cfs;
#[cfg(sched_drl)]
pub mod drl;
#[cfg(sched_eevdf)]
pub mod eevdf;
#[cfg(sched_hybrid_drl)]
pub mod hybrid_drl;
#[cfg(sched_mlfq)]
pub mod mlfq;
#[cfg(sched_o1)]
pub mod o1;
#[cfg(sched_rr)]
pub mod round_robin;

trait InstanceScheduler {
    fn instance_scheduler(&mut self) -> !;
}

pub trait Scheduler {
    fn scheduler() -> !;
}

impl<T> Scheduler for T
where
    T: Default + InstanceScheduler,
{
    fn scheduler() -> ! {
        let mut schdler = T::default();
        schdler.instance_scheduler();
    }
}
