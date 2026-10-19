<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <b>jsonx-lib</b>
    <br>
    <sub><sup>FAST, STRICT, BUDGETED JSON</sup></sub>
</h1>

<div align="center">
    <a href="https://crates.io/crates/jsonx-lib"><img alt="Crates.io" src="https://img.shields.io/crates/v/jsonx-lib"></a>
    <a href="https://crates.io/crates/jsonx-lib"><img alt="Downloads" src="https://img.shields.io/crates/d/jsonx-lib?color=%230099ff"></a>
    <a href="https://docs.rs/jsonx-lib"><img alt="docs.rs" src="https://img.shields.io/docsrs/jsonx-lib"></a>
    <a href="https://github.com/jamesgober/jsonx-lib/actions"><img alt="CI" src="https://github.com/jamesgober/jsonx-lib/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.85%2B-blue"></a>
</div>

<br>

<div align="left">
    <p>
        <strong>jsonx-lib</strong> reads and writes JSON to the RFC 8259 grammar. The parser borrows strings and numbers straight from the input, keeps every number as its exact text, and refuses anything that is not JSON with the byte offset of the mistake. Pull and incremental readers walk a document as events, hand back any value as its raw source text, or read it straight into your own types through hand-written <code>FromJson</code> impls, borrowing strings from the input. <code>ToJson</code> writes your types straight to text, and a checked push writer streams any document out. The writers are deterministic, and their escaping cannot be broken out of by any string.
    </p>
    <p>
        Budgets on depth, input size, string length, number length, value count, and the memory one read holds (a tree or typed values, 64 MiB by default) are checked before the memory they guard is allocated, duplicate keys are refused on every way of reading, and nothing in the crate recurses on nesting depth, so a hostile document cannot make a read hold more than its budget, nor overflow the stack. It has no dependencies.
    </p>
    <br>
    <hr>
    <p>
        <strong>MSRV is 1.85+</strong> (Rust 2024 edition). <code>no_std</code>-compatible (needs only <code>alloc</code>), <code>#![forbid(unsafe_code)]</code>, zero dependencies.
    </p>
    <blockquote>
        <strong>Status: 0.5.0, pre-1.0.</strong> The public API is designed across the 0.x series and frozen at <code>1.0.0</code>, once lsp-lang and the LexerSketch API have used it end to end. See <a href="./CHANGELOG.md"><code>CHANGELOG.md</code></a> and <a href="./dev/ROADMAP.md"><code>ROADMAP</code></a>.
    </blockquote>
</div>

<hr>
<br>

## The model

The ways in, simplest first:

