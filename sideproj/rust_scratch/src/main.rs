#![feature(lang_items)]
#![allow(internal_features)]
#![no_std] // don't link the Rust standard library
#![no_main] // disable all Rust-level entry points

use core::panic::PanicInfo;

/// This function is called on panic.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

// #[unsafe(no_mangle)]
// extern "C" fn rust_eh_personality() {}

#[lang = "eh_personality"]
fn rust_eh_personality() {}

#[unsafe(no_mangle)] // don't mangle the name of this function
pub extern "C" fn _start() -> ! {
    loop {}
}
