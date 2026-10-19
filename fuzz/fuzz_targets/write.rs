//! A valid input transcoded event by event through the push writer is what
//! the value writers write.

#![no_main]

#[path = "../../tests/harness/mod.rs"]
mod harness;

libfuzzer_sys::fuzz_target!(|data: &[u8]| harness::check_write(data));