- **[`parse`](./docs/API.md#parse), [`from_str`](./docs/API.md#from_str), and [`to_string`](./docs/API.md#to_string)**: one call each, default budgets, duplicate keys refused.
- **[`Parser`](./docs/API.md#parser)**: the same with your own [`Limits`](./docs/API.md#limits), [`DuplicateKeys`](./docs/API.md#duplicatekeys) policy, and [`Numbers`](./docs/API.md#numbers) option, defined once as a `const`.
- **[`Reader`](./docs/API.md#reader)**: the document as a stream of [`Event`](./docs/API.md#event)s, with `read_raw` to keep a value as its exact source text, `read_value` to build one part, `decode` to read one part as a type, and `find_pointer` to go straight to one value. **[`StreamReader`](./docs/API.md#streamreader-and-step)** is the same reader fed in chunks as input arrives. Both read sequences of documents (NDJSON) on request.
- **Typed values**: [`FromJson`](./docs/API.md#typed-reading-fromjson) reads your type from a reader and [`ToJson`](./docs/API.md#typed-writing-tojson) writes it through a typed writer, with no tree in between. You implement them by hand (or let [`json_struct!`](./docs/API.md#json_struct) do it for a plain struct): no derive, no reflection. Errors say what, at which byte, and where (a JSON Pointer path).
- **[`Writer`](./docs/API.md#writer)**: a push writer, checked as it goes, into a `String`, an `io::Write`, or a `fmt::Write`, in any [`Format`](./docs/API.md#format) (indent width, HTML-safe, ASCII-only).

A parsed [`Value`](./docs/API.md#value) borrows from its input; [`Number`](./docs/API.md#number) keeps exact text with lazy conversion; [`Object`](./docs/API.md#object) keeps members in document order; [`RawValue`](./docs/API.md#rawvalue) keeps a validated value as text; [`json!`](./docs/API.md#json) builds values in code.

<br>

What it guarantees, and how each guarantee is checked:

| Guarantee | How it is held |
|---|---|
| The RFC 8259 grammar; everything outside it refused at the first byte that cannot continue a valid text. Lone surrogate escapes, and (by default) repeated keys, are refused by choice. | Property tests run random token soup and mutated documents through the crate and an independent strict reference parser written from the RFC grammar: verdict, value, and exact error offset agree. 234 JSONTestSuite-style cases written as data (73 accept, 127 refuse, 34 implementation-defined) pin the kind and offset of every refusal. |
| Every way of reading agrees: parse, events, raw slices, the incremental reader at any chunking, typed reads. | Properties compare the incremental reader with the complete one for every mix of calls and chunk boundaries, and every reading path's duplicate-key refusals with `parse`'s. |
| Duplicate keys cannot smuggle a second value. | Refused by default on every reading path, compared after unescaping, in `O(n log n)`; the documented JSON-RPC `Reader` loop sees the error at the object's `}`. Every policy is checked against a naive reference. |
| Written output parses back to the same value, in every format (for objects without repeated keys: the writers do not check keys for repeats). | Round-trip properties over arbitrary documents, compact, pretty, indented, HTML-safe, and ASCII-only, checked by both parsers; typed output matches the `Value` output it stands for. |
| No string can change the structure of the output. | Arbitrary strings (rich in quotes, backslashes, control characters, U+2028/U+2029) written as values and keys always read back, under the reference parser, as exactly themselves. |
| Budgets refuse before allocating; a read of a document, tree or typed values, holds at most about `max_memory` (64 MiB by default). | A counting allocator measures peak heap: an 8 MiB string over a 1 KiB budget is refused with under 1 KiB allocated; over-size input with none; five hostile document shapes (flat, nested, pairs, members, escapes) stay within `max_memory` under the defaults. |
| No stack overflow, no panic. | A million levels of nesting parse, write, transcode, clone, compare, and drop on a thread with a 256 KiB stack. Five cargo-fuzz targets cover every reading entry point with these oracles (no counterexample in about 4.0 M runs of the smoke pass), and the same checks run on arbitrary bytes in `cargo test` on every platform. |

<hr>
<br>

## Installation

```toml
[dependencies]
jsonx-lib = "0.5"
```

Without the standard library:

```toml
[dependencies]
jsonx-lib = { version = "0.5", default-features = false }
```

<hr>
<br>

## Quick start

```rust
use jsonx_lib::{json, parse, to_string};

let doc = parse(r#"{"name": "jsonx", "version": [0, 5, 0], "stable": false}"#)?;
assert_eq!(doc["name"], "jsonx");
assert_eq!(doc["version"][1].as_u64(), Some(5));
assert!(doc["license"].is_null()); // indexing never panics

let reply = json!({ "ok": true, "items": [1, 2, 3] });
assert_eq!(to_string(&reply), r#"{"ok":true,"items":[1,2,3]}"#);

let err = parse(r#"{"a": [1, 2,]}"#).unwrap_err();
assert_eq!(err.to_string(), "trailing comma before `}` or `]` at byte 12");
# Ok::<(), jsonx_lib::Error>(())
```

### Your own types, no serde

```rust
use std::borrow::Cow;
use jsonx_lib::{json_struct, DecodeError, FromJson, Reader, ToJson, ValueWriter, Written};

// A plain struct: one line writes both impls.
struct Position { line: u32, character: u32 }
json_struct!(Position { line, character });

// Or by hand, for full control.
struct Location<'a> { uri: Cow<'a, str>, position: Position }

impl<'de> FromJson<'de> for Location<'de> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let (mut uri, mut position) = (None, None);
        let mut object = reader.read_object()?;
        while let Some(key) = object.next_key()? {
            match &*key {
                "uri" => object.field(&mut uri)?,
                "position" => object.field(&mut position)?,
                _ => {} // unknown members are skipped, still validated
            }
        }
        Ok(Location { uri: object.required(uri, "uri")?, position: object.required(position, "position")? })
    }
}

impl ToJson for Location<'_> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.object(|o| {
            o.member("uri", &self.uri);
            o.member("position", &self.position);
        })
    }
}

let text = r#"{"uri": "file:///a.rs", "position": {"line": 3, "character": 14}}"#;
let loc: Location = jsonx_lib::from_str(text)?;
assert!(matches!(loc.uri, Cow::Borrowed(_))); // borrowed from `text`
assert_eq!(jsonx_lib::to_string(&loc), r#"{"uri":"file:///a.rs","position":{"line":3,"character":14}}"#);

let bad = r#"{"uri": "x", "position": {"line": "3", "character": 0}}"#;
let err = jsonx_lib::from_str::<Location>(bad).err().map(|e| e.to_string());
assert_eq!(err.as_deref(), Some("expected u32, found string at byte 34 (at /position/line)"));
# Ok::<(), DecodeError>(())
```

### Budgets and duplicate keys

```rust
use jsonx_lib::{DuplicateKeys, ErrorKind, Limits, Parser};

const WIRE: Parser = Parser::new()
    .limits(Limits::new().with_max_input_bytes(1 << 20).with_max_depth(32).with_max_memory(4 << 20))
    .duplicate_keys(DuplicateKeys::Error);

let err = WIRE.parse(&"[".repeat(1_000_000)).unwrap_err();
assert_eq!((err.kind(), err.offset()), (ErrorKind::DepthLimitExceeded, 32));

let err = WIRE.parse(r#"{"role": "user", "role": "admin"}"#).unwrap_err();
assert_eq!(err.kind(), ErrorKind::DuplicateKey);
```

### Reading a message without building a tree

```rust
use jsonx_lib::{Event, Reader};

let message = r#"{"jsonrpc": "2.0", "id": 4, "method": "hover", "params": {"line": 3}}"#;
let mut reader = Reader::new(message);
let (mut method, mut params) = (None, None);
assert_eq!(reader.next_event()?, Some(Event::StartObject));
while let Some(Event::Key(key)) = reader.next_event()? {
    match &*key {
        "method" => method = reader.read_value()?,
        "params" => params = reader.read_raw()?, // exact text, validated, not decoded
        _ => { let _skipped = reader.read_raw()?; }
    }
}
reader.finish()?;
assert_eq!(method.as_ref().and_then(|m| m.as_str()), Some("hover"));
assert_eq!(params, Some(r#"{"line": 3}"#));

// A repeated member never reaches the end of that loop: the reader refuses
// the object at its `}`, as `parse` does.
let mut reader = Reader::new(r#"{"method": "initialize", "method": "shutdown"}"#);
reader.next_event()?;
while let Ok(Some(Event::Key(_))) = reader.next_event() {
    reader.read_raw()?;
}
assert!(reader.finish().is_err());
# Ok::<(), jsonx_lib::Error>(())
```

### Input as it arrives, output as it goes

```rust
use jsonx_lib::{Step, StreamReader, Writer};

// NDJSON from a socket, fed in whatever pieces arrive.
let mut stream = StreamReader::new().documents();
let mut out = String::new();
let mut writer = Writer::new(&mut out);
writer.begin_array()?;
for chunk in [&b"{\"id\": 1, \"ok\": tr"[..], b"ue}\n{\"id\": 2, \"ok\": false}\n"] {
    stream.feed(chunk)?;
    while let Step::Ready(Some(record)) = stream.read_value()? {
        writer.value(&record["id"])?;
    }
}
stream.end_input();
writer.end_array()?;
writer.finish()?;
assert_eq!(out, "[1,2]");
# Ok::<(), Box<dyn std::error::Error>>(())
```

<hr>
<br>

## Performance

Measured against 0.2.0 on the same machine (Windows x86_64, Rust 1.99), with
paired samples so load hits both versions alike; time is 0.5's over 0.2's,
10th to 90th percentile:

| Input | `parse` | Reader events (duplicate keys refused) |
|---|---:|---:|
| An LSP request's params (~110 bytes) | 0.80-0.93, 5 allocations (0.2: 7) | 0.77-0.81, no allocation |
| A whole LSP request (~175 bytes) | 0.82-0.97 | 0.72-0.97 |
| A 1 MiB LSP document | 0.80-1.02 | 0.87-1.18 |

On the 1 MiB benchmark document (`cargo bench --bench bench`): `parse` about
220 MiB/s, events about 510 MiB/s, `read_raw` about 610 MiB/s, `to_string`
about 1.05 GiB/s, typed decoding into structs about 300 MiB/s, and the
incremental reader about 395 MiB/s. Typed decoding of a small request is
2.3 to 2.6 times serde_json's derive (median about 2.5) and allocates nothing. Method and every
number: [the release notes](./docs/release/v0.5.0.md#benchmarks).

<hr>
<br>

## Documentation

- [API reference](./docs/API.md): every public item, with examples, and a guide to moving a JSON-RPC server off serde_json.
- [Release notes for 0.5.0](./docs/release/v0.5.0.md) and [for 0.2.0](./docs/release/v0.2.0.md).
- [Roadmap](./dev/ROADMAP.md) and [engineering directives](./dev/DIRECTIVES.md).
- Examples: `cargo run --example quick_start`, `rpc_envelope`, `typed_lsp`.

<hr>
<br>

## Contributing

See <a href="./dev/DIRECTIVES.md"><code>dev/DIRECTIVES.md</code></a> for engineering standards and the definition of done. Before a PR: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features` must be clean.

<br>

<div id="license">
    <h2>License</h2>
    <p>Licensed under either of</p>
    <ul>
        <li><b>Apache License, Version 2.0</b> &mdash; <a href="./LICENSE-APACHE">LICENSE-APACHE</a></li>
        <li><b>MIT License</b> &mdash; <a href="./LICENSE-MIT">LICENSE-MIT</a></li>
    </ul>
    <p>at your option.</p>
</div>

<div align="center">
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2026 <strong>James Gober.</strong></sup>
</div>
