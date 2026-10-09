//! Property tests for every invariant in `dev/DIRECTIVES.md` §4, checked
//! against the independent reference parser in `tests/common` wherever one
//! applies.

mod common;

use common::{Fail, RefValue, to_ref};
use jsonx_lib::{DuplicateKeys, ErrorKind, Event, Limits, Number, Parser, Value};
use proptest::{prelude::*, sample::Index};

/// No budgets and no duplicate check: the grammar alone decides.
const GRAMMAR_ONLY: Parser = Parser::new()
    .limits(Limits::unlimited())
    .duplicate_keys(DuplicateKeys::KeepAll);

// ---- Generators ----------------------------------------------------------------

/// Any JSON number text, including ones too large or precise for any machine type.
fn number_text() -> impl Strategy<Value = String> {
    "-?(0|[1-9][0-9]{0,30})(\\.[0-9]{1,20})?([eE][+-]?[0-9]{1,5})?"
}

/// Strings rich in the characters escaping must handle.
fn tricky_string() -> impl Strategy<Value = String> {
    let ch = prop_oneof![
        3 => any::<char>(),
        1 => prop::sample::select(vec![
            '"', '\\', '/', '\u{0}', '\u{1}', '\u{8}', '\u{c}', '\n', '\r', '\t', '\u{1f}',
            '\u{7f}', '\u{2028}', '\u{2029}', '\u{2027}', '\u{20ac}', '\u{1f600}', 'u', 'a',
        ]),
    ];
    prop_oneof![
        prop::collection::vec(ch, 0..24).prop_map(String::from_iter),
        any::<String>(),
    ]
}

fn leaf() -> impl Strategy<Value = Value<'static>> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>().prop_map(Value::from),
        number_text().prop_map(|t| {
            Number::from_text(&t).map_or(Value::Null, |n| Value::Number(n.into_owned()))
        }),
        tricky_string().prop_map(Value::from),
    ]
}

/// Arbitrary documents. Object keys may repeat.
fn value() -> impl Strategy<Value = Value<'static>> {
    leaf().prop_recursive(6, 96, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(Value::from),
            prop::collection::vec((tricky_string(), inner), 0..8)
                .prop_map(|members| Value::Object(members.into_iter().collect())),
        ]
    })
}

/// Pieces that, glued at random, land near every grammar boundary.
const FRAGMENTS: &[&str] = &[
    "{",
    "}",
    "[",
    "]",
    ",",
    ":",
    "\"",
    "\\",
    "\"a\"",
    "\"k\":",
    "\"\\u00e9\"",
    "\"\\ud83d\\ude00\"",
    "\\ud800",
    "\\udc00",
    "\\u",
    "u",
    "0",
    "1",
    "9",
    "-",
    ".",
    "e",
    "E",
    "+",
    "true",
    "tru",
    "false",
    "null",
    "nul",
    " ",
    "\n",
    "\t",
    "\r",
    "\u{1}",
    "\u{e9}",
    "\u{2028}",
    "\u{feff}",
    "x",
    "/",
    "b",
    "n",
    "12",
    "0.5",
    "1e3",
    "\\n",
    "\\\"",
    "D800",
    "DC00",
    "dc00",
    "00",
];

fn soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..24).prop_map(|parts| parts.concat())
}

/// A valid document with one random edit at a character boundary.
fn mutated() -> impl Strategy<Value = String> {
    (
        value(),
        any::<Index>(),
        prop::sample::select(FRAGMENTS),
        0..3u8,
    )
        .prop_map(|(v, at, fragment, edit)| {
            let text = jsonx_lib::to_string(&v);
            let bounds: Vec<usize> = (0..=text.len())
                .filter(|&i| text.is_char_boundary(i))
                .collect();
            let i = bounds.get(at.index(bounds.len())).copied().unwrap_or(0);
            let (head, tail) = text.split_at(i);
            match edit {
                0 => format!("{head}{fragment}{tail}"),
                1 => {
                    let mut chars = tail.chars();
                    let _removed = chars.next();
                    format!("{head}{}", chars.as_str())
                }
                _ => head.to_owned(),
            }
        })
}

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

    /// Clone, equality, and `into_owned` agree with each other.
    #[test]
    fn prop_clone_and_into_owned_preserve_equality(v in value()) {
        let text = jsonx_lib::to_string(&v);
        let parsed = GRAMMAR_ONLY.parse(&text).unwrap_or_default();
        prop_assert_eq!(&parsed.clone(), &v);
        prop_assert_eq!(parsed.into_owned(), v.clone());
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
