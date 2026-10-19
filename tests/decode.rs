//! Typed reading (`FromJson`) and writing (`ToJson`) end to end: shapes,
//! errors and their paths, borrowing, duplicates, raw values, document
//! sequences, and round trips.

mod common;

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, VecDeque},
    rc::Rc,
    sync::Arc,
};

use common::strategies::value;
use jsonx_lib::{
    Array, DecodeError, DecodeErrorKind, DuplicateKeys, ErrorKind, FromJson, Kind, Limits,
    Nullable, Number, Object, Parser, PathSegment, RawValue, Reader, Step, StreamReader, ToJson,
    Value, ValueWriter, Written, from_str,
};
use proptest::prelude::*;

fn kind_of<T>(r: Result<T, DecodeError>) -> Option<DecodeErrorKind> {
    r.err().map(|e| e.kind().clone())
}

#[derive(Debug, PartialEq)]
struct Position {
    line: u32,
    character: u32,
}

impl<'de> FromJson<'de> for Position {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let (mut line, mut character) = (None, None);
        let mut o = reader.read_object()?;
        while let Some(key) = o.next_key()? {
            match &*key {
                "line" => o.field(&mut line)?,
                "character" => o.field(&mut character)?,
                _ => {}
            }
        }
        Ok(Self {
            line: o.required(line, "line")?,
            character: o.required(character, "character")?,
        })
    }
}

impl ToJson for Position {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.object(|o| {
            o.member("line", &self.line);
            o.member("character", &self.character);
        })
    }
}

