//! Every parse entry point, every duplicate-key policy and budget set, the
//! writers in every format, and the reference parser, agree on the input.

#![no_main]

#[path = "../../tests/harness/mod.rs"]
mod harness;

libfuzzer_sys::fuzz_target!(|data: &[u8]| harness::check_parse(data));
