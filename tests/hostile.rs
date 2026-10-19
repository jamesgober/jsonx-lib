//! Hostile input: extreme nesting, every budget at its boundary, and shapes
//! that would expose a quadratic path.

use jsonx_lib::{DuplicateKeys, ErrorKind, Limits, Parser, Value, json};

const MILLION: usize = 1_000_000;

fn kind_at<T>(r: Result<T, jsonx_lib::Error>) -> Option<(ErrorKind, usize)> {
    r.err().map(|e| (e.kind(), e.offset()))
}

/// Runs `f` on a thread with a deliberately small stack: anything that
/// recursed per level of nesting would overflow it.
fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    let handle = std::thread::Builder::new().stack_size(256 * 1024).spawn(f);
    let joined = handle.map(std::thread::JoinHandle::join);
    assert!(
        matches!(joined, Ok(Ok(()))),
        "the small-stack thread failed"
    );
}

#[test]
fn million_deep_nesting_is_refused_cleanly_by_default() {
    let open = "[".repeat(MILLION);
    assert_eq!(
        kind_at(jsonx_lib::parse(&open)),
        Some((ErrorKind::DepthLimitExceeded, 128))
    );
    let objects = "{\"a\":".repeat(MILLION);
    assert_eq!(
        kind_at(jsonx_lib::parse(&objects)),
        Some((ErrorKind::DepthLimitExceeded, 128 * 5))
    );
    let mut reader = jsonx_lib::Reader::new(&open);
    let mut events = 0;
    let err = loop {
        match reader.next_event() {
            Ok(Some(_)) => events += 1,
            Ok(None) => break None,
            Err(e) => break Some((e.kind(), e.offset())),
        }
    };
    assert_eq!(events, 128);
    assert_eq!(err, Some((ErrorKind::DepthLimitExceeded, 128)));
}

#[test]
fn million_deep_unclosed_input_is_unexpected_end_without_recursion() {
    on_small_stack(|| {
        let open = "[".repeat(MILLION);
        let p = Parser::new().limits(Limits::unlimited());
        assert_eq!(
            kind_at(p.parse(&open)),
            Some((ErrorKind::UnexpectedEnd, MILLION))
        );
    });
}

#[test]
fn million_deep_value_parses_writes_clones_compares_and_drops_without_recursion() {
    on_small_stack(|| {
        let text = "[".repeat(MILLION) + &"]".repeat(MILLION);
        let p = Parser::new().limits(Limits::unlimited());
        let v = p.parse(&text).unwrap_or_default();
        assert_eq!(jsonx_lib::to_string(&v), text);
        let copy = v.clone();
        assert_eq!(copy, v);
        let owned: Value<'static> = copy.into_owned();
        assert_eq!(owned, v);
        let mut skip = p.reader(&text);
        assert_eq!(
            skip.read_raw().ok().flatten().map(str::len),
            Some(2 * MILLION)
        );
        drop(owned);
        drop(v);
    });
}

#[test]
fn million_deep_objects_round_trip_without_recursion() {
    on_small_stack(|| {
        let text = "{\"k\":".repeat(MILLION) + "null" + &"}".repeat(MILLION);
        let p = Parser::new().limits(Limits::unlimited());
        let v = p.parse(&text).unwrap_or_default();
        assert_eq!(v.to_string(), text);
        assert_eq!(v.clone(), v);
    });
}

#[test]
fn deep_value_built_in_code_drops_without_recursion() {
    on_small_stack(|| {
        let mut v = json!(null);
        for i in 0..MILLION {
            v = if i % 2 == 0 {
                json!([v])
            } else {
                json!({ "x": v })
            };
        }
        let mut pretty = String::new();
        jsonx_lib::write_value(&mut pretty, &v);
        assert_eq!(pretty.len(), MILLION / 2 * 2 + MILLION / 2 * 6 + 4);
    });
}

