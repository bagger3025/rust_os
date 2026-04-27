#![no_std]
#![no_main]

mod scheduler;

extern crate alloc;

use core::sync::atomic::{AtomicBool, Ordering};
use kernel::{
    bio, console, kalloc, kmain, null, plic, println,
    proc::{self, user_init, Cpus},
    trap, virtio_disk, vm,
};
#[cfg(sched_rr)]
use scheduler::round_robin::RoundRobin as ActiveScheduler;
#[cfg(sched_mlfq)]
use scheduler::mlfq::Mlfq as ActiveScheduler;
#[cfg(sched_o1)]
use scheduler::o1::O1 as ActiveScheduler;
#[cfg(sched_eevdf)]
use scheduler::eevdf::Eevdf as ActiveScheduler;
#[cfg(sched_drl)]
use scheduler::drl::Drl as ActiveScheduler;
#[cfg(sched_hybrid_drl)]
use scheduler::hybrid_drl::HybridDrl as ActiveScheduler;
#[cfg(not(any(
    sched_rr,
    sched_mlfq,
    sched_o1,
    sched_eevdf,
    sched_drl,
    sched_hybrid_drl
)))]
use scheduler::cfs::Cfs as ActiveScheduler;

use crate::scheduler::Scheduler;

static STARTED: AtomicBool = AtomicBool::new(false);

kmain!(main);

extern "C" fn main() -> ! {
    let cpuid = unsafe { Cpus::cpu_id() };
    if cpuid == 0 {
        let initcode = include_bytes!(concat!(env!("OUT_DIR"), "/bin/_initcode"));
        console::init(); // console init
        println!("");
        println!("octox kernel is booting");
        println!("");
        null::init(); // null device init
        kalloc::init(); // physical memory allocator
        vm::kinit(); // create kernel page table
        vm::kinithart(); // turn on paging
        proc::init(); // process table
        trap::inithart(); // install kernel trap vector
        plic::init(); // set up interrupt controller
        plic::inithart(); // ask PLIC for device interrupts
        bio::init(); // buffer cache
        virtio_disk::init(); // emulated hard disk
        user_init(initcode);
        STARTED.store(true, Ordering::SeqCst);
    } else {
        while !STARTED.load(Ordering::SeqCst) {
            core::hint::spin_loop()
        }
        println!("hart {} starting", unsafe { Cpus::cpu_id() });
        vm::kinithart(); // turn on paging
        trap::inithart(); // install kernel trap vector
        plic::inithart(); // ask PLIC for device interrupts
    }
    ActiveScheduler::scheduler();
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    use kernel::printf::panic_inner;
    panic_inner(info)
}
