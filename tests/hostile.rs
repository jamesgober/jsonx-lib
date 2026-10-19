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
fn test_million_deep_nesting_default_limits_refused_at_the_depth_budget() {
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
fn test_million_deep_unclosed_input_unlimited_is_unexpected_end() {
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
fn test_million_deep_value_small_stack_every_operation_succeeds() {
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
fn test_million_deep_objects_small_stack_round_trip() {
    on_small_stack(|| {
        let text = "{\"k\":".repeat(MILLION) + "null" + &"}".repeat(MILLION);
        let p = Parser::new().limits(Limits::unlimited());
        let v = p.parse(&text).unwrap_or_default();
        assert_eq!(v.to_string(), text);
        assert_eq!(v.clone(), v);
    });
}

#[test]
fn test_deep_value_built_in_code_small_stack_drops() {
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
fn test_every_budget_at_its_boundary_refuses_only_past_it() {
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
fn test_default_budgets_oversized_documents_are_refused() {
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
fn test_wide_objects_duplicate_check_is_not_quadratic() {
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
fn test_long_escape_runs_decode_in_linear_time() {
    let body = "\\n\\u00e9\\ud83d\\ude00".repeat(100_000);
    let text = format!("\"{body}\"");
    let v = jsonx_lib::parse(&text).unwrap_or_default();
    assert_eq!(v.as_str().map(str::len), Some(100_000 * (1 + 2 + 4)));
}

#[test]
fn test_pretty_printing_deep_values_small_stack_succeeds() {
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
fn test_push_writer_million_deep_transcoding_small_stack_succeeds() {
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

/// `max_memory` bounds what one read builds, not what a document yields
/// over many reads: a large array read item by item, each item dropped,
/// reads in full, as in 0.2 (verification JX3-COR-01).
#[test]
fn test_read_value_item_by_item_past_max_memory_reads_every_item() {
    const ITEMS: usize = 2_000_000;
    let mut text = String::with_capacity(ITEMS * 20 + 2);
    text.push('[');
    for i in 0..ITEMS {
        if i > 0 {
            text.push(',');
        }
        text.push_str(r#"{"id":1,"name":"x"}"#);
    }
    text.push(']');
    // Over the input and value budgets, but not over the memory budget,
    // which stays at its default.
    let limits = Limits::new()
        .with_max_input_bytes(1 << 30)
        .with_max_elements(usize::MAX);
    assert_eq!(limits.max_memory, Limits::new().max_memory);
    let parser = Parser::new().limits(limits);
    let mut r = parser.reader(&text);
    assert_eq!(
        r.next_event().ok().flatten(),
        Some(jsonx_lib::Event::StartArray)
    );
    let mut read = 0usize;
    while let Some(item) = r.read_value().unwrap() {
        assert_eq!(item["id"], 1);
        read += 1;
    }
    assert_eq!(read, ITEMS);
    assert!(r.finish().is_ok());

    // The same through typed reads, item by item, under a tight budget.
    let tight = Parser::new().limits(limits.with_max_memory(4096));
    let mut r = tight.reader(&text[..20 * 100_000]);
    r.next_event().unwrap();
    for _ in 0..99_999 {
        let item: jsonx_lib::Value = r.decode().unwrap();
        assert_eq!(item["name"], "x");
    }
}

/// Under the fully default budgets: more than `max_memory` of decoded
/// strings in all, each item far below it.
#[test]
fn test_read_value_default_budgets_decoded_strings_past_max_memory_read_in_full() {
    let item = format!(r#"{{"s":"{}"}}"#, "\\n".repeat(500));
    let count = 64_000;
    let text = format!("[{}]", vec![item.as_str(); count].join(","));
    assert!(text.len() < Limits::new().max_input_bytes);
    // Each item holds its decoded string's buffer (sized from its 1000-byte
    // source), its member, and its value: more than 64 MiB in all.
    let per_item = 1000 + 56 + size_of::<Value>();
    assert!(count * per_item > Limits::new().max_memory);
    let mut r = jsonx_lib::Reader::new(&text);
    r.next_event().unwrap();
    let mut read = 0usize;
    while let Some(v) = r.read_value().unwrap() {
        assert_eq!(v["s"].as_str().map(str::len), Some(500));
        read += 1;
    }
    assert_eq!(read, count);
}

/// One value over the budget is still refused, alone or nested in a typed
/// read, where the nested trees share the read's count.
#[test]
fn test_read_value_one_value_past_max_memory_is_refused() {
    let tight = Parser::new().limits(Limits::new().with_max_memory(1 << 20));
    let big = format!("[[{}0], 1]", "0,".repeat(100_000));
    let mut r = tight.reader(&big);
    r.next_event().unwrap();
    let err = r.read_value().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::MemoryLimitExceeded);
    // 40,000 small trees, each under the budget, in one typed read: over it.
    let many = format!("[{}{{}}]", r#"{"a":[1,2]},"#.repeat(40_000));
    let err = tight.decode::<Vec<jsonx_lib::Value>>(&many).unwrap_err();
    assert_eq!(
        err.kind(),
        &jsonx_lib::DecodeErrorKind::Syntax(ErrorKind::MemoryLimitExceeded)
    );
    // Read one by one, they fit.
    let mut r = tight.reader(&many);
    r.next_event().unwrap();
    let mut read = 0usize;
    while r.read_value().unwrap().is_some() {
        read += 1;
    }
    assert_eq!(read, 40_001);
}
