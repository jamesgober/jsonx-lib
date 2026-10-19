//! Property tests for every invariant in `dev/DIRECTIVES.md` §4, checked
//! against the independent reference parser in `tests/common` wherever one
//! applies.

mod common;

use common::{
    Fail, RefValue,
    strategies::{mutated, soup, tricky_string, value},
    to_ref,
};
use jsonx_lib::{
    DuplicateKeys, ErrorKind, Event, Format, Limits, Parser, ToJson, Value, ValueWriter, Writer,
    Written,
};
use proptest::{prelude::*, sample::Index};

/// No budgets and no duplicate check: the grammar alone decides.
const GRAMMAR_ONLY: Parser = Parser::new()
    .limits(Limits::unlimited())
    .duplicate_keys(DuplicateKeys::KeepAll);

// ---- The agreement check ----------------------------------------------------------

/// The crate and the reference must agree on whether `text` is JSON, on the
/// value when it is, and on the exact offset when it is not.
fn agrees_with_reference(text: &str) -> Result<(), TestCaseError> {
    let ours = GRAMMAR_ONLY.parse(text);
    let reference = common::parse(text.as_bytes());
    match &ours {
        Ok(v) => {
            prop_assert_eq!(reference.lone_surrogate, None);
            let expected = to_ref(v);
            prop_assert_eq!(reference.result.as_ref().ok(), Some(&expected));
        }
        Err(e) => match e.kind() {
            ErrorKind::LoneSurrogate => {
                prop_assert_eq!(reference.lone_surrogate, Some(e.offset()), "{:?}", text);
            }
            ErrorKind::UnexpectedEnd => {
                prop_assert_eq!(e.offset(), text.len());
                prop_assert_eq!(
                    reference.result.as_ref().err(),
                    Some(&Fail::Eof),
                    "{:?}",
                    text
                );
                prop_assert_eq!(reference.lone_surrogate, None);
            }
            kind => {
                prop_assert!(!kind.is_budget());
                let dead = Fail::Dead(e.offset());
                let want = Some(&dead);
                prop_assert_eq!(reference.result.as_ref().err(), want, "{:?} {:?}", text, e);
                prop_assert_eq!(reference.lone_surrogate, None);
            }
        },
    }
    // The pull reader, driven to the end, reaches the same verdict.
    let mut reader = GRAMMAR_ONLY.reader(text);
    let streamed = loop {
        match reader.next_event() {
            Ok(Some(_)) => {}
            Ok(None) => break Ok(()),
            Err(e) => break Err(e),
        }
    };
    prop_assert_eq!(streamed.err(), ours.err());
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// Invariant 1: exactly RFC 8259, with the exact error offset.
    #[test]
    fn prop_soup_agrees_with_reference(text in soup()) {
        agrees_with_reference(&text)?;
    }

    /// Invariant 1, near valid documents.
    #[test]
    fn prop_mutations_agree_with_reference(text in mutated()) {
        agrees_with_reference(&text)?;
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// Invariant 2: the writer's output parses back to the same value,
    /// compact and pretty, and writing is deterministic.
    #[test]
    fn prop_round_trip(v in value()) {
        let compact = jsonx_lib::to_string(&v);
        let back = GRAMMAR_ONLY.parse(&compact);
        prop_assert_eq!(back.as_ref().ok(), Some(&v));
        prop_assert_eq!(back.map(|b| jsonx_lib::to_string(&b)).ok(), Some(compact.clone()));

        let pretty = jsonx_lib::to_string_pretty(&v);
        prop_assert_eq!(GRAMMAR_ONLY.parse(&pretty).ok(), Some(v.clone()));

        // The independent reference reads the same value from both.
        let reference = common::parse(compact.as_bytes());
        prop_assert_eq!(reference.lone_surrogate, None);
        prop_assert_eq!(reference.result.ok(), Some(to_ref(&v)));
        prop_assert_eq!(common::parse(pretty.as_bytes()).result.ok(), Some(to_ref(&v)));

        // Every output path writes the same bytes.
        #[cfg(feature = "std")]
        {
            let mut io = Vec::new();
            prop_assert!(jsonx_lib::to_writer(&mut io, &v).is_ok());
            prop_assert_eq!(io, compact.as_bytes());
        }
        prop_assert_eq!(format!("{v}"), compact);
        prop_assert_eq!(format!("{v:#}"), pretty);
    }

    /// Invariant 4: no string, as a value or a key, can change the structure
    /// of the emitted JSON; it always reads back as itself.
    #[test]
    fn prop_escaping_cannot_inject(s in tricky_string()) {
        let mut lit = String::new();
        jsonx_lib::write_string(&mut lit, &s);
        prop_assert!(lit.bytes().all(|b| b >= 0x20));
        let separators = ['\u{2028}', '\u{2029}'];
        prop_assert!(!lit.contains(separators));

        let alone = common::parse(lit.as_bytes());
        prop_assert_eq!(alone.lone_surrogate, None);
        prop_assert_eq!(alone.result.ok(), Some(RefValue::Str(s.clone())));

        let embedded = format!("{{\"k\":{lit},{lit}:[{lit}],\"z\":1}}");
        let want = RefValue::Obj(vec![
            ("k".into(), RefValue::Str(s.clone())),
            (s.clone(), RefValue::Arr(vec![RefValue::Str(s.clone())])),
            ("z".into(), RefValue::Num("1".into())),
        ]);
        prop_assert_eq!(common::parse(embedded.as_bytes()).result.ok(), Some(want));
    }

    /// Every proper prefix of a document is refused as cut short, never as
    /// malformed (unless it is itself a complete document, like `1` from `12`).
    #[test]
    fn prop_truncation_is_unexpected_end(v in value(), at in any::<Index>()) {
        let text = jsonx_lib::to_string(&v);
        let bounds: Vec<usize> = (0..text.len()).filter(|&i| text.is_char_boundary(i)).collect();
        prop_assume!(!bounds.is_empty());
        let cut = bounds.get(at.index(bounds.len())).copied().unwrap_or(0);
        let prefix = text.get(..cut).unwrap_or("");
        if let Err(e) = GRAMMAR_ONLY.parse(prefix) {
            prop_assert_eq!((e.kind(), e.offset()), (ErrorKind::UnexpectedEnd, cut));
        }
    }

    /// `read_raw` returns exact slices: each parses to the item it covers.
    #[test]
    fn prop_read_raw_slices_parse_to_their_items(items in prop::collection::vec(value(), 0..6)) {
        let doc = Value::from(items.clone());
        let text = jsonx_lib::to_string_pretty(&doc);
        let mut reader = GRAMMAR_ONLY.reader(&text);
        prop_assert_eq!(reader.next_event().ok().flatten(), Some(Event::StartArray));
        for item in &items {
            let raw = reader.read_raw();
            let raw = raw.ok().flatten().unwrap_or_default();
            let parsed = GRAMMAR_ONLY.parse(raw).ok();
            prop_assert_eq!(parsed.as_ref(), Some(item));
        }
        prop_assert_eq!(reader.read_raw().ok(), Some(None));
        prop_assert_eq!(reader.next_event().ok().flatten(), Some(Event::EndArray));
        prop_assert!(reader.finish().is_ok());
    }

    /// Typed output (`ToJson`) nests a `Value` in the same layout as a `Value`
    /// built around it, compact and pretty, and every output path agrees.
    #[test]
    fn prop_typed_output_matches_value_output(v in value(), items in prop::collection::vec(value(), 0..4)) {
        struct Holder<'a> { v: &'a Value<'a>, items: &'a [Value<'static>] }
        impl ToJson for Holder<'_> {
            fn to_json(&self, out: ValueWriter<'_>) -> Written {
                out.object(|o| {
                    o.member("v", self.v);
                    o.member_with("items", |w| w.array(|a| a.items(self.items)));
                    o.member("n", &[1u8, 2]);
                })
            }
        }
        let holder = Holder { v: &v, items: &items };
        let mut built = jsonx_lib::Object::new();
        built.push("v", v.clone());
        built.push("items", Value::from(items.clone()));
        built.push("n", jsonx_lib::json!([1, 2]));
        let built = Value::Object(built);
        prop_assert_eq!(jsonx_lib::to_string(&holder), jsonx_lib::to_string(&built));
        prop_assert_eq!(jsonx_lib::to_string_pretty(&holder), jsonx_lib::to_string_pretty(&built));
        #[cfg(feature = "std")]
        {
            let mut io = Vec::new();
            prop_assert!(jsonx_lib::to_writer_pretty(&mut io, &holder).is_ok());
            prop_assert_eq!(io, jsonx_lib::to_string_pretty(&built).into_bytes());
        }
    }

    /// Every format writes text that reads back as the same value, under
    /// both parsers, with the byte property it promises.
    #[test]
    fn prop_every_format_round_trips(v in value(), indent in 0u8..9, pretty in any::<bool>(), ascii in any::<bool>(), html in any::<bool>()) {
        let base = if pretty { Format::compact().with_indent(indent) } else { Format::compact() };
        let format = base.ascii(ascii).html_safe(html);
        let text = jsonx_lib::to_string_with(&v, format);
        prop_assert_eq!(GRAMMAR_ONLY.parse(&text).ok(), Some(v.clone()));
        prop_assert_eq!(common::parse(text.as_bytes()).result.ok(), Some(to_ref(&v)));
        if ascii {
            prop_assert!(text.is_ascii());
        }
        if html {
            prop_assert!(!text.contains(['<', '>', '&']));
        }
        // The default formats are the 0.2 layouts.
        prop_assert_eq!(jsonx_lib::to_string_with(&v, Format::compact()), jsonx_lib::to_string(&v));
        prop_assert_eq!(jsonx_lib::to_string_with(&v, Format::pretty()), jsonx_lib::to_string_pretty(&v));
    }

    /// Raw text obeys the strict formats too: forwarded verbatim in the
    /// plain ones, with `<`, `>`, `&`, and non-ASCII escaped in its strings
    /// under HTML-safe and ASCII-only, and the same value either way.
    #[test]
    fn prop_raw_values_obey_every_format(v in value(), s in tricky_string(), pretty in any::<bool>(), ascii in any::<bool>(), html in any::<bool>()) {
        let base = if pretty { Format::pretty() } else { Format::compact() };
        let format = base.ascii(ascii).html_safe(html);
        let doc = jsonx_lib::json!({ "s": s.as_str(), "v": v.clone(), "tag": "</script>&<b>caf\u{e9}\u{2028}" });
        let raw = jsonx_lib::RawValue::encode(&doc);
        let text = jsonx_lib::to_string_with(&raw, format);
        prop_assert_eq!(GRAMMAR_ONLY.parse(&text).ok(), Some(doc.clone()));
        if ascii {
            prop_assert!(text.is_ascii());
        }
        if html {
            prop_assert!(!text.contains(['<', '>', '&']));
        }
        if !ascii && !html {
            prop_assert_eq!(text.as_str(), raw.as_str());
        }
        // The push writer writes raw values the same way.
        let mut out = String::new();
        let mut w = Writer::new(&mut out).with_format(format);
        w.begin_array().unwrap();
        w.value(&raw).unwrap();
        w.end_array().unwrap();
        w.finish().unwrap();
        prop_assert_eq!(GRAMMAR_ONLY.parse(&out).ok(), Some(Value::Array(vec![doc].into())));
        if html {
            prop_assert!(!out.contains(['<', '>', '&']));
        }
    }

    /// The push writer, fed a reader's events, writes what the value writers
    /// write, in every layout.
    #[test]
    fn prop_events_through_the_writer_match_the_value_writer(v in value(), pretty in any::<bool>()) {
        let format = if pretty { Format::pretty() } else { Format::compact() };
        let text = jsonx_lib::to_string(&v);
        let mut out = String::new();
        let mut w = Writer::new(&mut out).with_format(format);
        for event in GRAMMAR_ONLY.reader(&text) {
            prop_assert!(w.event(&event.unwrap_or(Event::Null)).is_ok());
        }
        prop_assert!(w.finish().is_ok());
        prop_assert_eq!(out, jsonx_lib::to_string_with(&v, format));
    }

    /// `Reader::find_pointer` finds what `Value::pointer` finds, for every
    /// pointer to a value in the document and some that miss.
    #[test]
    fn prop_find_pointer_matches_value_pointer(v in value()) {
        let text = jsonx_lib::to_string_pretty(&v);
        // Every path in the value, breadth first, capped.
        let mut paths = vec![String::new()];
        let mut i = 0;
        while i < paths.len() && paths.len() < 64 {
            let path = paths[i].clone();
            if let Some(target) = v.pointer(&path) {
                match target {
                    Value::Array(a) => paths.extend((0..=a.len()).map(|n| format!("{path}/{n}"))),
                    Value::Object(o) => {
                        for key in o.keys() {
                            paths.push(format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")));
                        }
                        paths.push(format!("{path}/no such key"));
                    }
                    _ => paths.push(format!("{path}/0")),
                }
            }
            i += 1;
        }
        paths.extend(["x", "/~2", "/01"].map(String::from));
        for path in &paths {
            let found = GRAMMAR_ONLY.reader(&text).find_pointer(path);
            let found = found.ok().flatten().and_then(|raw| GRAMMAR_ONLY.parse(raw).ok());
            prop_assert_eq!(found.as_ref(), v.pointer(path), "{}", path);
        }
    }

    /// `Hash` agrees with `==`: a value and its round trip hash alike.
    #[test]
    fn prop_hash_agrees_with_eq(v in value()) {
        use std::hash::{BuildHasher, RandomState};
        let state = RandomState::new();
        let text = jsonx_lib::to_string(&v);
        let back = GRAMMAR_ONLY.parse(&text).unwrap_or_default();
        prop_assert_eq!(&back, &v);
        prop_assert_eq!(state.hash_one(&back), state.hash_one(&v));
        prop_assert_eq!(state.hash_one(back.into_owned()), state.hash_one(&v));
    }

    /// Clone, equality, and `into_owned` agree with each other.
    #[test]
    fn prop_clone_and_into_owned_preserve_equality(v in value()) {
        let text = jsonx_lib::to_string(&v);
        let parsed = GRAMMAR_ONLY.parse(&text).unwrap_or_default();
        prop_assert_eq!(&parsed.clone(), &v);
        prop_assert_eq!(parsed.into_owned(), v.clone());
    }
}

/// Objects whose keys come from a small set, so they often repeat.
fn repeating_objects() -> impl Strategy<Value = String> {
    let key = prop::sample::select(vec!["a", "b", "\\u0061", "c\\n", "c\\u000a"]);
    let leaf = prop_oneof![
        Just("1".to_owned()),
        Just("[]".to_owned()),
        Just("\"x\"".to_owned())
    ];
    leaf.prop_recursive(4, 40, 6, move |inner| {
        prop_oneof![
            prop::collection::vec((key.clone(), inner.clone()), 0..6).prop_map(|members| {
                let body: Vec<String> = members
                    .iter()
                    .map(|(k, v)| format!("\"{k}\":{v}"))
                    .collect();
                format!("{{{}}}", body.join(","))
            }),
            prop::collection::vec(inner, 0..4).prop_map(|items| format!("[{}]", items.join(","))),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// JX-SEC-02: under the default policy, every way of reading refuses the
    /// same duplicate at the same offset: `parse`, the event loop,
    /// `read_raw`, and the incremental reader.
    #[test]
    fn prop_every_reading_path_refuses_the_same_duplicates(text in repeating_objects(), cut in any::<Index>()) {
        let parsed = jsonx_lib::parse(&text).err();
        let mut events = jsonx_lib::Reader::new(&text);
        let streamed = loop {
            match events.next_event() {
                Ok(Some(_)) => {}
                Ok(None) => break None,
                Err(e) => break Some(e),
            }
        };
        prop_assert_eq!(streamed, parsed);
        prop_assert_eq!(jsonx_lib::Reader::new(&text).read_raw().err(), parsed);
        // A pointer lookup answers only for a document without repeats, a
        // member whose key repeats included.
        for pointer in ["", "/a", "/0", "/a/a", "/b/0/a"] {
            let found = jsonx_lib::Reader::new(&text).find_pointer(pointer);
            prop_assert_eq!(found.err(), parsed, "{}", pointer);
        }
        let mut incremental = jsonx_lib::StreamReader::new();
        let at = cut.index(text.len() + 1);
        let (head, tail) = text.as_bytes().split_at(at);
        prop_assert!(incremental.feed(head).is_ok());
        let raw = match incremental.read_raw() {
            Err(e) => Err(e),
            Ok(jsonx_lib::Step::Ready(_)) => Ok(()),
            Ok(jsonx_lib::Step::NeedInput) => incremental.feed(tail).and_then(|()| {
                incremental.end_input();
                incremental.read_raw().map(|_| ())
            }),
        };
        prop_assert_eq!(raw.err(), parsed);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// Invariant 3: arbitrary bytes never panic, through every entry point.
    #[test]
    fn prop_arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _default = jsonx_lib::parse_bytes(&bytes);
        let _tight = Parser::new()
            .limits(Limits::new().with_max_depth(3).with_max_string_bytes(5).with_max_number_bytes(4).with_max_elements(7))
            .parse_bytes(&bytes);
        if let Ok(v) = GRAMMAR_ONLY.parse_bytes(&bytes) {
            let text = v.to_string();
            prop_assert_eq!(GRAMMAR_ONLY.parse(&text).ok(), Some(v));
        }
        if let Ok(mut reader) = GRAMMAR_ONLY.reader_bytes(&bytes) {
            for _ in 0..bytes.len() + 2 {
                match reader.read_raw() {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        if !matches!(reader.next_event(), Ok(Some(_))) {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }

    /// Invariant 3, on bytes that look like JSON.
    #[test]
    fn prop_json_like_bytes_never_panic(text in soup(), policy in 0..4u8) {
        let policy = match policy {
            0 => DuplicateKeys::Error,
            1 => DuplicateKeys::First,
            2 => DuplicateKeys::Last,
            _ => DuplicateKeys::KeepAll,
        };
        let _parsed = Parser::new().duplicate_keys(policy).parse(&text);
        let mut reader = Parser::new().duplicate_keys(policy).reader(&text);
        let _value = reader.read_value();
    }
}
