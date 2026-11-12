//! Edge cases across the reading and writing paths: the input budgets of a
//! long stream of documents and of a value being buffered, input after a
//! document's value, pointer lookups under each duplicate-key policy, error
//! messages built from hostile keys, and raw text in the default format.

use std::collections::BTreeMap;

use jsonx_lib::{
    DecodeError, DuplicateKeys, ErrorKind, Event, Format, Limits, Parser, RawValue, Reader, Step,
    StreamReader, Value, to_string, to_string_with,
};

// ---- Documents mode: the input budget is per document ------------------------

#[test]
fn documents_mode_counts_the_input_budget_per_document() {
    // Each line is 27 bytes; the budget is 64, and 27 000 bytes are fed.
    let parser = Parser::new().limits(Limits::new().with_max_input_bytes(64));
    let mut r = parser.stream_reader().documents();
    let line = b"{\"id\": 12345, \"ok\": true}\n";
    let mut docs = 0;
    for _ in 0..1000 {
        r.feed(line).unwrap();
        while let Step::Ready(Some(doc)) = r.read_value().unwrap() {
            assert_eq!(doc["id"], 12345);
            docs += 1;
        }
    }
    r.end_input();
    assert_eq!(r.read_value().unwrap(), Step::Ready(None));
    assert_eq!(docs, 1000);

    // The same with events only, and with documents of 40 bytes: what a
    // document finished before is never counted against the next.
    let mut r = parser.stream_reader().documents();
    let line = b"{\"id\": 12345, \"ok\": true, \"pad\": 12345}\n";
    assert_eq!(line.len(), 40);
    let mut ends = 0;
    for _ in 0..1000 {
        r.feed(line).unwrap();
        while let Step::Ready(Some(event)) = r.next_event().unwrap() {
            if event == Event::EndObject {
                ends += 1;
            }
        }
    }
    assert_eq!(ends, 1000);
}

#[test]
fn documents_mode_refuses_one_document_over_the_budget() {
    let parser = Parser::new().limits(Limits::new().with_max_input_bytes(16));
    let mut r = parser.stream_reader().documents();
    r.feed(b"[1, 2, 3]\n").unwrap();
    assert!(matches!(r.read_value().unwrap(), Step::Ready(Some(_))));
    // The second document starts at byte 10; it may run to byte 26.
    r.feed(b"[4, 5, ").unwrap();
    assert_eq!(r.read_value().unwrap(), Step::NeedInput);
    r.feed(b"6, 7, ").unwrap();
    let err = r.feed(b"8, 9]").unwrap_err();
    assert_eq!((err.kind(), err.offset()), (ErrorKind::InputTooLarge, 26));
    assert_eq!(r.read_value().unwrap_err(), err);

    // Outside documents mode the budget covers all the input, as before.
    let mut r = parser.stream_reader();
    r.feed(b"[1, 2, 3, 4, 5, ").unwrap();
    let err = r.feed(b"6]").unwrap_err();
    assert_eq!((err.kind(), err.offset()), (ErrorKind::InputTooLarge, 16));
}

// ---- The buffer behind the whole-value calls ---------------------------------

#[test]
fn whole_value_calls_bound_the_value_they_buffer() {
    // A long-lived stream with no total budget: the value budget still
    // bounds what a pending whole-value call holds.
    let limits = Limits::new()
        .with_max_input_bytes(usize::MAX)
        .with_max_value_bytes(4096);
    let parser = Parser::new().limits(limits);
    let spaces = [b' '; 512];

    for call in ["read_raw", "read_value", "decode"] {
        let mut r = parser.stream_reader().documents();
        r.feed(b"\n\n[1,").unwrap();
        let mut refused = None;
        for _ in 0..64 {
            let step = match call {
                "read_raw" => r
                    .read_raw()
                    .map(|s| s.is_ready())
                    .map_err(|e| e.to_string()),
                "read_value" => r
                    .read_value()
                    .map(|s| s.is_ready())
                    .map_err(|e| e.to_string()),
                _ => r
                    .decode::<Vec<u8>>()
                    .map(|s| s.is_ready())
                    .map_err(|e| e.to_string()),
            };
            assert_eq!(step, Ok(false), "{call}");
            if let Err(e) = r.feed(&spaces) {
                refused = Some(e);
                break;
            }
            assert!(r.buffered() <= 4096 + 2, "{call}: {}", r.buffered());
        }
        // The value starts at byte 2 and may hold 4096 bytes.
        let err = refused.unwrap();
        assert_eq!(
            (err.kind(), err.offset()),
            (ErrorKind::InputTooLarge, 4098),
            "{call}"
        );
    }

    // Events stream in constant memory: no value is held for them.
    let mut r = parser.stream_reader().documents();
    r.feed(b"[1,").unwrap();
    for _ in 0..256 {
        r.feed(&spaces).unwrap();
        while let Step::Ready(Some(_)) = r.next_event().unwrap() {}
        assert!(r.buffered() < 16 * 1024);
    }

    // A value within the budget is read whole.
    let mut r = parser.stream_reader();
    r.feed(b"[1,").unwrap();
    assert_eq!(r.read_raw().unwrap(), Step::NeedInput);
    r.feed(&spaces).unwrap();
    r.feed(b"2]").unwrap();
    assert_eq!(
        r.read_raw().unwrap().ready().flatten().map(str::len),
        Some(3 + 512 + 2)
    );
}