#[test]
fn every_budget_at_its_boundary() {
    let p = |l: Limits| Parser::new().limits(l);

    // Depth: `[[1]]` is depth 2.
    for (max, ok) in [(1, false), (2, true), (3, true)] {
        let r = p(Limits::new().with_max_depth(max)).parse("[[1]]");
        assert_eq!(r.is_ok(), ok, "depth {max}");
    }
    assert!(p(Limits::new().with_max_depth(0)).parse("1").is_ok());
    assert_eq!(
        kind_at(p(Limits::new().with_max_depth(0)).parse("[]")),
        Some((ErrorKind::DepthLimitExceeded, 0))
    );

    // Input size: five bytes.
    for (max, ok) in [(4, false), (5, true), (6, true)] {
        let r = p(Limits::new().with_max_input_bytes(max)).parse("[123]");
        assert_eq!(r.is_ok(), ok, "input {max}");
    }
    assert_eq!(
        kind_at(p(Limits::new().with_max_input_bytes(4)).parse("[123]")),
        Some((ErrorKind::InputTooLarge, 4))
    );
    assert_eq!(
        kind_at(p(Limits::new().with_max_input_bytes(4)).parse_bytes(b"[\xff23]")),
        Some((ErrorKind::InputTooLarge, 4))
    );

    // String length, in source bytes between the quotes (an escape counts in full).
    for (max, ok) in [(3, false), (4, true), (5, true)] {
        let r = p(Limits::new().with_max_string_bytes(max)).parse("[\"ab\\n\"]");
        assert_eq!(r.is_ok(), ok, "string {max}");
    }
    assert_eq!(
        kind_at(p(Limits::new().with_max_string_bytes(3)).parse("[\"ab\\n\"]")),
        Some((ErrorKind::StringTooLong, 1))
    );
    // Keys count too.
    assert_eq!(
        kind_at(p(Limits::new().with_max_string_bytes(1)).parse("{\"ab\":1}")),
        Some((ErrorKind::StringTooLong, 1))
    );
    // A string cut short within its budget is just cut short.
    assert_eq!(
        kind_at(p(Limits::new().with_max_string_bytes(9)).parse("\"abc")),
        Some((ErrorKind::UnexpectedEnd, 4))
    );

    // Number length.
    for (max, ok) in [(5, false), (6, true), (7, true)] {
        let r = p(Limits::new().with_max_number_bytes(max)).parse("[-1.5e3]");
        assert_eq!(r.is_ok(), ok, "number {max}");
    }
    assert_eq!(
        kind_at(p(Limits::new().with_max_number_bytes(5)).parse("[-1.5e3]")),
        Some((ErrorKind::NumberTooLong, 1))
    );

    // Element count: `[1, [2], {"k": 3}]` holds six values.
    let doc = "[1, [2], {\"k\": 3}]";
    for (max, ok) in [(5, false), (6, true), (7, true)] {
        let r = p(Limits::new().with_max_elements(max)).parse(doc);
        assert_eq!(r.is_ok(), ok, "elements {max}");
    }
    assert_eq!(
        kind_at(p(Limits::new().with_max_elements(5)).parse(doc)),
        Some((ErrorKind::TooManyElements, 15))
    );
    assert_eq!(
        kind_at(p(Limits::new().with_max_elements(0)).parse("null")),
        Some((ErrorKind::TooManyElements, 0))
    );
}

#[test]
fn default_budgets_refuse_oversized_documents() {
    let limits = Limits::new();
    let digits = "1".repeat(limits.max_number_bytes + 1);
    assert_eq!(
        kind_at(jsonx_lib::parse(&digits)),
        Some((ErrorKind::NumberTooLong, 0))
    );
    assert!(jsonx_lib::parse(&digits[1..]).is_ok());

    let long = format!("\"{}\"", "a".repeat(limits.max_string_bytes + 1));
    assert_eq!(
        kind_at(jsonx_lib::parse(&long)),
        Some((ErrorKind::StringTooLong, 0))
    );

    let huge = " ".repeat(limits.max_input_bytes + 1);
    assert_eq!(
        kind_at(jsonx_lib::parse(&huge)),
        Some((ErrorKind::InputTooLarge, limits.max_input_bytes))
    );

    // The element budget (4 Mi values): the array is value 1, item i is value i + 2.
    let items = limits.max_elements + 10;
    let flat = format!("[{}0]", "0,".repeat(items - 1));
    let refused_item = limits.max_elements - 1;
    assert_eq!(
        kind_at(jsonx_lib::parse(&flat)),
        Some((ErrorKind::TooManyElements, 1 + 2 * refused_item))
    );
}

#[test]
fn wide_objects_check_duplicates_without_quadratic_cost() {
    // 200k distinct keys: a quadratic duplicate check would take minutes.
    let n = 200_000;
    let mut text = String::from("{");
    for i in 0..n {
        text.push_str(&format!("\"key{i}\":{i},"));
    }
    text.push_str("\"key0\":-1}");
    let err = kind_at(jsonx_lib::parse(&text));
    assert_eq!(err, Some((ErrorKind::DuplicateKey, text.len() - 10)));
    let first = Parser::new()
        .duplicate_keys(DuplicateKeys::First)
        .parse(&text)
        .unwrap_or_default();
    assert_eq!(first.as_object().map(jsonx_lib::Object::len), Some(n));
    assert_eq!(first["key0"], 0);
    let last = Parser::new()
        .duplicate_keys(DuplicateKeys::Last)
        .parse(&text)
        .unwrap_or_default();
    assert_eq!(last["key0"], -1);

    // Every key the same.
    let same = format!("{{{}\"k\":0}}", "\"k\":0,".repeat(n));
    let one = Parser::new()
        .duplicate_keys(DuplicateKeys::First)
        .parse(&same)
        .unwrap_or_default();
    assert_eq!(one.as_object().map(jsonx_lib::Object::len), Some(1));
}

