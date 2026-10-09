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
        <strong>jsonx-lib</strong> reads and writes JSON with strict RFC 8259 conformance. The parser borrows strings and numbers straight from the input, keeps every number as its exact text, and refuses anything that is not JSON with the byte offset of the mistake. A pull reader walks a document as events and hands back any value as its raw source text, validated but never decoded. The writer is deterministic, and its escaping cannot be broken out of by any string.
    </p>
    <p>
        Budgets on depth, input size, string length, number length, and value count are checked before the memory they guard is allocated, and nothing in the crate recurses on nesting depth, so hostile input can exhaust neither memory nor the stack. It has no dependencies.
    </p>
    <br>
    <hr>
    <p>
        <strong>MSRV is 1.85+</strong> (Rust 2024 edition). <code>no_std</code>-compatible (needs only <code>alloc</code>), <code>#![forbid(unsafe_code)]</code>, zero dependencies.
    </p>
    <blockquote>
        <strong>Status: 0.2.0, pre-1.0.</strong> The public API is designed across the 0.x series and frozen at <code>1.0.0</code>, once lsp-lang and the LexerSketch API have used it end to end. See <a href="./CHANGELOG.md"><code>CHANGELOG.md</code></a> and <a href="./dev/ROADMAP.md"><code>ROADMAP</code></a>.
    </blockquote>
</div>

<hr>
<br>

## The model

Three ways in, simplest first:

- **[`parse`](./docs/API.md#parse) and [`to_string`](./docs/API.md#to_string)** (plus `parse_bytes` and `to_string_pretty`): one call each, default budgets, duplicate keys refused.
- **[`Parser`](./docs/API.md#parser)**: the same parse with your own [`Limits`](./docs/API.md#limits) and [`DuplicateKeys`](./docs/API.md#duplicatekeys) policy, defined once as a `const`.
- **[`Reader`](./docs/API.md#reader)**: the document as a stream of [`Event`](./docs/API.md#event)s, with `read_raw` to keep a value as its exact source text and `read_value` to build just one part.

A parsed [`Value`](./docs/API.md#value) borrows from its input; [`Number`](./docs/API.md#number) keeps exact text with lazy `i64`/`u64`/`f64` conversion; [`Object`](./docs/API.md#object) keeps members in document order; [`json!`](./docs/API.md#json) builds values in code.

<br>

What it guarantees, and how each guarantee is checked:

| Guarantee | How it is held |
|---|---|
| Accepts exactly RFC 8259; refuses the rest at the first byte that cannot continue a valid text. | Property tests run random token soup and mutated documents through the crate and an independent strict reference parser written from the RFC grammar: verdict, value, and exact error offset agree. 234 JSONTestSuite-style cases written as data (73 accept, 127 refuse, 34 implementation-defined) pin the kind and offset of every refusal. |
| Written output parses back to the same value. | Round-trip properties over arbitrary documents, compact and pretty, checked by both parsers. |
| No string can change the structure of the output. | Arbitrary strings (rich in quotes, backslashes, control characters, U+2028/U+2029) written as values and keys always read back, under the reference parser, as exactly themselves. |
| Budgets refuse before allocating. | A counting allocator measures peak heap growth: an 8 MiB string over a 1 KiB budget is refused with under 1 KiB allocated; over-size input with none. |
| No stack overflow, no panic. | A million levels of nesting parse, write, clone, compare, and drop on a thread with a 256 KiB stack; arbitrary bytes through every entry point never panic. |
| Duplicate keys cannot smuggle a second value. | Refused by default, compared after unescaping, in `O(n log n)`; every policy checked against a naive reference. |

<hr>
<br>

## Installation

```toml
[dependencies]
jsonx-lib = "0.2"
```

Without the standard library:

```toml
[dependencies]
jsonx-lib = { version = "0.2", default-features = false }
```

<hr>
<br>

## Quick start

```rust
use jsonx_lib::{json, parse, to_string};

let doc = parse(r#"{"name": "jsonx", "version": [0, 2, 0], "stable": false}"#)?;
assert_eq!(doc["name"], "jsonx");
assert_eq!(doc["version"][1].as_u64(), Some(2));
assert!(doc["license"].is_null()); // indexing never panics

let reply = json!({ "ok": true, "items": [1, 2, 3] });
assert_eq!(to_string(&reply), r#"{"ok":true,"items":[1,2,3]}"#);

let err = parse(r#"{"a": [1, 2,]}"#).unwrap_err();
assert_eq!(err.to_string(), "trailing comma before `}` or `]` at byte 12");
# Ok::<(), jsonx_lib::Error>(())
```

### Budgets and duplicate keys

```rust
use jsonx_lib::{DuplicateKeys, ErrorKind, Limits, Parser};

const WIRE: Parser = Parser::new()
    .limits(Limits::new().with_max_input_bytes(1 << 20).with_max_depth(32))
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
# Ok::<(), jsonx_lib::Error>(())
```

<hr>
<br>

## Performance

Latest local Criterion means (`cargo bench --bench bench`, release profile, Windows x86_64, one run on a busy machine; run-to-run noise of 10&ndash;30% was observed) on an LSP-shaped document of completion items with ranges, escaped strings, floats, and small arrays:

| Benchmark | 1 MB | 100 MB |
|---|---:|---:|
| `parse` into a `Value` (duplicate keys refused) | ~122 MiB/s | ~81 MiB/s |
| `parse` with `DuplicateKeys::KeepAll` | ~150 MiB/s | ~82 MiB/s |
| `Reader`: every event, no tree | ~372 MiB/s | ~368 MiB/s |
| `Reader::read_raw`: validate the whole document | ~454 MiB/s | ~429 MiB/s |
| `to_string` | ~548 MiB/s | ~229 MiB/s |
| `to_string_pretty` | ~291 MiB/s | ~157 MiB/s |
| Deep `clone` | ~257 MiB/s | ~110 MiB/s |

Strings (1 MB): parsing a string with few escapes ~4.7 GiB/s, with dense escapes ~187 MiB/s; `write_string` ~5.2 GiB/s and ~325 MiB/s. Numbers: `Number::from_f64` ~5.0 M/s, `as_f64` ~47 M/s, `as_i64` ~92 M/s. No comparison against other JSON crates has been run yet (planned for 0.9).

Building a tree is bound by the allocator (about eight allocations per completion item, freed again on drop); the event reader and `read_raw` show what the scanner itself costs.

<hr>
<br>

## Documentation

- [API reference](./docs/API.md): every public item, with examples.
- [Release notes for 0.2.0](./docs/release/v0.2.0.md).
- [Roadmap](./dev/ROADMAP.md) and [engineering directives](./dev/DIRECTIVES.md).

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