#[test]
fn value_budget_defaults_and_setter() {
    assert_eq!(Limits::new().max_value_bytes, 64 << 20);
    assert_eq!(Limits::unlimited().max_value_bytes, usize::MAX);
    assert_eq!(Limits::new().with_max_value_bytes(9).max_value_bytes, 9);
}

// ---- Input after the document's value ----------------------------------------

#[test]
fn whole_value_calls_refuse_input_after_the_document() {
    for (text, at) in [("7 8", 2), ("[1] x", 4), ("{\"a\":1}}", 7), ("1 ,", 2)] {
        let expected = (ErrorKind::TrailingCharacters, at);

        let mut r = Reader::new(text);
        assert!(r.read_value().unwrap().is_some());
        let err = r.read_value().unwrap_err();
        assert_eq!((err.kind(), err.offset()), expected, "{text}");
        assert_eq!(r.next_event().unwrap_err(), err);

        let mut r = Reader::new(text);
        assert!(r.read_raw().unwrap().is_some());
        let err = r.read_raw().unwrap_err();
        assert_eq!((err.kind(), err.offset()), expected, "{text}");

        let mut s = StreamReader::new();
        s.feed(text.as_bytes()).unwrap();
        s.end_input();
        assert!(matches!(s.read_value().unwrap(), Step::Ready(Some(_))));
        let err = s.read_value().unwrap_err();
        assert_eq!((err.kind(), err.offset()), expected, "{text}");

        let mut s = StreamReader::new();
        s.feed(text.as_bytes()).unwrap();
        assert!(matches!(s.read_raw().unwrap(), Step::Ready(Some(_))));
        let err = s.read_raw().unwrap_err();
        assert_eq!((err.kind(), err.offset()), expected, "{text}");

        let mut s = StreamReader::new();
        s.feed(text.as_bytes()).unwrap();
        s.end_input();
        assert!(s.decode::<Value>().unwrap().is_ready());
        let err = s.decode::<Value>().unwrap_err();
        assert_eq!(
            err.syntax_error().map(|e| (e.kind(), e.offset())),
            Some(expected)
        );

        let mut s = StreamReader::new();
        s.feed(text.as_bytes()).unwrap();
        s.end_input();
        let err = s.finish().unwrap_err();
        assert_eq!((err.kind(), err.offset()), expected, "{text}");
    }

    // Whitespace after the value is fine, and documents mode reads on.
    let mut r = Reader::new("[1]  \n");
    assert!(r.read_value().unwrap().is_some());
    assert_eq!(r.read_value().unwrap(), None);
    let mut r = Reader::new("7 8").documents();
    assert!(r.read_value().unwrap().is_some());
    assert!(r.read_value().unwrap().is_some());
    assert_eq!(r.read_value().unwrap(), None);

    // Fed in pieces, the stream reader gives the complete reader's answer:
    // it waits rather than answer before the next byte is known.
    let mut s = StreamReader::new();
    s.feed(b"false").unwrap();
    assert!(s.next_event().unwrap().is_ready());
    assert_eq!(s.read_raw().unwrap(), Step::NeedInput);
    s.feed(b"{").unwrap();
    let err = s.read_raw().unwrap_err();
    assert_eq!(
        (err.kind(), err.offset()),
        (ErrorKind::TrailingCharacters, 5)
    );
}

