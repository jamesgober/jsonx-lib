//! The public API end to end: borrowing, the reader's positioning rules, the
//! JSON-RPC envelope pattern lsp-lang needs, typed access, and the writers.

use std::borrow::Cow;

use jsonx_lib::{
    DuplicateKeys, ErrorKind, Event, Kind, Limits, Number, Object, Parser, Reader, Value, json,
};

#[test]
fn strings_borrow_unless_they_had_escapes() {
    let text = "[\"plain\", \"esc\\taped\", 12.50]";
    let v = jsonx_lib::parse(text).unwrap_or_default();
    assert!(matches!(&v[0], Value::String(Cow::Borrowed("plain"))));
    assert!(matches!(&v[1], Value::String(Cow::Owned(s)) if s == "esc\taped"));
    // The borrowed string is a slice of the input itself.
    let inner = v[0].as_str().unwrap_or_default();
    assert!(text.as_bytes().as_ptr_range().contains(&inner.as_ptr()));
    assert_eq!(v[2].as_number().map(Number::as_str), Some("12.50"));
}

#[test]
fn reader_handles_a_json_rpc_envelope_like_lsp_lang() {
    struct Envelope<'a> {
        id: Option<&'a str>,
        method: Option<Cow<'a, str>>,
        params: Option<&'a str>,
        result: Option<&'a str>,
    }

    fn envelope(body: &str) -> Result<Envelope<'_>, jsonx_lib::Error> {
        let mut r = Reader::new(body);
        let mut env = Envelope {
            id: None,
            method: None,
            params: None,
            result: None,
        };
        if r.next_event()? != Some(Event::StartObject) {
            return Err(jsonx_lib::parse("x").unwrap_err());
        }
        while let Some(Event::Key(key)) = r.next_event()? {
            match &*key {
                "id" => env.id = r.read_raw()?,
                "method" => match r.next_event()? {
                    Some(Event::String(m)) => env.method = Some(m),
                    _ => env.method = None,
                },
                "params" => env.params = r.read_raw()?,
                "result" => env.result = r.read_raw()?,
                _ => {
                    let _ignored = r.read_raw()?;
                }
            }
        }
        r.finish()?;
        Ok(env)
    }

    let req = envelope(r#"{"jsonrpc":"2.0","id":"a","method":"m","params":[1, {"x": "\n"}]}"#);
    let req = req.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(req.id, Some("\"a\""));
    assert!(matches!(req.method, Some(Cow::Borrowed("m"))));
    assert_eq!(req.params, Some(r#"[1, {"x": "\n"}]"#));
    assert_eq!(req.result, None);

    // `"result": null` is present (Some("null")), unlike an absent member.
    let resp = envelope(r#"{"id":1,"result":null}"#).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resp.result, Some("null"));
    assert_eq!(resp.id, Some("1"));

    // A broken body is refused as a whole, even after the fields were read.
    let broken = envelope(r#"{"id":1,"result":null,}"#);
    assert_eq!(
        broken.err().map(|e| e.kind()),
        Some(ErrorKind::TrailingComma)
    );
}

#[test]
fn reader_positioning_rules() {
    let mut r = Reader::new(r#"{"a": [1, 2], "b": {}}"#);
    // At a key, there is no value to read: nothing is consumed.
    assert_eq!(r.next_event().ok().flatten(), Some(Event::StartObject));
    assert_eq!(r.read_raw().ok(), Some(None));
    assert_eq!(r.read_value().ok(), Some(None));
    assert_eq!(r.next_event().ok().flatten(), Some(Event::Key("a".into())));
    assert_eq!(r.event_offset(), 1);
    assert_eq!(r.next_event().ok().flatten(), Some(Event::StartArray));
    assert_eq!(r.depth(), 2);
    assert_eq!(r.read_value().ok().flatten(), Some(json!(1)));
    assert_eq!(r.read_raw().ok().flatten(), Some("2"));
    assert_eq!(r.read_raw().ok(), Some(None)); // the array ends
    assert_eq!(r.next_event().ok().flatten(), Some(Event::EndArray));
    assert_eq!(r.next_event().ok().flatten(), Some(Event::Key("b".into())));
    assert_eq!(r.read_value().ok().flatten(), Some(json!({})));
    assert_eq!(r.next_event().ok().flatten(), Some(Event::EndObject));
    assert_eq!(r.read_raw().ok(), Some(None)); // the document ended
    assert_eq!(r.next_event().ok(), Some(None));
    assert_eq!(r.next_event().ok(), Some(None));
}

#[test]
fn reader_errors_latch_and_the_iterator_yields_them_once() {
    let mut r = Reader::new("[1 2]");
    let _start = r.next_event();
    let _one = r.next_event();
    let first = r.next_event().err();
    assert_eq!(
        first.map(|e| (e.kind(), e.offset())),
        Some((ErrorKind::ExpectedCommaOrBracket, 3))
    );
    assert_eq!(r.next_event().err(), first);
    assert_eq!(r.read_raw().err(), first);
    assert_eq!(r.read_value().err(), first);

    let items: Vec<_> = Reader::new("[1 2]").collect();
    assert_eq!(items.len(), 3);
    assert!(items.last().is_some_and(Result::is_err));

    let over = Parser::new()
        .limits(Limits::new().with_max_input_bytes(1))
        .reader("[]");
    let items: Vec<_> = over.collect();
    assert_eq!(items.len(), 1);
}

#[test]
fn read_value_applies_the_duplicate_policy() {
    let text = r#"{"x": {"k": 1, "k": 2}}"#;
    let mut r = Reader::new(text);
    let _start = r.next_event();
    let _key = r.next_event();
    assert_eq!(
        r.read_value().err().map(|e| (e.kind(), e.offset())),
        Some((ErrorKind::DuplicateKey, 15))
    );

    let mut r = Parser::new()
        .duplicate_keys(DuplicateKeys::Last)
        .reader(text);
    let _start = r.next_event();
    let _key = r.next_event();
    assert_eq!(r.read_value().ok().flatten(), Some(json!({ "k": 2 })));
}

#[test]
fn duplicate_policies_apply_at_every_depth() {
    let text = r#"[{"a": {"b": 1, "b": 2}, "a": 3}]"#;
    let first = Parser::new()
        .duplicate_keys(DuplicateKeys::First)
        .parse(text)
        .unwrap_or_default();
    assert_eq!(first.to_string(), r#"[{"a":{"b":1}}]"#);
    let last = Parser::new()
        .duplicate_keys(DuplicateKeys::Last)
        .parse(text)
        .unwrap_or_default();
    assert_eq!(last.to_string(), r#"[{"a":3}]"#);
    let all = Parser::new()
        .duplicate_keys(DuplicateKeys::KeepAll)
        .parse(text)
        .unwrap_or_default();
    assert_eq!(all.to_string(), r#"[{"a":{"b":1,"b":2},"a":3}]"#);
    // The innermost duplicate closes first, so it is the one reported.
    let err = jsonx_lib::parse(text).err().map(|e| e.offset());
    assert_eq!(err, Some(16));
}

#[test]
fn typed_access() {
    let v =
        json!({ "n": -3, "u": 18446744073709551615u64, "f": 0.25, "s": "x", "b": true, "z": null });
    assert_eq!(v["n"].as_i64(), Some(-3));
    assert_eq!(v["n"].as_u64(), None);
    assert_eq!(v["u"].as_u64(), Some(u64::MAX));
    assert_eq!(v["u"].as_i64(), None);
    assert_eq!(v["f"].as_f64(), Some(0.25));
    assert_eq!(v["f"].as_i64(), None);
    assert_eq!(v["s"].as_str(), Some("x"));
    assert_eq!(v["b"].as_bool(), Some(true));
    assert!(v["z"].is_null());
    assert!(v["missing"].is_null());
    assert_eq!(v.kind(), Kind::Object);
    assert_eq!(v["s"].kind().name(), "string");
    assert_eq!(v["n"], -3);
    assert_eq!(v["n"], -3i8);
    assert_eq!(v["u"], u64::MAX);
    assert_eq!(v["u"], usize::MAX as u64);
    assert_eq!(v["f"], 0.25);
    assert_ne!(v["s"], "y");
    assert_eq!(v["s"], String::from("x"));
    assert_eq!(Value::from(f64::NAN), Value::Null);
    assert_eq!(Value::from(Some("a")), "a");
    assert_eq!(Value::from(None::<i32>), Value::Null);
    assert_eq!(Value::from('c'), "c");
    assert_eq!(Value::from(()), Value::Null);
    assert_eq!(Value::from(1.5f32).to_string(), "1.5");
    let collected: Value = (1..=3).collect();
    assert_eq!(collected.to_string(), "[1,2,3]");
    let obj: Object = [("a", 1), ("b", 2)].into_iter().collect();
    assert_eq!(Value::from(obj).to_string(), r#"{"a":1,"b":2}"#);
}

#[test]
fn equality_is_exact() {
    assert_ne!(jsonx_lib::parse("1").ok(), jsonx_lib::parse("1.0").ok());
    assert_ne!(
        jsonx_lib::parse(r#"{"a":1,"b":2}"#).ok(),
        jsonx_lib::parse(r#"{"b":2,"a":1}"#).ok()
    );
    assert_eq!(
        jsonx_lib::parse("[1, \"\\u0041\"]").ok(),
        jsonx_lib::parse("[1,\"A\"]").ok()
    );
    // Numeric comparison is available through the accessors.
    assert_eq!(
        jsonx_lib::parse("1.0").ok().and_then(|v| v.as_f64()),
        Some(1.0)
    );
}

#[cfg(feature = "std")]
#[test]
fn writers_agree_and_propagate_io_errors() {
    let v = json!({ "list": [1, "two", null], "nested": { "ok": true } });
    let mut buf = Vec::new();
    assert!(jsonx_lib::to_writer(&mut buf, &v).is_ok());
    assert_eq!(buf, jsonx_lib::to_string(&v).as_bytes());
    let mut buf = Vec::new();
    assert!(jsonx_lib::to_writer_pretty(&mut buf, &v).is_ok());
    assert_eq!(buf, jsonx_lib::to_string_pretty(&v).as_bytes());

    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let err = jsonx_lib::to_writer(Broken, &v).err();
    assert_eq!(err.map(|e| e.to_string()), Some("disk full".to_owned()));

    // A large value goes out in chunks and arrives whole.
    let big: Value = (0..100_000).map(|i| json!({ "i": i })).collect();
    let mut buf = Vec::new();
    assert!(jsonx_lib::to_writer(&mut buf, &big).is_ok());
    assert_eq!(buf, jsonx_lib::to_string(&big).as_bytes());
}

#[test]
fn errors_display_kind_and_offset() {
    let err = jsonx_lib::parse("[1,]").err();
    assert_eq!(
        err.map(|e| e.to_string()),
        Some("trailing comma before `}` or `]` at byte 3".to_owned())
    );
    let boxed: Box<dyn std::error::Error> = Box::new(jsonx_lib::parse("").unwrap_err());
    assert_eq!(boxed.to_string(), "unexpected end of input at byte 0");
}

#[test]
fn values_outlive_their_text_when_owned() {
    let owned: Value<'static> = {
        let text = String::from("{\"k\": [\"v\\n\", 1e5]}");
        jsonx_lib::parse(&text)
            .map(Value::into_owned)
            .unwrap_or_default()
    };
    assert_eq!(owned["k"][0], "v\n");
    assert_eq!(owned["k"][1].as_f64(), Some(1e5));
}

#[test]
fn object_editing_keeps_order() {
    let mut v = jsonx_lib::parse(r#"{"c": 1, "a": 2, "b": 3}"#)
        .map(Value::into_owned)
        .unwrap_or_default();
    let obj = v.as_object_mut().unwrap_or_else(|| panic!("object"));
    assert_eq!(obj.insert("a", "new"), Some(json!(2)));
    obj.push("d", json!([]));
    assert_eq!(obj.remove("c"), Some(json!(1)));
    assert_eq!(obj.remove("nope"), None);
    assert_eq!(v.to_string(), r#"{"a":"new","b":3,"d":[]}"#);
    assert_eq!(v.pointer("/d"), Some(&json!([])));
}

#[test]
fn test_reader_loop_with_duplicate_method_is_refused_like_parse() {
    // The audit's JX-SEC-02 case: a second `method` must not slip through the
    // documented envelope loop when `parse` refuses the same text.
    let text = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","method":"shutdown","params":{}}"#;
    let parsed = jsonx_lib::parse(text).err().map(|e| (e.kind(), e.offset()));
    assert_eq!(parsed, Some((ErrorKind::DuplicateKey, 46)));

    let run = || -> Result<Option<Value<'_>>, jsonx_lib::Error> {
        let mut reader = Reader::new(text);
        let mut method = None;
        assert_eq!(reader.next_event()?, Some(Event::StartObject));
        while let Some(Event::Key(key)) = reader.next_event()? {
            match &*key {
                "method" => method = reader.read_value()?,
                _ => {
                    let _skipped = reader.read_raw()?;
                }
            }
        }
        reader.finish()?;
        Ok(method)
    };
    assert_eq!(run().err().map(|e| (e.kind(), e.offset())), parsed);

    // Inside a raw slice too, and through the incremental reader.
    let wrapped = format!("[{text}]");
    let mut r = Reader::new(&wrapped);
    let _start = r.next_event();
    assert_eq!(
        r.read_raw().err().map(|e| (e.kind(), e.offset())),
        Some((ErrorKind::DuplicateKey, 47))
    );
    let mut s = jsonx_lib::StreamReader::new();
    assert!(s.feed(text.as_bytes()).is_ok());
    assert_eq!(s.read_raw().err().map(|e| (e.kind(), e.offset())), parsed);

    // The other policies are explicit opt-ins, and pass every member through
    // the event stream.
    let mut r = Parser::new()
        .duplicate_keys(DuplicateKeys::Last)
        .reader(text);
    assert!(r.read_raw().is_ok_and(|raw| raw == Some(text)));
}
