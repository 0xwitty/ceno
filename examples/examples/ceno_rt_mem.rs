#![no_main]
#![no_std]

// Use volatile functions to prevent compiler optimizations.
use core::ptr::{read_volatile, write_volatile};

extern crate ceno_rt;
const OUTPUT_ADDRESS: u32 = 0x8000_0000;
const HINT_START: u32 = 0x4000_0000;
const HINT_END: u32 = 0x5000_0000;
const HINT: u32 = 0x4000_1000;

ceno_rt::entry!(main);
#[inline(never)]
fn main() {
    test_data_section();

    // let out = fibonacci_recurse(20, 0, 1);
    let out = unsafe {
        let x = HINT as *mut u32;
        read_volatile(x)
    };
    test_output(out + 6765);
}

/// Test the .data section is loaded and read/write works.
#[inline(never)]
fn test_data_section() {
    // Use X[1] to be sure it is not the same as *OUTPUT_ADDRESS.
    static mut X: [u32; 2] = [0, 42];

    unsafe {
        assert_eq!(read_volatile(&X[1]), 42);
        write_volatile(&mut X[1], 99);
        assert_eq!(read_volatile(&X[1]), 99);
    }
}

// A sufficiently complicated function to test the stack.
#[inline(never)]
fn fibonacci_recurse(count: u32, a: u32, b: u32) -> u32 {
    let count = black_box(count);
    if count == 0 {
        a
    } else {
        fibonacci_recurse(count - 1, b, a + b)
    }
}

// Store the output to a specific memory location so the emulator tests can find it.
#[inline(never)]
fn test_output(out: u32) {
    unsafe {
        write_volatile(OUTPUT_ADDRESS as *mut u32, out);
    }
}

fn black_box<T>(x: T) -> T {
    unsafe { read_volatile(&x) }
}
