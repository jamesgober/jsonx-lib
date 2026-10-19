//! The incremental reader, fed the input in chunks sized by the first byte,
//! agrees with the complete reader, invalid UTF-8 included.

#![no_main]

#[path = "../../tests/harness/mod.rs"]
mod harness;

libfuzzer_sys::fuzz_target!(|data: &[u8]| harness::check_stream(data));
