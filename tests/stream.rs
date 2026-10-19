//! The incremental reader against the complete one: fed in any chunks, a
//! `StreamReader` gives the same events, raw slices, values, kinds, and
//! errors at the same offsets as a `Reader` over the whole input, for any
//! mix of calls.

mod common;

use common::strategies::{mutated, soup, value};
use jsonx_lib::{
    DuplicateKeys, Error, ErrorKind, Event, Kind, Limits, Parser, Step, StreamReader, Value,
};
use proptest::{prelude::*, sample::Index};

/// No budgets and no duplicate check: the grammar alone decides.
const GRAMMAR_ONLY: Parser = Parser::new()
    .limits(Limits::unlimited())
    .duplicate_keys(DuplicateKeys::KeepAll);

/// Tight budgets and duplicates refused, to exercise every error path.
const TIGHT: Parser = Parser::new().limits(
    Limits::new()
        .with_max_depth(4)
        .with_max_string_bytes(6)
        .with_max_number_bytes(5)
        .with_max_elements(12),
);

/// What one call returned, detached from the input.
#[derive(Debug, PartialEq)]
enum Out {
    Event(Option<Event<'static>>),
    Raw(Option<String>),
    Value(Option<Value<'static>>),
    Kind(Option<Kind>),
    Err(Error),
}

fn owned(e: Event<'_>) -> Event<'static> {
    match e {
        Event::Key(k) => Event::Key(k.into_owned().into()),
        Event::String(s) => Event::String(s.into_owned().into()),
        Event::Number(n) => Event::Number(n.into_owned()),
        Event::StartObject => Event::StartObject,
        Event::EndObject => Event::EndObject,
        Event::StartArray => Event::StartArray,
        Event::EndArray => Event::EndArray,
        Event::Bool(b) => Event::Bool(b),
        Event::Null => Event::Null,
    }
}

/// Whether a run is over after `out`.
fn ends(out: &Out) -> bool {
    matches!(out, Out::Err(_) | Out::Event(None))
}

/// How many calls a run makes at most: enough to finish any document of
/// `len` bytes whatever the mix of calls.
fn budget(len: usize) -> usize {
    3 * len + 8
}

/// The calls of `ops`, cycled, on a complete reader over `text`.
fn run_reader(parser: &Parser, text: &str, ops: &[u8], calls: usize) -> Vec<Out> {
    let mut r = parser.reader(text);
    let mut outs = Vec::new();
    for i in 0..calls {
        let out = match ops.get(i % ops.len().max(1)).copied().unwrap_or(0) % 4 {
            0 => r.next_event().map(|e| Out::Event(e.map(owned))),
            1 => r.read_raw().map(|s| Out::Raw(s.map(String::from))),
            2 => r.read_value().map(|v| Out::Value(v.map(Value::into_owned))),
            _ => r.peek_kind().map(Out::Kind),
        }
        .unwrap_or_else(Out::Err);
        let done = ends(&out);
        outs.push(out);
        if done {
            break;
        }
    }
    outs
}

/// The same calls on a stream reader fed `bytes` in pieces ending at `cuts`.
fn run_stream(parser: &Parser, bytes: &[u8], cuts: &[usize], ops: &[u8]) -> Vec<Out> {
    let mut r = parser.stream_reader();
    let mut bounds: Vec<usize> = cuts.iter().map(|&c| c.min(bytes.len())).collect();
    bounds.sort_unstable();
    bounds.push(bytes.len());
    let mut chunks = bounds.into_iter();
    let mut fed = 0;
    let mut feed_more = |r: &mut StreamReader| match chunks.next() {
        Some(end) => {
            let piece = bytes.get(fed..end).unwrap_or_default();
            fed = end;
            r.feed(piece).err()
        }
        None => {
            r.end_input();
            None
        }
    };
    let mut outs = Vec::new();
    for i in 0..budget(bytes.len()) {
        let op = ops.get(i % ops.len().max(1)).copied().unwrap_or(0) % 4;
        let out = loop {
            let step = match op {
                0 => r.next_event().map(|s| s.map(|e| Out::Event(e.map(owned)))),
                1 => r
                    .read_raw()
                    .map(|s| s.map(|v| Out::Raw(v.map(String::from)))),
                2 => r
                    .read_value()
                    .map(|s| s.map(|v| Out::Value(v.map(Value::into_owned)))),
                _ => r.peek_kind().map(|s| s.map(Out::Kind)),
            };
            match step {
                Ok(Step::Ready(out)) => break out,
                Ok(Step::NeedInput) => {
                    if let Some(e) = feed_more(&mut r) {
                        break Out::Err(e);
                    }
                }
                Err(e) => break Out::Err(e),
            }
        };
        let done = ends(&out);
        outs.push(out);
        if done {
            break;
        }
    }
    outs
}

/// What the stream must report for bytes that may not be UTF-8: the
/// complete reader over the valid prefix, with "the text ended" read as
/// "the input stopped being UTF-8" where it did.
fn model(parser: &Parser, bytes: &[u8], ops: &[u8]) -> Vec<Out> {
    let valid = match core::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(e) => e.valid_up_to(),
    };
    let text = core::str::from_utf8(bytes.get(..valid).unwrap_or_default()).unwrap_or("");
    let mut outs = run_reader(parser, text, ops, budget(bytes.len()));
    if valid < bytes.len() {
        if let Some(last) = outs.last_mut() {
            let ran_out = match last {
                Out::Err(e) => e.kind() == ErrorKind::UnexpectedEnd && e.offset() == valid,
                Out::Event(None) => true,
                _ => false,
            };
            if ran_out {
                *last = Out::Err(model_utf8_error(valid));
            }
        }
    }
    outs
}

fn model_utf8_error(at: usize) -> Error {
    // Any parse of these bytes reports invalid UTF-8 at `at`.
    let mut bytes = vec![b' '; at];
    bytes.push(0xFF);
    jsonx_lib::parse_bytes(&bytes).unwrap_err()
}

fn cut_points() -> impl Strategy<Value = Vec<usize>> {
    prop::collection::vec(0usize..600, 0..12)
}

fn ops() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![Just(vec![0u8]), prop::collection::vec(any::<u8>(), 1..16),]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// Valid documents, written compact and pretty, in any chunks.
    #[test]
    fn prop_documents_match_the_complete_reader(v in value(), pretty in any::<bool>(), cuts in cut_points(), ops in ops()) {
        let text = if pretty { jsonx_lib::to_string_pretty(&v) } else { jsonx_lib::to_string(&v) };
        for parser in [GRAMMAR_ONLY, TIGHT, Parser::new()] {
            let want = run_reader(&parser, &text, &ops, budget(text.len()));
            prop_assert_eq!(run_stream(&parser, text.as_bytes(), &cuts, &ops), want);
        }
    }

    /// Near-valid and broken input: the same error at the same offset.
    #[test]
    fn prop_soup_and_mutations_match_the_complete_reader(text in prop_oneof![soup(), mutated()], cuts in cut_points(), ops in ops()) {
        for parser in [GRAMMAR_ONLY, TIGHT, Parser::new()] {
            let want = run_reader(&parser, &text, &ops, budget(text.len()));
            prop_assert_eq!(run_stream(&parser, text.as_bytes(), &cuts, &ops), want);
        }
    }

    /// Arbitrary bytes, UTF-8 or not, never panic and follow the model.
    #[test]
    fn prop_arbitrary_bytes_follow_the_model(bytes in prop::collection::vec(any::<u8>(), 0..256), cuts in cut_points(), ops in ops()) {
        for parser in [GRAMMAR_ONLY, TIGHT] {
            prop_assert_eq!(run_stream(&parser, &bytes, &cuts, &ops), model(&parser, &bytes, &ops));
        }
    }

    /// Text with multi-byte characters cut mid-character, plus one byte that
    /// breaks UTF-8 somewhere.
    #[test]
    fn prop_broken_utf8_inside_documents(v in value(), at in any::<Index>(), bad in prop::sample::select(vec![0x80u8, 0xC3, 0xE2, 0xF0, 0xFF]), cuts in cut_points(), ops in ops()) {
        let mut bytes = jsonx_lib::to_string(&v).into_bytes();
        let i = at.index(bytes.len() + 1);
        bytes.insert(i, bad);
        prop_assert_eq!(run_stream(&GRAMMAR_ONLY, &bytes, &cuts, &ops), model(&GRAMMAR_ONLY, &bytes, &ops));
    }
}

#[test]
fn test_stream_one_byte_chunks_cost_linear_time() {
    // Quadratic rescanning of a token cut by every chunk boundary would take
    // minutes here: 4 MiB of string, 4 MiB of digits, 4 MiB of whitespace,
    // each fed a byte at a time.
    let big = 4 << 20;
    let text = format!(
        "[\"{}\", {}{}, {{\"k\"{}: 1}}]",
        "x".repeat(big),
        "1".repeat(big),
        " ".repeat(big),
        " ".repeat(big)
    );
    let mut r = Parser::new().limits(Limits::unlimited()).stream_reader();
    let mut events = 0;
    for byte in text.as_bytes() {
        assert!(r.feed(std::slice::from_ref(byte)).is_ok());
        while let Ok(Step::Ready(Some(_))) = r.next_event() {
            events += 1;
        }
        assert!(r.buffered() < big + 16 * 1024);
    }
    r.end_input();
    assert_eq!(r.next_event().ok(), Some(Step::Ready(None)));
    assert_eq!(events, 8);
}

#[test]
fn test_stream_read_value_duplicate_before_syntax_error_matches_reader() {
    let text = r#"[{"a": 1, "a": 2}, x]"#;
    let mut r = jsonx_lib::Reader::new(text);
    let want = r.read_value().err();
    let mut s = StreamReader::new();
    assert!(s.feed(text.as_bytes()).is_ok());
    assert_eq!(s.read_value().err(), want);
    assert_eq!(
        want.map(|e| (e.kind(), e.offset())),
        Some((ErrorKind::DuplicateKey, 10))
    );
}

#[cfg(feature = "std")]
#[test]
fn test_stream_read_from_small_io_reads_reads_the_whole_value() {
    struct Trickle<'a>(&'a [u8]);
    impl std::io::Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(3).min(self.0.len());
            buf.get_mut(..n)
                .unwrap_or_default()
                .copy_from_slice(self.0.get(..n).unwrap_or_default());
            self.0 = self.0.get(n..).unwrap_or_default();
            Ok(n)
        }
    }
    let text = br#"{"list": [1, 2, {"deep": "value"}], "end": true}"#;
    let mut source = Trickle(text);
    let mut r = StreamReader::new();
    let value = loop {
        // Owned before matching, so the reader is free again in every arm.
        let step = r.read_value().map(|s| s.map(|v| v.map(Value::into_owned)));
        match step {
            Ok(Step::Ready(v)) => break v,
            Ok(Step::NeedInput) => {
                assert!(r.read_from(&mut source).is_ok());
            }
            Err(e) => panic!("{e}"),
        }
    };
    assert_eq!(
        value,
        jsonx_lib::parse_bytes(text).ok().map(Value::into_owned)
    );

    struct Failing;
    impl std::io::Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("gone"))
        }
    }
    assert!(StreamReader::new().read_from(&mut Failing).is_err());

    // A budget refused while reading is reported by the next reading call.
    let mut tiny = Parser::new()
        .limits(Limits::new().with_max_input_bytes(2))
        .stream_reader();
    let mut source: &[u8] = b"[1, 2]";
    assert!(tiny.read_from(&mut source).is_ok());
    assert_eq!(
        tiny.next_event().err().map(|e| e.kind()),
        Some(ErrorKind::InputTooLarge)
    );
}
