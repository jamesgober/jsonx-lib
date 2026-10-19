//! Typed reads (`FromJson`) of the input: values or errors, never a panic,
//! and agreement with the tree where they overlap.

#![no_main]

#[path = "../../tests/harness/mod.rs"]
mod harness;

libfuzzer_sys::fuzz_target!(|data: &[u8]| harness::check_decode(data));