#[test]
fn stream_reader_finish_checks_the_rest_of_the_input() {
    let mut s = StreamReader::new();
    s.feed(b"{\"a\": [1, ").unwrap();
    assert_eq!(s.finish().unwrap(), Step::NeedInput);
    s.feed(b"2]} ").unwrap();
    // The document is complete, but more input may still come.
    assert_eq!(s.finish().unwrap(), Step::NeedInput);
    s.end_input();
    assert_eq!(s.finish().unwrap(), Step::Ready(()));
    assert_eq!(s.finish().unwrap(), Step::Ready(()));

    // After a value read whole: the next whole-value call waits, as
    // `next_event` would, to see whether more than whitespace follows.
    let mut s = StreamReader::new();
    s.feed(b"[1]  ").unwrap();
    assert!(matches!(s.read_value().unwrap(), Step::Ready(Some(_))));
    assert_eq!(s.read_value().unwrap(), Step::NeedInput);
    assert_eq!(s.read_raw().unwrap(), Step::NeedInput);
    s.feed(b"\n").unwrap();
    assert_eq!(s.decode::<Value>().unwrap(), Step::NeedInput);
    s.end_input();
    assert_eq!(s.read_value().unwrap(), Step::Ready(None));
    assert_eq!(s.finish().unwrap(), Step::Ready(()));

    // An error in the rest of the document.
    let mut s = StreamReader::new();
    s.feed(b"[1, 2,]").unwrap();
    s.end_input();
    let err = s.finish().unwrap_err();
    assert_eq!((err.kind(), err.offset()), (ErrorKind::TrailingComma, 6));

    // Input that ends inside the document.
    let mut s = StreamReader::new();
    s.feed(b"[1").unwrap();
    s.end_input();
    assert_eq!(s.finish().unwrap_err().kind(), ErrorKind::UnexpectedEnd);

    // A whole-value call left waiting: its value is read as events, and its
    // budget no longer applies.
    let limits = Limits::new().with_max_value_bytes(8);
    let mut s = Parser::new().limits(limits).stream_reader();
    s.feed(b"[1, ").unwrap();
    assert_eq!(s.read_value().unwrap(), Step::NeedInput);
    assert_eq!(s.finish().unwrap(), Step::NeedInput);
    s.feed(b"2, 3, 4, 5]").unwrap();
    s.end_input();
    assert_eq!(s.finish().unwrap(), Step::Ready(()));

    // Documents mode: every document is checked.
    let mut s = StreamReader::new().documents();
    s.feed(b"{}\n[]\n[,]").unwrap();
    s.end_input();
    let err = s.finish().unwrap_err();
    assert_eq!((err.kind(), err.offset()), (ErrorKind::ExpectedValue, 7));
}

// ---- find_pointer under each duplicate-key policy ----------------------------

