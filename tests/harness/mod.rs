//! The checks behind the fuzz targets in `fuzz/` and the always-on
//! `tests/fuzz_harness.rs`: given arbitrary bytes, every entry point either
//! succeeds or returns an error, never panics, and every way of reading or
//! writing agrees with the others and with the independent reference parser
//! in `tests/common`.
//!
//! Each check panics (through `assert!`) on a violation, which is what the
//! fuzzer and the test harness report.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

#[path = "../common/mod.rs"]
pub mod common;

use std::collections::BTreeMap;

use common::{Fail, to_ref};
use jsonx_lib::{
    DuplicateKeys, ErrorKind, Event, Format, Limits, Nullable, Parser, RawValue, Step, Value,
    Writer,
};

/// The grammar alone: no budgets, every member kept.
const GRAMMAR_ONLY: Parser = Parser::new()
    .limits(Limits::unlimited())
    .duplicate_keys(DuplicateKeys::KeepAll);

/// Budgets small enough to be hit by short inputs.
const TIGHT: Parser = Parser::new().limits(
    Limits::new()
        .with_max_depth(4)
        .with_max_string_bytes(8)
        .with_max_number_bytes(6)
        .with_max_elements(24)
        .with_max_memory(2048),
);

fn policies() -> [Parser; 4] {
    [
        Parser::new(),
        Parser::new().duplicate_keys(DuplicateKeys::First),
        Parser::new().duplicate_keys(DuplicateKeys::Last),
        GRAMMAR_ONLY,
    ]
}

/// Parses `data` every way there is and checks that the answers agree.
pub fn check_parse(data: &[u8]) {
    for parser in policies().into_iter().chain([TIGHT]) {
        let parsed = parser.parse_bytes(data);
        // The event reader reaches the same verdict.
        if let Ok(mut reader) = parser.reader_bytes(data) {
            let mut events = Ok(());
            loop {
                match reader.next_event() {
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(e) => {
                        events = Err(e);
                        break;
                    }
                }
            }
            match &parsed {
                // The memory budget applies to the tree only: the events may
                // go on past where the tree stopped.
                Err(e) if e.kind() == ErrorKind::MemoryLimitExceeded => {
                    assert!(
                        events
                            .err()
                            .is_none_or(|later| later.offset() >= e.offset())
                    );
                }
                other => assert_eq!(events.err(), other.as_ref().err().copied()),
            }
            // A raw read of the whole document is its trimmed text.
            if parsed.is_ok() {
                let text = core::str::from_utf8(data).unwrap();
                let raw = parser.reader(text).read_raw().unwrap();
                assert_eq!(raw, Some(text.trim_matches([' ', '\t', '\n', '\r'])));
            }
        }
        if let Ok(v) = parsed {
            check_value(&v);
        }
    }
    // The independent reference parser agrees on the verdict, the value, and
    // the error offset.
    if let Ok(text) = core::str::from_utf8(data) {
        let ours = GRAMMAR_ONLY.parse(text);
        let reference = common::parse(data);
        match &ours {
            Ok(v) => assert_eq!(reference.result.as_ref().ok(), Some(&to_ref(v))),
            Err(e) => match e.kind() {
                ErrorKind::LoneSurrogate => assert_eq!(reference.lone_surrogate, Some(e.offset())),
                ErrorKind::UnexpectedEnd => assert_eq!(reference.result.err(), Some(Fail::Eof)),
                _ => assert_eq!(reference.result.err(), Some(Fail::Dead(e.offset()))),
            },
        }
    }
}

/// A parsed value survives every writer and every copy unchanged.
fn check_value(v: &Value<'_>) {
    for format in [
        Format::compact(),
        Format::pretty(),
        Format::compact().ascii(true).html_safe(true),
        Format::compact().with_indent(0),
    ] {
        let text = jsonx_lib::to_string_with(v, format);
        assert_eq!(GRAMMAR_ONLY.parse(&text).as_ref(), Ok(v), "{text}");
    }
    let compact = jsonx_lib::to_string(v);
    assert_eq!(format!("{v}"), compact);
    let copy = v.clone();
    assert_eq!(&copy, v);
    assert_eq!(copy.into_owned(), *v);
    #[cfg(feature = "std")]
    {
        let mut io = Vec::new();
        jsonx_lib::to_writer(&mut io, v).unwrap();
        assert_eq!(io, compact.as_bytes());
    }
    use std::hash::{BuildHasher, RandomState};
    let state = RandomState::new();
    let back = GRAMMAR_ONLY.parse(&compact).unwrap();
    assert_eq!(state.hash_one(&back), state.hash_one(v));
}

