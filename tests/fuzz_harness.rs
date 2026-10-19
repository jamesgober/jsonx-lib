//! The fuzz targets' checks (`tests/harness`), run in plain `cargo test` on
//! arbitrary bytes and on JSON-like input, so they hold on every platform,
//! including Windows where libFuzzer does not run.

mod harness;

use harness::common::strategies;

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    #[test]
    fn prop_arbitrary_bytes_pass_every_check(data in prop::collection::vec(any::<u8>(), 0..160)) {
        harness::check_all(&data);
    }

    #[test]
    fn prop_json_like_input_passes_every_check(text in prop_oneof![strategies::soup(), strategies::mutated()], ops in any::<u8>()) {
        harness::check_all(text.as_bytes());
        let mut with_op = vec![ops];
        with_op.extend_from_slice(text.as_bytes());
        harness::check_reader(&with_op);
        harness::check_stream(&with_op);
    }

    #[test]
    fn prop_valid_documents_pass_every_check(v in strategies::value(), chunk in any::<u8>()) {
        let text = jsonx_lib::to_string_pretty(&v);
        harness::check_all(text.as_bytes());
        let mut with_chunk = vec![chunk];
        with_chunk.extend_from_slice(text.as_bytes());
        harness::check_stream(&with_chunk);
    }
}

#[test]
fn test_harness_seeds_pass() {
    for seed in [
        &b""[..],
        b"null",
        b"{\"a\":[1,2,{\"b\":null}],\"a\":3}",
        b"[1,]",
        b"\"\\ud800\"",
        b"[\"\xC3\x28\"]",
        b"  {\"k\": \"v\"}  \n {\"k\": 2}",
    ] {
        harness::check_all(seed);
    }
}
