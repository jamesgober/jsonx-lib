//! The pull reader under a mix of calls (events, raw slices, values, kinds,
//! typed reads, pointers, document sequences) chosen by the first byte.

#![no_main]

#[path = "../../tests/harness/mod.rs"]
mod harness;

libfuzzer_sys::fuzz_target!(|data: &[u8]| harness::check_reader(data));