#[test]
fn long_escape_runs_decode_in_linear_time() {
    let body = "\\n\\u00e9\\ud83d\\ude00".repeat(100_000);
    let text = format!("\"{body}\"");
    let v = jsonx_lib::parse(&text).unwrap_or_default();
    assert_eq!(v.as_str().map(str::len), Some(100_000 * (1 + 2 + 4)));
}

#[test]
fn pretty_printing_deep_values_is_iterative() {
    on_small_stack(|| {
        let depth = 2_000;
        let text = "[".repeat(depth) + &"]".repeat(depth);
        let v = Parser::new()
            .limits(Limits::unlimited())
            .parse(&text)
            .unwrap_or_default();
        let pretty = jsonx_lib::to_string_pretty(&v);
        let back = Parser::new()
            .limits(Limits::unlimited())
            .parse(&pretty)
            .unwrap_or_default();
        assert_eq!(back, v);
    });
}

#[test]
fn million_deep_transcoding_through_the_push_writer_is_iterative() {
    on_small_stack(|| {
        let text = "[{\"k\":".repeat(MILLION / 2) + "0" + &"}]".repeat(MILLION / 2);
        let p = Parser::new().limits(Limits::unlimited());
        let mut out = String::new();
        // Compact: pretty output of a million levels is quadratic in size.
        let mut w = jsonx_lib::Writer::new(&mut out);
        for event in p.reader(&text) {
            assert!(event.is_ok_and(|e| w.event(&e).is_ok()));
        }
        assert!(w.finish().is_ok());
        assert_eq!(out, text);
    });
}

#[test]
fn test_unterminated_string_within_the_budget_is_unexpected_end() {
    let max = 4;
    let p = Parser::new().limits(Limits::new().with_max_string_bytes(max));
    // A body of exactly the budget, cut off before its quote.
    assert_eq!(
        kind_at(p.parse("\"abcd")),
        Some((ErrorKind::UnexpectedEnd, 5))
    );
    // One byte more is over the budget.
    assert_eq!(
        kind_at(p.parse("\"abcde")),
        Some((ErrorKind::StringTooLong, 0))
    );
    // An unterminated string that starts past offset `max`.
    let late = format!("[{}\"ab", "0, ".repeat(4));
    assert_eq!(
        kind_at(p.parse(&late)),
        Some((ErrorKind::UnexpectedEnd, late.len()))
    );
    let late_long = format!("[{}\"abcde", "0, ".repeat(4));
    assert_eq!(
        kind_at(p.parse(&late_long)),
        Some((ErrorKind::StringTooLong, 13))
    );
}

#[test]
fn test_long_invalid_number_late_in_the_input_is_number_too_long() {
    let p = Parser::new().limits(Limits::new().with_max_number_bytes(3));
    let pad = "0, ".repeat(10);
    // Too long wins over the syntax error past the budget...
    let doc = format!("[{pad}1234.x]");
    assert_eq!(kind_at(p.parse(&doc)), Some((ErrorKind::NumberTooLong, 31)));
    // ...but a syntax error within the budget is reported as such.
    let doc = format!("[{pad}12.x]");
    assert_eq!(kind_at(p.parse(&doc)), Some((ErrorKind::InvalidNumber, 34)));
    // Cut short within the budget: the input simply ended.
    let doc = format!("[{pad}12.");
    assert_eq!(
        kind_at(p.parse(&doc)),
        Some((ErrorKind::UnexpectedEnd, doc.len()))
    );
}

#[test]
fn test_byte_entry_points_at_the_input_budget_accept_exactly_the_budget() {
    let p = Parser::new().limits(Limits::new().with_max_input_bytes(5));
    assert!(p.parse_bytes(b"[123]").is_ok());
    assert!(p.reader_bytes(b"[123]").is_ok());
    assert_eq!(
        kind_at(p.parse_bytes(b"[1234]")),
        Some((ErrorKind::InputTooLarge, 5))
    );
    assert_eq!(
        p.reader_bytes(b"[1234]")
            .err()
            .map(|e| (e.kind(), e.offset())),
        Some((ErrorKind::InputTooLarge, 5))
    );
    let mut s = p.stream_reader();
    assert!(s.feed(b"[12").is_ok() && s.feed(b"3]").is_ok());
    assert_eq!(
        s.feed(b" ").err().map(|e| e.kind()),
        Some(ErrorKind::InputTooLarge)
    );
}

#[test]
fn test_display_into_a_failing_formatter_returns_the_error() {
    use std::fmt::Write as _;

    struct Full;
    impl std::fmt::Write for Full {
        fn write_str(&mut self, _: &str) -> std::fmt::Result {
            Err(std::fmt::Error)
        }
    }
    let big: Value = (0..10_000).map(|i| json!({ "i": i })).collect();
    assert!(write!(Full, "{big}").is_err());
    assert!(write!(Full, "{big:#}").is_err());
    assert!(write!(Full, "{}", json!(1)).is_err());
}