#[test]
fn find_pointer_takes_the_member_parse_keeps() {
    let doc = r#"{"role": "user", "x": {"k": 1, "k": 2}, "role": "admin"}"#;

    let last = Parser::new().duplicate_keys(DuplicateKeys::Last);
    let tree = last.parse(doc).unwrap();
    assert_eq!(tree["role"], "admin");
    assert_eq!(
        last.reader(doc).find_pointer("/role").unwrap(),
        Some(r#""admin""#)
    );
    assert_eq!(last.reader(doc).find_pointer("/x/k").unwrap(), Some("2"));
    assert_eq!(
        last.reader(doc).find_pointer("/x").unwrap(),
        Some(r#"{"k": 1, "k": 2}"#)
    );
    assert_eq!(last.reader(doc).find_pointer("/none").unwrap(), None);
    assert_eq!(last.reader(doc).find_pointer("/x/none").unwrap(), None);

    // Nested: the last member is the one descended into.
    let nested = r#"[0, {"a": {"b": 1}, "a": {"b": 2, "c": [3, 4]}}]"#;
    let mut r = last.reader(nested);
    assert_eq!(r.find_pointer("/1/a/b").unwrap(), Some("2"));
    // The reader stands past the value it started on.
    assert_eq!(r.next_event().unwrap(), None);
    assert_eq!(
        last.reader(nested).find_pointer("/1/a/c/1").unwrap(),
        Some("4")
    );
    assert_eq!(last.reader(nested).find_pointer("/1/a/c/9").unwrap(), None);
    assert_eq!(
        last.parse(nested).unwrap().pointer("/1/a/b"),
        Some(&Value::from(2))
    );

    // Inside a larger read, the reader stands past the value it started on.
    let mut r = last.reader(r#"{"m": {"a": 1, "a": 2}, "n": 3}"#);
    assert_eq!(r.next_event().unwrap(), Some(Event::StartObject));
    assert_eq!(r.next_event().unwrap(), Some(Event::Key("m".into())));
    assert_eq!(r.find_pointer("/a").unwrap(), Some("2"));
    assert_eq!(r.next_event().unwrap(), Some(Event::Key("n".into())));

    // An error after the target is still reported.
    let bad = r#"{"a": 1, "a": 2, "b": [1,]}"#;
    assert_eq!(
        last.reader(bad).find_pointer("/a").unwrap_err().kind(),
        ErrorKind::TrailingComma
    );

    // First: the first member, as parse keeps it.
    let first = Parser::new().duplicate_keys(DuplicateKeys::First);
    assert_eq!(first.parse(doc).unwrap()["role"], "user");
    assert_eq!(
        first.reader(doc).find_pointer("/role").unwrap(),
        Some(r#""user""#)
    );
    assert_eq!(first.reader(doc).find_pointer("/x/k").unwrap(), Some("1"));
}

// ---- DecodeError messages ----------------------------------------------------

#[test]
fn decode_error_display_escapes_and_shortens_keys() {
    let text = "{\"inner\\nINJECTED LOG LINE\": {\"n\": \"x\"}}";
    let err: DecodeError =
        jsonx_lib::from_str::<BTreeMap<String, BTreeMap<String, u32>>>(text).unwrap_err();
    // Programs see the exact key.
    assert_eq!(err.pointer(), "/inner\nINJECTED LOG LINE/n");
    assert_eq!(
        err.path().first().map(ToString::to_string).as_deref(),
        Some("inner\nINJECTED LOG LINE")
    );
    // The message is one line, with the control character escaped.
    let message = err.to_string();
    assert!(!message.contains('\n'), "{message}");
    assert!(
        message.ends_with("(at /inner\\u{a}INJECTED LOG LINE/n)"),
        "{message}"
    );

    // Other characters that can break a log line or hide text.
    let err = DecodeError::invalid_value("x", 0)
        .at_key("a\u{1b}[31m\r\u{2028}\u{202e}\\b~/")
        .at_index(2);
    assert_eq!(
        err.to_string(),
        "invalid value: expected x at byte 0 (at /2/a\\u{1b}[31m\\u{d}\\u{2028}\\u{202e}\\\\b~0~1)"
    );

    // A long key is cut.
    let long = "k".repeat(100_000);
    let err = DecodeError::invalid_value("x", 0).at_key(long.as_str());
    let message = err.to_string();
    assert!(message.len() < 200, "{}", message.len());
    assert!(
        message.ends_with(&format!("(at /{}...)", "k".repeat(64))),
        "{message}"
    );
    assert_eq!(err.pointer().len(), 100_001);

    // A short key is shown whole.
    let err = DecodeError::invalid_value("x", 0).at_key("k".repeat(64));
    assert!(
        err.to_string()
            .ends_with(&format!("(at /{})", "k".repeat(64)))
    );
}

// ---- Raw text and the line separators ----------------------------------------

#[test]
fn raw_text_keeps_line_separators_except_under_strict_formats() {
    let bs = '\\';
    let raw = RawValue::from_string(String::from("[\"a\u{2028}b\u{2029}c\", 1]")).unwrap();
    let escaped = format!("[\"a{bs}u2028b{bs}u2029c\", 1]");
    // The plain formats copy raw text as it is (documented on `Format`).
    assert_eq!(to_string(&raw), raw.as_str());
    assert_eq!(to_string_with(&raw, Format::pretty()), raw.as_str());
    // HTML-safe and ASCII output escape them, for JavaScript source.
    assert_eq!(
        to_string_with(&raw, Format::compact().html_safe(true)),
        escaped
    );
    assert_eq!(to_string_with(&raw, Format::compact().ascii(true)), escaped);
    // A string written from a value has them escaped in every format.
    assert_eq!(
        to_string("a\u{2028}b\u{2029}c"),
        format!("\"a{bs}u2028b{bs}u2029c\"")
    );
}