#[test]
fn test_struct_reads_and_writes_back() {
    let p: Position = from_str(r#"{"character": 4, "extra": [1, {"x": null}], "line": 2}"#)
        .unwrap_or(Position {
            line: 0,
            character: 0,
        });
    assert_eq!(
        p,
        Position {
            line: 2,
            character: 4
        }
    );
    assert_eq!(jsonx_lib::to_string(&p), r#"{"line":2,"character":4}"#);
    let back: Result<Position, _> = from_str(&jsonx_lib::to_string(&p));
    assert_eq!(back.ok(), Some(p));
}

#[test]
fn test_struct_errors_carry_kind_offset_and_path() {
    let text = r#"[{"line": 1, "character": 2}, {"line": 3}]"#;
    let err = from_str::<Vec<Position>>(text).err();
    assert_eq!(
        err.as_ref().map(|e| e.kind()),
        Some(&DecodeErrorKind::MissingField("character"))
    );
    assert_eq!(err.as_ref().map(DecodeError::offset), Some(30));
    assert_eq!(
        err.as_ref().map(DecodeError::pointer).as_deref(),
        Some("/1")
    );

    let err =
        from_str::<BTreeMap<String, Position>>(r#"{"a/b": {"line": -1, "character": 0}}"#).err();
    assert_eq!(
        err.as_ref().map(|e| e.kind()),
        Some(&DecodeErrorKind::OutOfRange { expected: "u32" })
    );
    assert_eq!(
        err.as_ref().map(|e| e.path().to_vec()),
        Some(vec![
            PathSegment::Key("a/b".into()),
            PathSegment::Key("line".into())
        ])
    );
    assert_eq!(
        err.as_ref().map(DecodeError::pointer).as_deref(),
        Some("/a~1b/line")
    );

    // A repeated field is refused at the second key, with its path.
    let err = from_str::<Position>(r#"{"line": 1, "line": 2, "character": 0}"#).err();
    assert_eq!(
        err.as_ref().map(|e| (e.kind().clone(), e.offset())),
        Some((DecodeErrorKind::DuplicateField, 12))
    );
    // A repeated unknown member is refused by the reader at the `}`.
    let err = from_str::<Position>(r#"{"x": 1, "x": 2, "line": 1, "character": 0}"#).err();
    assert_eq!(
        err.and_then(|e| e.syntax_error())
            .map(|e| (e.kind(), e.offset())),
        Some((ErrorKind::DuplicateKey, 9))
    );

    // Other policies are honoured by `field`.
    let first = Parser::new().duplicate_keys(DuplicateKeys::First);
    let last = Parser::new().duplicate_keys(DuplicateKeys::Last);
    let text = r#"{"line": 1, "line": 2, "character": 0}"#;
    assert_eq!(first.decode::<Position>(text).ok().map(|p| p.line), Some(1));
    assert_eq!(last.decode::<Position>(text).ok().map(|p| p.line), Some(2));

    // Syntax errors inside a typed read keep the path where they occurred.
    let err = from_str::<Vec<Position>>(r#"[{"line": 1, "character": 2,}]"#).err();
    assert_eq!(
        err.as_ref()
            .and_then(DecodeError::syntax_error)
            .map(|e| e.kind()),
        Some(ErrorKind::TrailingComma)
    );
    assert_eq!(
        err.as_ref().map(DecodeError::pointer).as_deref(),
        Some("/0")
    );
    assert!(err.is_some_and(|e| e.to_string().ends_with("(at /0)")));
}

#[test]
fn test_every_integer_type_at_its_limits() {
    macro_rules! check {
        ($($t:ty),*) => {$(
            let min = <$t>::MIN.to_string();
            let max = <$t>::MAX.to_string();
            assert_eq!(from_str::<$t>(&min).ok(), Some(<$t>::MIN));
            assert_eq!(from_str::<$t>(&max).ok(), Some(<$t>::MAX));
            // Ten times past the bound, or below zero for an unsigned type.
            let below = if min.starts_with('-') { format!("{min}0") } else { "-1".to_owned() };
            assert_eq!(kind_of(from_str::<$t>(&below)), Some(DecodeErrorKind::OutOfRange { expected: stringify!($t) }));
            let above = format!("{}0", max);
            assert_eq!(kind_of(from_str::<$t>(&above)), Some(DecodeErrorKind::OutOfRange { expected: stringify!($t) }));
            assert_eq!(kind_of(from_str::<$t>("1.0")), Some(DecodeErrorKind::OutOfRange { expected: stringify!($t) }));
            assert_eq!(kind_of(from_str::<$t>("1e2")), Some(DecodeErrorKind::OutOfRange { expected: stringify!($t) }));
            assert_eq!(kind_of(from_str::<$t>("\"1\"")), Some(DecodeErrorKind::WrongType { expected: stringify!($t), found: Kind::String }));
            assert_eq!(kind_of(from_str::<$t>("[1]")), Some(DecodeErrorKind::WrongType { expected: stringify!($t), found: Kind::Array }));
            assert_eq!(<$t as ToJson>::is_absent(&0), false);
            assert_eq!(jsonx_lib::to_string(&<$t>::MAX), max);
        )*};
    }
    check!(
        i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
    );
    assert_eq!(from_str::<u8>("-0").ok(), Some(0));
    assert_eq!(
        kind_of(from_str::<u8>("-1")),
        Some(DecodeErrorKind::OutOfRange { expected: "u8" })
    );
}

#[test]
fn test_floats_bools_chars_unit_and_strings() {
    assert_eq!(from_str::<f64>("1e2").ok(), Some(100.0));
    assert_eq!(
        from_str::<f64>("-0.0").ok().map(f64::to_bits),
        Some((-0.0f64).to_bits())
    );
    assert_eq!(
        kind_of(from_str::<f64>("1e400")),
        Some(DecodeErrorKind::OutOfRange { expected: "f64" })
    );
    assert_eq!(from_str::<f32>("0.1").ok(), Some(0.1));
    assert_eq!(
        kind_of(from_str::<f32>("1e39")),
        Some(DecodeErrorKind::OutOfRange { expected: "f32" })
    );
    assert_eq!(
        kind_of(from_str::<f64>("null")),
        Some(DecodeErrorKind::WrongType {
            expected: "f64",
            found: Kind::Null
        })
    );
    assert_eq!(from_str::<bool>("false").ok(), Some(false));
    assert_eq!(
        kind_of(from_str::<bool>("0")),
        Some(DecodeErrorKind::WrongType {
            expected: "a boolean",
            found: Kind::Number
        })
    );
    assert_eq!(from_str::<char>("\"\\u00e9\"").ok(), Some('\u{e9}'));
    assert_eq!(
        kind_of(from_str::<char>("\"ab\"")),
        Some(DecodeErrorKind::InvalidValue("a single character"))
    );
    assert_eq!(
        kind_of(from_str::<char>("\"\"")),
        Some(DecodeErrorKind::InvalidValue("a single character"))
    );
    assert_eq!(from_str::<()>("null").ok(), Some(()));
    assert_eq!(
        kind_of(from_str::<()>("{}")),
        Some(DecodeErrorKind::WrongType {
            expected: "null",
            found: Kind::Object
        })
    );

    let text = r#"["plain", "esc\n"]"#;
    let strs: Vec<Cow<str>> = from_str(text).unwrap_or_default();
    assert!(matches!(&strs[0], Cow::Borrowed("plain")));
    assert!(matches!(&strs[1], Cow::Owned(s) if s == "esc\n"));
    let first = strs[0].as_ptr();
    assert!(text.as_bytes().as_ptr_range().contains(&first));
    assert_eq!(
        from_str::<Vec<&str>>(r#"["a", "b"]"#).ok(),
        Some(vec!["a", "b"])
    );
    let err = from_str::<Vec<&str>>(r#"["a", "\n"]"#).err();
    assert_eq!(
        err.as_ref().map(|e| e.kind()),
        Some(&DecodeErrorKind::StringNotBorrowable)
    );
    assert_eq!(err.map(|e| e.pointer()).as_deref(), Some("/1"));
    assert_eq!(from_str::<Box<str>>("\"x\"").ok().as_deref(), Some("x"));
    assert_eq!(from_str::<String>("\"x\"").ok().as_deref(), Some("x"));
}

#[test]
fn test_containers_pointers_and_collections() {
    assert_eq!(from_str::<Option<u8>>("null").ok(), Some(None));
    assert_eq!(from_str::<Option<u8>>("3").ok(), Some(Some(3)));
    assert_eq!(from_str::<Box<u8>>("3").ok().map(|b| *b), Some(3));
    assert_eq!(from_str::<Rc<u8>>("3").ok().map(|b| *b), Some(3));
    assert_eq!(from_str::<Arc<u8>>("3").ok().map(|b| *b), Some(3));
    assert_eq!(
        <Box<Option<u8>> as FromJson>::absent(),
        Some(Box::new(None))
    );
    assert_eq!(
        <Rc<Option<u8>> as FromJson>::absent().map(|r| *r),
        Some(None)
    );
    assert_eq!(
        <Arc<Option<u8>> as FromJson>::absent().map(|r| *r),
        Some(None)
    );
    assert_eq!(
        from_str::<Box<[u8]>>("[1,2]").ok().as_deref(),
        Some(&[1u8, 2][..])
    );
    assert_eq!(
        from_str::<VecDeque<u8>>("[1]").ok(),
        Some(VecDeque::from([1]))
    );
    assert_eq!(
        from_str::<BTreeSet<u8>>("[2, 1, 2]").ok(),
        Some(BTreeSet::from([1, 2]))
    );
    assert_eq!(from_str::<[u8; 3]>("[1, 2, 3]").ok(), Some([1, 2, 3]));
    assert_eq!(
        kind_of(from_str::<[u8; 3]>("[1, 2]")),
        Some(DecodeErrorKind::WrongLength {
            expected: 3,
            found: 2
        })
    );
    assert_eq!(
        kind_of(from_str::<[u8; 1]>("[1, 2, 3]")),
        Some(DecodeErrorKind::WrongLength {
            expected: 1,
            found: 3
        })
    );
    assert_eq!(
        from_str::<(u8, String, bool)>(r#"[1, "a", true]"#).ok(),
        Some((1, "a".into(), true))
    );
    assert_eq!(
        kind_of(from_str::<(u8, u8)>("[1]")),
        Some(DecodeErrorKind::WrongLength {
            expected: 2,
            found: 1
        })
    );
    assert_eq!(
        kind_of(from_str::<(u8,)>("[1, 2]")),
        Some(DecodeErrorKind::WrongLength {
            expected: 1,
            found: 2
        })
    );
    type Eight = (u8, u8, u8, u8, u8, u8, u8, u8);
    let eight: Result<Eight, _> = from_str("[1,2,3,4,5,6,7,8]");
    assert_eq!(eight.ok(), Some((1, 2, 3, 4, 5, 6, 7, 8)));
    assert_eq!(
        kind_of(from_str::<Vec<u8>>("{}")),
        Some(DecodeErrorKind::WrongType {
            expected: "an array",
            found: Kind::Object
        })
    );

    let borrowed: BTreeMap<&str, u8> = from_str(r#"{"k": 1}"#).unwrap_or_default();
    assert_eq!(borrowed.get("k"), Some(&1));
    let err = from_str::<BTreeMap<&str, u8>>(r#"{"k\n": 1}"#).err();
    assert_eq!(
        err.map(|e| e.kind().clone()),
        Some(DecodeErrorKind::StringNotBorrowable)
    );
    let cows: BTreeMap<Cow<str>, u8> = from_str(r#"{"k": 1}"#).unwrap_or_default();
    assert_eq!(cows.len(), 1);
    let boxed: BTreeMap<Box<str>, u8> = from_str(r#"{"k": 1}"#).unwrap_or_default();
    assert_eq!(boxed.len(), 1);

    // Two spellings of one key: the reader refuses them under `Error`; a
    // map read under the other policies keeps one.
    let text = r#"{"a": 1, "\u0061": 2}"#;
    assert!(from_str::<BTreeMap<String, u8>>(text).is_err());
    let first = Parser::new()
        .duplicate_keys(DuplicateKeys::First)
        .decode::<BTreeMap<String, u8>>(text);
    assert_eq!(first.ok().and_then(|m| m.get("a").copied()), Some(1));
    let last = Parser::new()
        .duplicate_keys(DuplicateKeys::Last)
        .decode::<BTreeMap<String, u8>>(text);
    assert_eq!(last.ok().and_then(|m| m.get("a").copied()), Some(2));
}

#[cfg(feature = "std")]
#[test]
fn test_std_collections_read_and_follow_the_policy() {
    use std::collections::{HashMap, HashSet};

    assert_eq!(
        from_str::<HashSet<u8>>("[2, 2]").ok().map(|s| s.len()),
        Some(1)
    );
    let map: HashMap<String, Vec<u8>> = from_str(r#"{"a": [1], "b": []}"#).unwrap_or_default();
    assert_eq!(map.len(), 2);
    let text = r#"{"a": 1, "\u0061": 2}"#;
    let last = Parser::new()
        .duplicate_keys(DuplicateKeys::Last)
        .decode::<HashMap<String, u8>>(text);
    assert_eq!(last.ok().and_then(|m| m.get("a").copied()), Some(2));
    let refused = Parser::new()
        .duplicate_keys(DuplicateKeys::KeepAll)
        .decode::<HashMap<String, u8>>(text);
    assert_eq!(refused.ok().map(|m| m.len()), Some(1));
}

#[test]
fn test_crate_types_read_as_themselves() {
    let v: Value = from_str(r#"{"a": [1, "x"]}"#).unwrap_or_default();
    assert_eq!(v["a"][1], "x");
    let n: Option<Number> = from_str("12.50").ok();
    assert_eq!(n.as_ref().map(Number::as_str), Some("12.50"));
    assert_eq!(
        kind_of(from_str::<Number>("\"1\"")),
        Some(DecodeErrorKind::WrongType {
            expected: "a number",
            found: Kind::String
        })
    );
    let a: Option<Array> = from_str("[1, 2]").ok();
    assert_eq!(a.map(|a| a.len()), Some(2));
    assert_eq!(
        kind_of(from_str::<Array>("{}")),
        Some(DecodeErrorKind::WrongType {
            expected: "an array",
            found: Kind::Object
        })
    );
    let o: Option<Object> = from_str(r#"{"k": null}"#).ok();
    assert!(o.is_some_and(|o| o.contains_key("k")));
    assert_eq!(
        kind_of(from_str::<Object>("[]")),
        Some(DecodeErrorKind::WrongType {
            expected: "an object",
            found: Kind::Array
        })
    );
    // A value that is not there at all.
    let mut r = Reader::new("[]");
    let _start = r.next_event();
    assert_eq!(
        kind_of(r.decode::<Value>()),
        Some(DecodeErrorKind::Syntax(ErrorKind::ExpectedValue))
    );
    let mut r = Reader::new("[]");
    let _start = r.next_event();
    assert_eq!(
        kind_of(r.decode::<u8>()),
        Some(DecodeErrorKind::Syntax(ErrorKind::ExpectedValue))
    );
    // A truncated document.
    assert_eq!(
        kind_of(from_str::<u8>("")),
        Some(DecodeErrorKind::Syntax(ErrorKind::UnexpectedEnd))
    );
    assert_eq!(
        kind_of(from_str::<u8>("1 2")),
        Some(DecodeErrorKind::Syntax(ErrorKind::TrailingCharacters))
    );
}

#[test]
fn test_nullable_keeps_absent_null_and_value_apart() {
    struct Patch {
        name: Nullable<String>,
    }
    impl<'de> FromJson<'de> for Patch {
        fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
            let mut name = None;
            let mut o = reader.read_object()?;
            while let Some(key) = o.next_key()? {
                if key == "name" {
                    o.field(&mut name)?;
                }
            }
            Ok(Self {
                name: o.required(name, "name")?,
            })
        }
    }
    impl ToJson for Patch {
        fn to_json(&self, out: ValueWriter<'_>) -> Written {
            out.object(|o| o.optional("name", &self.name))
        }
    }
    for (text, want) in [
        ("{}", Nullable::Absent),
        (r#"{"name":null}"#, Nullable::Null),
        (r#"{"name":"x"}"#, Nullable::Value("x".to_owned())),
    ] {
        let p: Option<Patch> = from_str(text).ok();
        assert_eq!(p.as_ref().map(|p| &p.name), Some(&want));
        assert_eq!(p.map(|p| jsonx_lib::to_string(&p)).as_deref(), Some(text));
    }
    assert_eq!(jsonx_lib::to_string(&Nullable::<u8>::Absent), "null");
    assert_eq!(Nullable::Value(1).as_ref().into_option(), Some(&1));
    assert!(Nullable::<u8>::Null.is_null());
    assert_eq!(Nullable::<u8>::default(), Nullable::Absent);
}

#[test]
fn test_raw_values_are_validated_once_and_written_verbatim() {
    assert_eq!(
        RawValue::new("  [1,  2] ")
            .ok()
            .map(|r| r.as_str().to_owned())
            .as_deref(),
        Some("[1,  2]")
    );
    assert_eq!(
        RawValue::new("").err().map(|e| e.kind()),
        Some(ErrorKind::UnexpectedEnd)
    );
    assert_eq!(
        RawValue::new("   ").err().map(|e| e.kind()),
        Some(ErrorKind::UnexpectedEnd)
    );
    assert_eq!(
        RawValue::new("[1] x").err().map(|e| e.kind()),
        Some(ErrorKind::TrailingCharacters)
    );
    assert_eq!(
        RawValue::new(r#"{"a":1,"a":2}"#).err().map(|e| e.kind()),
        Some(ErrorKind::DuplicateKey)
    );
    let owned = RawValue::from_string(" 7 ".to_owned()).ok();
    assert_eq!(owned.as_ref().map(RawValue::as_str), Some("7"));
    let exact = RawValue::from_string("true".to_owned()).ok();
    assert_eq!(
        exact.as_ref().map(|r| r.to_string()).as_deref(),
        Some("true")
    );
    assert_eq!(format!("{:?}", RawValue::encode(&[1, 2])), "[1,2]");

    let text = r#"{"id": 1, "params": {"deep": [1, 2]}}"#;
    let mut r = Reader::new(text);
    let _ = r.next_event();
    let _ = r.next_event();
    let _ = r.read_raw_value();
    let _ = r.next_event();
    let params = r.read_raw_value().ok().flatten();
    assert_eq!(
        params.as_ref().map(RawValue::as_str),
        Some(r#"{"deep": [1, 2]}"#)
    );
    let params = params.unwrap_or_else(|| RawValue::encode(&()));
    assert!(
        text.as_bytes()
            .as_ptr_range()
            .contains(&params.as_str().as_ptr())
    );
    assert_eq!(
        params.parse().ok().map(|v| v["deep"][1].as_u64()),
        Some(Some(2))
    );
    let deep: Option<BTreeMap<String, Vec<u8>>> = params.decode().ok();
    assert_eq!(deep.map(|m| m["deep"].clone()), Some(vec![1, 2]));
    assert_eq!(jsonx_lib::to_string(&params), r#"{"deep": [1, 2]}"#);
    assert_eq!(params.clone().into_owned(), params);

    let from_reader: Option<RawValue> = from_str("[ 1 ]").ok();
    assert_eq!(from_reader.map(|r| r.as_str().len()), Some(5));
    let mut r = Reader::new("[]");
    let _start = r.next_event();
    assert_eq!(
        kind_of(r.decode::<RawValue>()),
        Some(DecodeErrorKind::Syntax(ErrorKind::ExpectedValue))
    );
}

#[test]
fn test_document_sequences_through_both_readers() {
    let text = "{\"n\":1}\n[2]\n\"three\" 4 null\n";
    let mut r = Reader::new(text).documents();
    let mut docs = Vec::new();
    while let Ok(Some(v)) = r.read_value() {
        docs.push(v.into_owned());
    }
    assert_eq!(docs.len(), 5);
    assert_eq!(docs[2], "three");
    let mut r = Reader::new(text).documents();
    assert!(r.read_raw().is_ok_and(|s| s == Some("{\"n\":1}")));
    assert!(r.finish().is_ok());
    // Each document is read in full before the next: an error points into it.
    let err = Reader::new("{}\n[1,]\n").documents().finish().err();
    assert_eq!(
        err.map(|e| (e.kind(), e.offset())),
        Some((ErrorKind::TrailingComma, 6))
    );
    // Concatenated without whitespace where the grammar allows it.
    let mut r = Reader::new("[1][2]{}").documents();
    let mut n = 0;
    while let Ok(Some(_)) = r.read_raw() {
        n += 1;
    }
    assert_eq!(n, 3);
    // Each document has its own element budget.
    let tight = Parser::new().limits(Limits::new().with_max_elements(2));
    let mut r = tight.reader("[1] [2] [3]").documents();
    assert!(r.read_value().is_ok() && r.read_value().is_ok() && r.read_value().is_ok());
    let mut r = tight.reader("[1, 2]").documents();
    assert_eq!(
        r.read_value().err().map(|e| e.kind()),
        Some(ErrorKind::TooManyElements)
    );

    // The incremental reader, fed a byte at a time.
    let mut s = StreamReader::new().documents();
    let mut got = Vec::new();
    for byte in text.as_bytes() {
        assert!(s.feed(std::slice::from_ref(byte)).is_ok());
        while let Ok(Step::Ready(Some(v))) = s.read_value() {
            got.push(v.into_owned());
        }
    }
    s.end_input();
    while let Ok(Step::Ready(Some(v))) = s.read_value() {
        got.push(v.into_owned());
    }
    assert_eq!(got, docs);
    assert_eq!(s.read_value().ok(), Some(Step::Ready(None)));
    assert_eq!(s.peek_kind().ok(), Some(Step::Ready(None)));

    // Typed, from the stream; offsets count from the first byte fed.
    let mut s = StreamReader::new().documents();
    assert!(s.feed(b"[1, 2]\n[3, \"x\"]\n").is_ok());
    assert_eq!(
        s.decode::<Vec<u8>>().ok(),
        Some(Step::Ready(Some(vec![1, 2])))
    );
    let err = s.decode::<Vec<u8>>().err();
    assert_eq!(err.as_ref().map(DecodeError::offset), Some(11));
    assert_eq!(err.map(|e| e.pointer()).as_deref(), Some("/1"));
}

#[test]
fn test_early_return_and_value_with_leave_the_reader_aligned() {
    // An implementation that returns as soon as it has what it wants.
    struct FirstKey(String);
    impl<'de> FromJson<'de> for FirstKey {
        fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
            let mut o = reader.read_object()?;
            let key = o.next_key()?.map(Cow::into_owned).unwrap_or_default();
            Ok(Self(key))
        }
    }
    let all: Vec<FirstKey> =
        from_str(r#"[{"a": [1, {"b": 2}], "c": 3}, {"d": 4}]"#).unwrap_or_default();
    assert_eq!(
        all.iter().map(|k| k.0.as_str()).collect::<Vec<_>>(),
        ["a", "d"]
    );

    let mut r = Reader::new(r#"{"list": [[1, 2], [3]], "n": 5}"#);
    let mut o = r.read_object().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(o.next_key().ok().flatten().as_deref(), Some("list"));
    let first = o.value_with(|r| {
        let mut rows = r.read_array()?;
        rows.next_item::<Vec<u8>>() // stops after the first row
    });
    assert_eq!(first.ok().flatten(), Some(vec![1, 2]));
    assert_eq!(o.next_key().ok().flatten().as_deref(), Some("n"));
    assert_eq!(o.raw().ok(), Some("5"));
    assert_eq!(o.next_key().ok(), Some(None));
    assert_eq!(o.next_key().ok(), Some(None));
    assert_eq!(o.offset(), 0);

    let mut r = Reader::new("[[1], [2, 3]]");
    let mut rows = r.read_array().unwrap_or_else(|e| panic!("{e}"));
    let first = rows.next_with(|r| {
        let mut items = r.read_array()?;
        items.next_item::<u8>()
    });
    assert_eq!(first.ok().flatten(), Some(Some(1)));
    assert!(rows.skip().is_ok_and(|more| more));
    assert_eq!(rows.index(), 2);
    assert_eq!(rows.offset(), 0);
    assert!(rows.end().is_ok());
}

#[test]
fn test_misplaced_calls_report_errors_instead_of_misreading() {
    // `value` with no key read: the reader is at a key, not a value.
    let mut r = Reader::new(r#"{"a": 1}"#);
    let mut o = r.read_object().unwrap_or_else(|e| panic!("{e}"));
    assert!(o.value::<u8>().is_err());
    // A raw read with no value there.
    let mut r = Reader::new(r#"{"a": 1}"#);
    let mut o = r.read_object().unwrap_or_else(|e| panic!("{e}"));
    assert!(o.raw().is_err());
    // An array reader over a reader someone else advanced.
    let mut r = Reader::new("[1}");
    let mut items = r.read_array().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(items.next_item::<u8>().ok(), Some(Some(1)));
    assert!(items.next_item::<u8>().is_err());
    // Errors from inside a skipped member keep its key.
    let mut r = Reader::new(r#"{"bad": [1,]}"#);
    let mut o = r.read_object().unwrap_or_else(|e| panic!("{e}"));
    let _ = o.next_key();
    assert_eq!(o.skip().err().map(|e| e.pointer()).as_deref(), Some("/bad"));
    let mut r = Reader::new("[[1,]]");
    let mut items = r.read_array().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        items.skip().err().map(|e| e.pointer()).as_deref(),
        Some("/0")
    );
}

#[test]
fn test_recursive_types_are_bounded_by_the_depth_budget() {
    struct Node(Vec<Node>);
    impl Node {
        fn depth(&self) -> usize {
            1 + self.0.iter().map(Node::depth).max().unwrap_or(0)
        }
    }
    impl<'de> FromJson<'de> for Node {
        fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
            reader.decode().map(Node)
        }
    }
    let ok = "[".repeat(100) + &"]".repeat(100);
    assert_eq!(from_str::<Node>(&ok).ok().map(|n| n.depth()), Some(100));
    let deep = "[".repeat(100_000) + &"]".repeat(100_000);
    let err = from_str::<Node>(&deep).err();
    assert_eq!(
        err.and_then(|e| e.syntax_error())
            .map(|e| (e.kind(), e.offset())),
        Some((ErrorKind::DepthLimitExceeded, 128))
    );
}

#[test]
fn test_from_value_and_to_value() {
    let v = jsonx_lib::json!({ "line": 1, "character": 2 });
    assert_eq!(
        jsonx_lib::from_value::<Position>(&v).ok(),
        Some(Position {
            line: 1,
            character: 2
        })
    );
    assert_eq!(
        jsonx_lib::to_value(&Position {
            line: 1,
            character: 2
        }),
        v
    );
    assert!(jsonx_lib::from_value::<Position>(&jsonx_lib::json!([1])).is_err());
}

#[test]
fn test_decode_error_api() {
    let e = DecodeError::custom(String::from("bad"), 3)
        .at_index(1)
        .at_key("k");
    assert_eq!(e.to_string(), "bad at byte 3 (at /k/1)");
    assert_eq!(e.syntax_error(), None);
    let boxed: Box<dyn std::error::Error> = Box::new(e.clone());
    assert_eq!(boxed.to_string(), e.to_string());
    assert_eq!(
        std::mem::size_of::<DecodeError>(),
        std::mem::size_of::<usize>()
    );
    for kind in [
        DecodeErrorKind::Syntax(ErrorKind::ExpectedKey),
        DecodeErrorKind::WrongType {
            expected: "x",
            found: Kind::Null,
        },
        DecodeErrorKind::OutOfRange { expected: "u8" },
        DecodeErrorKind::StringNotBorrowable,
        DecodeErrorKind::WrongLength {
            expected: 1,
            found: 2,
        },
        DecodeErrorKind::MissingField("f"),
        DecodeErrorKind::UnknownField,
        DecodeErrorKind::DuplicateField,
        DecodeErrorKind::InvalidValue("v"),
        DecodeErrorKind::Custom(Cow::Borrowed("c")),
    ] {
        assert!(!DecodeError::new(kind, 0).to_string().is_empty());
    }
}

/// A row of the typed round trip.
type Row = (i64, String, Option<bool>, BTreeMap<String, u32>, (u8, i128));

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Typed data written with `ToJson` reads back equal with `FromJson`.
    #[test]
    fn prop_typed_round_trip(data in prop::collection::vec((any::<i64>(), any::<String>(), any::<Option<bool>>(), prop::collection::btree_map(any::<String>(), any::<u32>(), 0..4), any::<(u8, i128)>()), 0..6)) {
        let text = jsonx_lib::to_string(&data);
        let back: Result<Vec<Row>, _> = from_str(&text);
        prop_assert_eq!(back.ok(), Some(data.clone()));
        let pretty = jsonx_lib::to_string_pretty(&data);
        let back: Result<Vec<Row>, _> = from_str(&pretty);
        prop_assert_eq!(back.ok(), Some(data));
    }

    /// Floats read back bit for bit.
    #[test]
    fn prop_float_round_trip(x in any::<f64>(), y in any::<f32>()) {
        prop_assume!(x.is_finite() && y.is_finite());
        prop_assert_eq!(from_str::<f64>(&jsonx_lib::to_string(&x)).ok().map(f64::to_bits), Some(x.to_bits()));
        prop_assert_eq!(from_str::<f32>(&jsonx_lib::to_string(&y)).ok().map(f32::to_bits), Some(y.to_bits()));
    }

    /// `Value` through `FromJson` is `parse`, and through `ToJson` is
    /// `to_string`.
    #[test]
    fn prop_value_through_the_traits(v in value()) {
        let text = jsonx_lib::to_string(&v);
        let p = Parser::new().limits(Limits::unlimited()).duplicate_keys(DuplicateKeys::KeepAll);
        prop_assert_eq!(p.decode::<Value>(&text).ok(), p.parse(&text).ok());
        prop_assert_eq!(jsonx_lib::to_value(&v), v.clone());
    }

    /// A sequence of documents, joined by whitespace and fed in pieces, reads
    /// back as the same documents through both readers.
    #[test]
    fn prop_document_sequences(docs in prop::collection::vec(value(), 0..5), cuts in prop::collection::vec(0usize..400, 0..6)) {
        let p = Parser::new().limits(Limits::unlimited()).duplicate_keys(DuplicateKeys::KeepAll);
        let text: String = docs.iter().map(|d| jsonx_lib::to_string(d) + "\n").collect();
        let mut r = p.reader(&text).documents();
        let mut got = Vec::new();
        while let Ok(Some(v)) = r.read_value() {
            got.push(v.into_owned());
        }
        prop_assert_eq!(&got, &docs);
        let mut s = p.stream_reader().documents();
        let mut bounds: Vec<usize> = cuts.iter().map(|&c| c.min(text.len())).collect();
        bounds.sort_unstable();
        bounds.push(text.len());
        let mut from = 0;
        let mut streamed = Vec::new();
        for to in bounds {
            prop_assert!(s.feed(text.as_bytes().get(from..to).unwrap_or_default()).is_ok());
            from = to;
            while let Ok(Step::Ready(Some(v))) = s.read_value() {
                streamed.push(v.into_owned());
            }
        }
        s.end_input();
        while let Ok(Step::Ready(Some(v))) = s.read_value() {
            streamed.push(v.into_owned());
        }
        prop_assert_eq!(streamed, docs);
    }

    /// Arbitrary bytes never panic through the typed entry points.
    #[test]
    fn prop_typed_reads_of_arbitrary_text_never_panic(text in common::strategies::soup()) {
        let _ = from_str::<Vec<Option<BTreeMap<String, (u8, f64)>>>>(&text);
        let _ = from_str::<Value>(&text);
        let _ = from_str::<RawValue>(&text);
        let _ = jsonx_lib::from_bytes::<Vec<&str>>(text.as_bytes());
    }
}
