//! Typed JSON without serde: an LSP-style request read into Rust types with
//! `FromJson` (strings borrowed from the message), a reply written with
//! `ToJson` (no tree built), and a stream of NDJSON log records read as it
//! arrives.
//!
//! Run with `cargo run --example typed_lsp`.

use std::borrow::Cow;

use jsonx_lib::{
    DecodeError, FromJson, Kind, Reader, Step, StreamReader, ToJson, ValueWriter, Written,
    json_struct,
};

/// Plain structs: one `json_struct!` line each writes both impls.
#[derive(Debug)]
struct Position {
    line: u32,
    character: u32,
}
json_struct!(Position { line, character });

#[derive(Debug)]
struct TextDocument<'a> {
    uri: Cow<'a, str>,
}
json_struct!(TextDocument<'a> { uri });

#[derive(Debug)]
struct HoverParams<'a> {
    text_document: TextDocument<'a>,
    position: Position,
}
json_struct!(HoverParams<'a> { text_document: "textDocument", position });

/// A JSON-RPC id is a number or a string: an enum, read by looking first.
#[derive(Debug)]
enum Id<'a> {
    Number(i64),
    Text(Cow<'a, str>),
}

impl<'de> FromJson<'de> for Id<'de> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        match reader.peek_kind()? {
            Some(Kind::String) => Ok(Id::Text(reader.decode()?)),
            Some(Kind::Number) => Ok(Id::Number(reader.decode()?)),
            Some(found) => Err(DecodeError::wrong_type(
                "a number or a string",
                found,
                reader.offset(),
            )),
            None => Err(reader.decode::<()>().unwrap_err()),
        }
    }
}

impl ToJson for Id<'_> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        match self {
            Id::Number(n) => out.i64(*n),
            Id::Text(s) => out.str(s),
        }
    }
}

/// The envelope, by hand: the pattern `json_struct!` writes for you.
#[derive(Debug)]
struct Request<'a> {
    id: Id<'a>,
    method: &'a str,
    params: HoverParams<'a>,
}

impl<'de> FromJson<'de> for Request<'de> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let (mut id, mut method, mut params) = (None, None, None);
        let mut object = reader.read_object()?;
        while let Some(key) = object.next_key()? {
            match &*key {
                "id" => object.field(&mut id)?,
                "method" => object.field(&mut method)?,
                "params" => object.field(&mut params)?,
                _ => {} // "jsonrpc" and anything newer: skipped, still validated
            }
        }
        Ok(Request {
            id: object.required(id, "id")?,
            method: object.required(method, "method")?,
            params: object.required(params, "params")?,
        })
    }
}

/// The reply, written straight to text.
struct Hover<'a> {
    id: &'a Id<'a>,
    markdown: String,
}

impl ToJson for Hover<'_> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.object(|o| {
            o.member("jsonrpc", "2.0");
            o.member("id", self.id);
            o.member_with("result", |w| {
                w.object(|r| {
                    r.member_with("contents", |c| {
                        c.object(|m| {
                            m.member("kind", "markdown");
                            m.member("value", &self.markdown);
                        })
                    });
                })
            });
        })
    }
}

fn main() -> Result<(), DecodeError> {
    let body = r#"{"jsonrpc": "2.0", "id": 7, "method": "textDocument/hover",
                   "params": {"textDocument": {"uri": "file:///a.rs"}, "position": {"line": 3, "character": 9}}}"#;
    let request: Request = jsonx_lib::from_str(body)?;
    println!(
        "{} at {}:{}",
        request.method, request.params.position.line, request.params.position.character
    );
    println!(
        "uri borrowed from the message: {}",
        matches!(request.params.text_document.uri, Cow::Borrowed(_))
    );

    let reply = Hover {
        id: &request.id,
        markdown: format!("`{}`", request.params.text_document.uri),
    };
    println!("{}", jsonx_lib::to_string(&reply));

    // A shape mismatch says what, where, and at which byte.
    let bad = r#"{"id": 1, "method": "m", "params": {"textDocument": {"uri": "x"}, "position": {"line": "3", "character": 0}}}"#;
    if let Err(e) = jsonx_lib::from_str::<Request>(bad) {
        println!("refused: {e}");
    }

    // A stream of NDJSON records, fed in pieces as a socket would deliver them.
    let mut stream = StreamReader::new().documents();
    for chunk in [
        &b"{\"line\": 1, \"char"[..],
        b"acter\": 2}\n{\"line\": 4,",
        b" \"character\": 0}\n",
    ] {
        stream.feed(chunk)?;
        while let Step::Ready(Some(p)) = stream.decode::<Position>()? {
            println!("position {:?}", p);
        }
    }
    stream.end_input();
    assert!(matches!(stream.decode::<Position>()?, Step::Ready(None)));
    Ok(())
}