/// Mixes the reader's calls as `data`'s first byte says, so `read_raw`,
/// `read_value`, `peek_kind`, `find_pointer`, and typed reads all meet
/// every kind of input.
pub fn check_reader(data: &[u8]) {
    let Some((&ops, rest)) = data.split_first() else {
        return;
    };
    let Ok(text) = core::str::from_utf8(rest) else {
        return;
    };
    for parser in [Parser::new(), TIGHT, GRAMMAR_ONLY] {
        let mut reader = parser.reader(text);
        for i in 0..text.len() + 4 {
            let step = match ops.rotate_left(i as u32 % 8) % 5 {
                0 => reader.next_event().map(|e| e.is_some()).map_err(drop),
                1 => reader.read_raw().map(|r| r.is_some()).map_err(drop),
                2 => reader.read_value().map(|v| v.is_some()).map_err(drop),
                3 => reader.peek_kind().map(|k| k.is_some()).map_err(drop),
                _ => reader.decode::<Value>().map(|_| true).map_err(drop),
            };
            if step.is_err() {
                break;
            }
        }
        // A pointer taken from the text itself.
        let pointer: String = text.chars().take(12).collect();
        let found = parser.reader(text).find_pointer(&pointer);
        if let (Ok(Some(raw)), Ok(v)) = (found, GRAMMAR_ONLY.parse(text)) {
            assert_eq!(GRAMMAR_ONLY.parse(raw).ok().as_ref(), v.pointer(&pointer));
        }
        // Document sequences never panic, and each document read is valid.
        let mut docs = parser.reader(text).documents();
        while let Ok(Some(raw)) = docs.read_raw() {
            assert!(GRAMMAR_ONLY.parse(raw).is_ok());
        }
    }
}

/// Feeds `data` (after a first byte that picks the chunk size) to the
/// incremental reader and checks it against the complete reader over the
/// valid UTF-8 prefix.
pub fn check_stream(data: &[u8]) {
    let Some((&chunk, rest)) = data.split_first() else {
        return;
    };
    let chunk = usize::from(chunk % 16) + 1;
    let valid = match core::str::from_utf8(rest) {
        Ok(_) => rest.len(),
        Err(e) => e.valid_up_to(),
    };
    let prefix = core::str::from_utf8(&rest[..valid]).unwrap();
    for parser in [Parser::new(), TIGHT, GRAMMAR_ONLY] {
        // The complete reader's events and verdict.
        let mut expected = Vec::new();
        let mut reader = parser.reader(prefix);
        let verdict = loop {
            match reader.next_event() {
                Ok(Some(e)) => expected.push(owned(e)),
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        let verdict = match verdict {
            Err(e)
                if valid < rest.len()
                    && e.kind() == ErrorKind::UnexpectedEnd
                    && e.offset() == valid =>
            {
                Err((ErrorKind::InvalidUtf8, valid))
            }
            Ok(()) if valid < rest.len() => Err((ErrorKind::InvalidUtf8, valid)),
            other => other.map_err(|e| (e.kind(), e.offset())),
        };
        // The stream's.
        let mut stream = parser.stream_reader();
        let mut got = Vec::new();
        let mut pieces = rest.chunks(chunk);
        let streamed = loop {
            match stream.next_event() {
                Ok(Step::Ready(Some(e))) => got.push(owned(e)),
                Ok(Step::Ready(None)) => break Ok(()),
                Ok(Step::NeedInput) => match pieces.next() {
                    Some(piece) => {
                        if let Err(e) = stream.feed(piece) {
                            break Err((e.kind(), e.offset()));
                        }
                    }
                    None => stream.end_input(),
                },
                Err(e) => break Err((e.kind(), e.offset())),
            }
        };
        assert_eq!(got, expected);
        assert_eq!(streamed, verdict);
    }
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

/// Typed reads of arbitrary bytes return values or errors, and agree with
/// the tree where they overlap.
pub fn check_decode(data: &[u8]) {
    let _ = jsonx_lib::from_bytes::<Vec<Option<i64>>>(data);
    let _ = jsonx_lib::from_bytes::<BTreeMap<String, Vec<f64>>>(data);
    let _ = jsonx_lib::from_bytes::<(bool, String, [u8; 2])>(data);
    let _ = jsonx_lib::from_bytes::<Vec<Nullable<char>>>(data);
    let _ = jsonx_lib::from_bytes::<Vec<&str>>(data);
    let lenient = Parser::new().numbers(jsonx_lib::Numbers::WholeValues);
    let _ = lenient.decode_bytes::<Vec<u128>>(data);
    if let Ok(raw) = jsonx_lib::from_bytes::<RawValue>(data) {
        assert_eq!(raw.parse().ok(), jsonx_lib::parse_bytes(data).ok());
    }
    let typed = jsonx_lib::from_bytes::<Value>(data);
    assert_eq!(typed.ok(), jsonx_lib::parse_bytes(data).ok());
    if let Err(e) = jsonx_lib::from_bytes::<Vec<u8>>(data) {
        let _ = (
            e.to_string(),
            e.pointer(),
            e.line_col(data),
            e.utf16_position(data),
        );
    }
    if let Err(e) = jsonx_lib::parse_bytes(data) {
        let (line, col) = e.line_col(data);
        assert!(line >= 1 && col >= 1);
    }
}

/// A valid document transcoded event by event through the push writer is
/// what the value writers write.
pub fn check_write(data: &[u8]) {
    let Ok(text) = core::str::from_utf8(data) else {
        return;
    };
    let Ok(v) = GRAMMAR_ONLY.parse(text) else {
        return;
    };
    for format in [Format::compact(), Format::pretty()] {
        let mut out = String::new();
        let mut w = Writer::new(&mut out).with_format(format);
        for event in GRAMMAR_ONLY.reader(text) {
            w.event(&event.unwrap()).unwrap();
        }
        w.finish().unwrap();
        assert_eq!(out, jsonx_lib::to_string_with(&v, format));
    }
    assert_eq!(jsonx_lib::to_value(&v), v);
}

/// Every check.
pub fn check_all(data: &[u8]) {
    check_parse(data);
    check_reader(data);
    check_stream(data);
    check_decode(data);
    check_write(data);
}
