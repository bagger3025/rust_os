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
