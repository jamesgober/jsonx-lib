# jsonx-lib &mdash; API Reference

> Complete reference for every public item in `jsonx-lib`, with examples.
> **Status: 0.2.0, pre-1.0.** The surface is designed across the 0.x series
> and frozen at `1.0.0`, after lsp-lang and the LexerSketch API have used it.
> See [`../dev/ROADMAP.md`](../dev/ROADMAP.md).

<sub>Copyright &copy; 2026 <strong>James Gober</strong>.</sub>

## Table of contents

- [Overview](#overview)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Concepts](#concepts)
  - [Borrowing](#borrowing)
  - [Numbers are text](#numbers-are-text)
  - [Errors and byte offsets](#errors-and-byte-offsets)
  - [Budgets](#budgets)
  - [Duplicate keys](#duplicate-keys)
  - [No recursion](#no-recursion)
- [Tier 1: functions](#tier-1-functions)
  - [`parse`](#parse) &middot; [`parse_bytes`](#parse_bytes) &middot; [`to_string`](#to_string) &middot; [`to_string_pretty`](#to_string_pretty)
- [`Parser`](#parser)
- [`Limits`](#limits)
- [`DuplicateKeys`](#duplicatekeys)
- [`Reader`](#reader)
- [`Event`](#event)
- [`Value`](#value)
- [`Kind`](#kind)
- [`Number`](#number)
- [`Array`](#array)
- [`Object`](#object), [`Iter`, `IterMut`](#iter-itermut)
- [`json!`](#json)
- [Writing](#writing): [`write_value`](#write_value), [`write_value_pretty`](#write_value_pretty), [`write_string`](#write_string), [`to_writer`](#to_writer), [`to_writer_pretty`](#to_writer_pretty), [`Display`](#display-and-debug)
- [`Error`](#error) and [`ErrorKind`](#errorkind)
- [Feature flags](#feature-flags)
- [Guide: moving a JSON-RPC server off serde_json](#guide-moving-a-json-rpc-server-off-serde_json)
- [Stability](#stability)

## Overview

`jsonx-lib` owns JSON text. It reads exactly RFC 8259, refuses everything
else with the byte offset of the mistake, borrows from the input wherever it
can, keeps numbers as their exact text, and writes deterministic output whose
escaping no string can break out of. Budgets on depth, size, string length,
number length, and value count are checked before the memory they guard is
used, and nothing in the crate recurses on nesting depth.

It does not map JSON onto your types by reflection: there is no derive.
Typed access is explicit, through accessors on [`Value`](#value) or the
[`Reader`](#reader)'s events.

Three ways in, simplest first:

| Tier | Entry point | For |
|---|---|---|
| 1 | [`parse`](#parse), [`to_string`](#to_string) | One call; default budgets, duplicate keys refused. |
| 2 | [`Parser`](#parser) | Your own [`Limits`](#limits) and [`DuplicateKeys`](#duplicatekeys) policy, kept in a `const`. |
| 3 | [`Reader`](#reader) | A document as events; raw slices of values; building only part of a document. |

## Installation

```toml
[dependencies]
jsonx-lib = "0.2"

# no_std (alloc only)
jsonx-lib = { version = "0.2", default-features = false }
```

MSRV: Rust 1.85. No dependencies.

## Quick start

```rust
use jsonx_lib::{json, parse, to_string};

let doc = parse(r#"{"name": "jsonx", "version": [0, 2, 0]}"#)?;
assert_eq!(doc["name"], "jsonx");
assert_eq!(doc["version"][1].as_u64(), Some(2));
assert!(doc["missing"].is_null()); // indexing never panics

let reply = json!({ "ok": true, "items": [1, 2, 3] });
assert_eq!(to_string(&reply), r#"{"ok":true,"items":[1,2,3]}"#);
# Ok::<(), jsonx_lib::Error>(())
```

## Concepts

### Borrowing

A parsed [`Value<'a>`](#value) borrows from the text it came from. A string
with no escape sequence is a slice of the input (`Cow::Borrowed`); a string
with escapes is decoded once into a buffer of exactly its source length
(`Cow::Owned`). Numbers are always slices. Keys follow the same rule as
strings. [`Value::into_owned`](#value) copies what is borrowed when the value
must outlive the text.

```rust
use std::borrow::Cow;
use jsonx_lib::{parse, Value};

let doc = parse(r#"["plain", "tab\there"]"#)?;
assert!(matches!(&doc[0], Value::String(Cow::Borrowed(_))));
assert!(matches!(&doc[1], Value::String(Cow::Owned(_))));
# Ok::<(), jsonx_lib::Error>(())
```

### Numbers are text

A [`Number`](#number) keeps the digits of the document. Nothing is converted
while parsing, so a 30-digit integer or a price like `19.990` round-trips
unchanged. Conversions happen when asked and never round silently into an
integer: [`as_i64`](#number) and [`as_u64`](#number) succeed only for integer
text in range; [`as_f64`](#number) is correctly rounded and `None` when the
value is not finite. Equality is by text: `1` and `1.0` are different
numbers, as they are different documents.

### Errors and byte offsets

Every failure is an [`Error`](#error): an [`ErrorKind`](#errorkind) and a byte
offset. For syntax errors the offset is the first byte at which the input
stops being the beginning of any valid JSON text: the mistake itself, not the
start of the value that contains it. Running out of input is always
[`UnexpectedEnd`](#errorkind) at the input's length. Three kinds point at the
start of something larger instead: an unpaired surrogate at the backslash of
its `\u` escape, a duplicate key at its second occurrence's opening quote,
and budget errors at the value that went over (input size: at the limit).

```rust
use jsonx_lib::{parse, ErrorKind};

let err = parse(r#"{"a": [1, 2,]}"#).unwrap_err();
assert_eq!((err.kind(), err.offset()), (ErrorKind::TrailingComma, 12));
assert_eq!(err.to_string(), "trailing comma before `}` or `]` at byte 12");
```

### Budgets

[`Limits`](#limits) bounds what a document may cost: nesting depth, input
bytes, bytes of any one string or key, bytes of any one number, and the total
count of values. Each is checked before the memory it guards is allocated: a
16 MiB string over a 1 KiB budget is refused without copying a byte, and a
string within budget is decoded into one buffer sized from its source length,
never grown. The defaults are generous for real documents and small enough
that a hostile one cannot exhaust memory.

### Duplicate keys

RFC 8259 says object names SHOULD be unique and leaves duplicates to the
implementation. Readers that disagree about them are a known way to smuggle
one value past a validator and another past the consumer, so the default
[`DuplicateKeys::Error`](#duplicatekeys) refuses them. `First`, `Last`, and
`KeepAll` are available. Keys are compared after unescaping. The check costs
at most 120 one-word comparisons for objects of up to 16 members, and a sort
(`O(n log n)`) above that.

### No recursion

Parsing, the [`Reader`](#reader), writing, `Clone`, `PartialEq`,
`into_owned`, and `Drop` never recurse once per nesting level. Drop recurses
at most a fixed 32 levels and flattens anything deeper onto an explicit stack;
everything else is a loop over an explicit stack. A million levels of nesting
are tested on a thread with a 256 KiB stack. The depth budget exists for
*your* code, which may recurse over the value.

## Tier 1: functions

### `parse`

```text
pub fn parse(text: &str) -> Result<Value<'_>, Error>
```

Parses one document with the default [`Limits`](#limits), refusing duplicate
keys. The whole input must be exactly one value with optional whitespace
around it (space, tab, line feed, carriage return; not even a byte-order
mark).

**Errors:** the first syntax error or exceeded budget; see [`ErrorKind`](#errorkind).

```rust
let v = jsonx_lib::parse(r#"{"list": [1, 2.5, "three"]}"#)?;
assert_eq!(v["list"][2], "three");
# Ok::<(), jsonx_lib::Error>(())
```

### `parse_bytes`

```text
pub fn parse_bytes(bytes: &[u8]) -> Result<Value<'_>, Error>
```

As [`parse`](#parse), for bytes that must be UTF-8 (RFC 8259 §8.1). The size
budget is checked first, then the encoding, then the syntax.

**Errors:** `InputTooLarge`; `InvalidUtf8` at the first byte of the first
invalid sequence; otherwise as `parse`.

```rust
use jsonx_lib::ErrorKind;

assert_eq!(jsonx_lib::parse_bytes(br#"{"id": 1}"#)?["id"], 1);
let err = jsonx_lib::parse_bytes(b"[\"\xff\"]").unwrap_err();
assert_eq!((err.kind(), err.offset()), (ErrorKind::InvalidUtf8, 2));
# Ok::<(), jsonx_lib::Error>(())
```

### `to_string`

```text
pub fn to_string(value: &Value<'_>) -> String
```

Compact JSON, no whitespace. Deterministic: members in order, numbers as
their exact text, strings escaped as [`write_string`](#write_string)
describes. The output parses back to an equal value.

```rust
let v = jsonx_lib::json!({ "id": 1, "tags": ["a"] });
assert_eq!(jsonx_lib::to_string(&v), r#"{"id":1,"tags":["a"]}"#);
```

### `to_string_pretty`

```text
pub fn to_string_pretty(value: &Value<'_>) -> String
```

Indented JSON: two spaces per level, one member or item per line, `": "`
after keys, `[]` and `{}` for empty containers, no trailing newline.

```rust
let v = jsonx_lib::json!({ "a": [1], "b": {} });
assert_eq!(jsonx_lib::to_string_pretty(&v), "{\n  \"a\": [\n    1\n  ],\n  \"b\": {}\n}");
```

## `Parser`

```text
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Parser { /* limits, duplicate-key policy */ }
```

A configuration: [`Limits`](#limits) and a [`DuplicateKeys`](#duplicatekeys)
policy. Small, `Copy`, and buildable in a `const`, so it is defined once and
reused for every document. [`parse`](#parse) is `Parser::new().parse`.

| Method | Description |
|---|---|
| `const fn new() -> Parser` | Default limits, `DuplicateKeys::Error`. |
| `const fn limits(self, Limits) -> Parser` | Replaces the budgets. |
| `const fn duplicate_keys(self, DuplicateKeys) -> Parser` | Sets the policy. |
| `const fn get_limits(&self) -> Limits` | The budgets. |
| `const fn get_duplicate_keys(&self) -> DuplicateKeys` | The policy. |
| `fn parse<'a>(&self, &'a str) -> Result<Value<'a>, Error>` | As [`parse`](#parse). |
| `fn parse_bytes<'a>(&self, &'a [u8]) -> Result<Value<'a>, Error>` | As [`parse_bytes`](#parse_bytes). |
| `fn reader<'a>(&self, &'a str) -> Reader<'a>` | A [`Reader`](#reader) with this configuration. |
| `fn reader_bytes<'a>(&self, &'a [u8]) -> Result<Reader<'a>, Error>` | The same over bytes; size and UTF-8 checked first. |

```rust
use jsonx_lib::{DuplicateKeys, ErrorKind, Limits, Parser};

const WIRE: Parser = Parser::new()
    .limits(Limits::new().with_max_input_bytes(1 << 20).with_max_depth(32))
    .duplicate_keys(DuplicateKeys::Error);

assert!(WIRE.parse(r#"{"ok": true}"#).is_ok());
let err = WIRE.parse(&"[".repeat(40)).unwrap_err();
assert_eq!((err.kind(), err.offset()), (ErrorKind::DepthLimitExceeded, 32));
```

## `Limits`

```text
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Limits {
    pub max_depth: usize,
    pub max_input_bytes: usize,
    pub max_string_bytes: usize,
    pub max_number_bytes: usize,
    pub max_elements: usize,
}
```

| Budget | Default | Counts | Error |
|---|---:|---|---|
| `max_depth` | 128 | nested arrays and objects; a scalar document is depth 0, `[]` is 1 | `DepthLimitExceeded` at the bracket that went too deep |
| `max_input_bytes` | 64 MiB | bytes of input | `InputTooLarge` at the limit |
| `max_string_bytes` | 16 MiB | source bytes between a string's quotes, keys included (an escape counts at its written length) | `StringTooLong` at the opening quote |
| `max_number_bytes` | 1 KiB | bytes of a number's text | `NumberTooLong` at the number's first byte |
| `max_elements` | 4 Mi | values: every scalar, array, and object counts one; keys do not | `TooManyElements` at the value that went over |

The depth default matches `serde_json`'s recursion limit. `#[non_exhaustive]`:
start from `Limits::new()`/`default()` or `Limits::unlimited()` and set fields
or chain the `with_*` methods.

| Method | Description |
|---|---|
| `const fn new() -> Limits` / `Default` | The defaults above. |
| `const fn unlimited() -> Limits` | Every budget `usize::MAX`, for trusted input (still no stack risk). |
| `const fn with_max_depth(self, usize) -> Limits` | Sets `max_depth`. |
| `const fn with_max_input_bytes(self, usize) -> Limits` | Sets `max_input_bytes`. |
| `const fn with_max_string_bytes(self, usize) -> Limits` | Sets `max_string_bytes`. |
| `const fn with_max_number_bytes(self, usize) -> Limits` | Sets `max_number_bytes`. |
| `const fn with_max_elements(self, usize) -> Limits` | Sets `max_elements`. |

```rust
use jsonx_lib::{ErrorKind, Limits, Parser};

let p = Parser::new().limits(Limits::new().with_max_elements(3));
assert!(p.parse("[1, 2]").is_ok()); // the array plus two items
assert_eq!(p.parse("[1, 2, 3]").unwrap_err().kind(), ErrorKind::TooManyElements);

let mut limits = Limits::default();
limits.max_string_bytes = 4;
assert_eq!(Parser::new().limits(limits).parse(r#""abcde""#).unwrap_err().kind(), ErrorKind::StringTooLong);
```

## `DuplicateKeys`

```text
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DuplicateKeys { Error /* default */, First, Last, KeepAll }
```

| Policy | Result for `{"a":1,"b":2,"a":3}` |
|---|---|
| `Error` | `ErrorKind::DuplicateKey` at the second `"a"`'s opening quote (offset 13). Duplicates are found when the object closes, so a syntax error later in the same object is reported first; an inner object closes, and is checked, before its parent. |
| `First` | `{"a":1,"b":2}`: the first occurrence and its value. |
| `Last` | `{"a":3,"b":2}`: the last value, at the first occurrence's position, as inserting into an ordered map would. |
| `KeepAll` | `{"a":1,"b":2,"a":3}`: every member as written; the writer reproduces them. |

```rust
use jsonx_lib::{DuplicateKeys, Parser};

let text = r#"{"a":1,"b":2,"a":3}"#;
let last = Parser::new().duplicate_keys(DuplicateKeys::Last).parse(text)?;
assert_eq!(last.to_string(), r#"{"a":3,"b":2}"#);
assert_eq!(Parser::new().parse(text).unwrap_err().offset(), 13);
# Ok::<(), jsonx_lib::Error>(())
```

## `Reader`

```text
#[derive(Clone, Debug)]
pub struct Reader<'a> { /* ... */ }
impl<'a> Iterator for Reader<'a> { type Item = Result<Event<'a>, Error>; }
```

A pull parser over a complete document in memory: the document one
[`Event`](#event) at a time, building nothing. Every parse in the crate goes
through it, so it checks the same grammar and budgets with the same errors and
offsets. It costs one byte of state per nesting level. It does not check
duplicate keys (that needs memory per object); [`read_value`](#reader) applies
the configured policy to what it builds. After an error the reader is
finished: every later call returns the same error, and the iterator yields it
once and then ends.

| Method | Description |
|---|---|
| `fn new(&'a str) -> Reader<'a>` | Default limits. For others, [`Parser::reader`](#parser). |
| `fn next_event(&mut self) -> Result<Option<Event<'a>>, Error>` | The next event; `Ok(None)` once the value is complete and only whitespace follows. |
| `fn read_raw(&mut self) -> Result<Option<&'a str>, Error>` | Consumes the next value whole and returns its exact source text, validated, with nothing decoded or allocated. |
| `fn read_value(&mut self) -> Result<Option<Value<'a>>, Error>` | Consumes the next value whole and builds it, applying the duplicate-key policy. |
| `fn finish(self) -> Result<(), Error>` | Reads and validates the rest of the document. |
| `fn depth(&self) -> usize` | Containers open. |
| `fn offset(&self) -> usize` | Bytes consumed. |
| `fn event_offset(&self) -> usize` | Where the last event's token began (a key's quote, a number's first byte, a bracket). |

**Positioning of `read_raw` and `read_value`.** Call them where a value comes
next: at the start, after a `Key`, or between array items. When no value
comes next (the current container ends, the document has ended, or a key
comes next), they return `Ok(None)` and consume nothing, so the next
`next_event` returns the end event or the key as usual.

```rust
use jsonx_lib::{Event, Reader};

let message = r#"{"jsonrpc": "2.0", "id": 4, "method": "hover", "params": {"line": 3}}"#;
let mut r = Reader::new(message);
let (mut id, mut method, mut params) = (None, None, None);
assert_eq!(r.next_event()?, Some(Event::StartObject));
while let Some(Event::Key(key)) = r.next_event()? {
    match &*key {
        "id" => id = r.read_raw()?,
        "method" => method = r.read_value()?,
        "params" => params = r.read_raw()?,
        _ => { let _skipped = r.read_raw()?; }
    }
}
r.finish()?;
assert_eq!(id, Some("4"));
assert_eq!(method.as_ref().and_then(|m| m.as_str()), Some("hover"));
assert_eq!(params, Some(r#"{"line": 3}"#));
# Ok::<(), jsonx_lib::Error>(())
```

## `Event`

```text
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event<'a> {
    StartObject, EndObject, StartArray, EndArray,
    Key(Cow<'a, str>), String(Cow<'a, str>), Number(Number<'a>), Bool(bool), Null,
}
```

A scalar value is one event; an array is `StartArray`, its items, `EndArray`;
an object is `StartObject`, then `Key` + value per member, then `EndObject`.
`Key` is returned after its `:` has been read. Strings and keys borrow unless
they had escapes.

```rust
use jsonx_lib::{Event, Reader};

let events: Vec<Event> = Reader::new(r#"{"a": [true]}"#).collect::<Result<_, _>>()?;
assert_eq!(events, [
    Event::StartObject, Event::Key("a".into()), Event::StartArray,
    Event::Bool(true), Event::EndArray, Event::EndObject,
]);
# Ok::<(), jsonx_lib::Error>(())
```

## `Value`

```text
#[derive(Default)]
pub enum Value<'a> {
    #[default] Null,
    Bool(bool),
    Number(Number<'a>),
    String(Cow<'a, str>),
    Array(Array<'a>),
    Object(Object<'a>),
}
```

Four machine words. `Clone`, `PartialEq`, `Eq`, `Debug`, `Display`, and
`Default` (`Null`). Equality is structural and exact: numbers by text, object
members in order.

**Accessors** (each `None` when the value is another kind):

| Method | Returns |
|---|---|
| `kind(&self) -> Kind` | Which of the six kinds. |
| `is_null(&self) -> bool` | Whether `null`. |
| `as_bool(&self) -> Option<bool>` | The boolean. |
| `as_number(&self) -> Option<&Number<'a>>` | The number. |
| `as_i64` / `as_u64` / `as_f64` | The number converted, see [`Number`](#number). |
| `as_str(&self) -> Option<&str>` | The string. |
| `as_array` / `as_array_mut` | The [`Array`](#array). |
| `as_object` / `as_object_mut` | The [`Object`](#object). |
| `get(&self, key: &str)` / `get_mut` | The first member named `key`, if an object. |
| `get_index(&self, i: usize)` | The item at `i`, if an array. |
| `pointer(&self, &str) -> Option<&Value<'a>>` | RFC 6901 JSON Pointer lookup (`""`, `/a/0`, `~0`, `~1`). |
| `take(&mut self) -> Value<'a>` | Moves the value out, leaving `null`. |
| `into_owned(self) -> Value<'static>` | Copies everything borrowed. |

**Indexing.** `value["key"]` and `value[3]` return `&Value`, reading as `null`
when the member or item is missing or the value is another kind. They never
panic.

**Conversions** (`From`): `bool`; every integer type up to 128 bits; `f64`
and `f32` (NaN and the infinities become `null`); `&str`, `&String`,
`String`, `Cow<str>`, `char`; `Number`, `Array`, `Object`; `Vec<T>` and
`Option<T>` for `T: Into<Value>`; `()` (null). `FromIterator<T: Into<Value>>`
collects into an array.

**Comparisons.** `Value` compares with `str`, `&str`, `String`, `bool`,
`f64` (by value), and every integer type (true when the number is an integer
equal to it).

```rust
use jsonx_lib::{parse, Value};

let doc = parse(r#"{"user": {"name": "ada", "langs": ["en", "fr"]}, "n": 7}"#)?;
assert_eq!(doc["user"]["langs"][1], "fr");
assert_eq!(doc["n"], 7);
assert_eq!(doc.pointer("/user/name").and_then(Value::as_str), Some("ada"));
assert!(doc["user"]["age"].is_null());

let owned: Value<'static> = doc.into_owned();
assert_eq!(owned.kind().name(), "object");
# Ok::<(), jsonx_lib::Error>(())
```

## `Kind`

```text
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind { Null, Bool, Number, String, Array, Object }
```

`name(self) -> &'static str` and `Display` give `null`, `boolean`, `number`,
`string`, `array`, `object`, for messages such as "expected a string, found
an array".

```rust
let v = jsonx_lib::json!([1]);
assert_eq!(format!("expected an object, found {}", v.kind()), "expected an object, found array");
```

## `Number`

```text
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Number<'a> { /* exact text */ }
```

| Method | Description |
|---|---|
| `fn from_text(&'a str) -> Option<Number<'a>>` | Exactly one RFC 8259 number, borrowed. `None` for `+1`, `01`, `.5`, `1.`, `NaN`, whitespace, `""`. |
| `fn from_f64(f64) -> Option<Number<'static>>` | Shortest text that reads back as the same `f64`; always has a fraction or exponent (`1.0`). Decimals for 1e-5 up to below 1e16, exponent form otherwise. `None` for NaN and infinities. |
| `From<i8..=i128, u8..=u128, isize, usize>` | Decimal text. |
| `fn as_str(&self) -> &str` | The exact text. |
| `fn is_integer(&self) -> bool` | No fraction and no exponent in the text. |
| `fn as_i64(&self) -> Option<i64>` | Integer text in range; nothing rounded. |
| `fn as_u64(&self) -> Option<u64>` | Integer text in range; `-0` is `0`. |
| `fn as_f64(&self) -> Option<f64>` | Correctly rounded; `None` if not finite (`1e400`); underflow is `0.0`. |
| `fn into_owned(self) -> Number<'static>` | Copies borrowed text. |

`Display` and `Debug` write the text.

```rust
use jsonx_lib::Number;

let n = Number::from_text("12345678901234567890123").unwrap();
assert_eq!(n.as_u64(), None);
assert_eq!(n.as_f64(), Some(1.2345678901234568e22));
assert_eq!(Number::from_f64(0.1 + 0.2).unwrap().as_str(), "0.30000000000000004");
assert_eq!(Number::from_f64(1e100).unwrap().as_str(), "1e100");
```

## `Array`

```text
#[derive(Default)]
pub struct Array<'a> { /* Vec<Value<'a>> */ }
impl<'a> Deref for Array<'a> { type Target = Vec<Value<'a>>; }   // and DerefMut
```

An array dereferences to `Vec<Value>`, so every slice and vector method works
on it (`len`, `iter`, `push`, indexing, `sort_by`). It is its own type so that
dropping, cloning, and comparing never recurse.

| Item | Description |
|---|---|
| `const fn new()`, `fn with_capacity(usize)` | Empty array. |
| `fn into_vec(self) -> Vec<Value<'a>>` | The items. |
| `fn into_owned(self) -> Array<'static>` | Copies everything borrowed. |
| `From<Vec<Value>>`, `FromIterator<T: Into<Value>>`, `Extend<T: Into<Value>>`, `IntoIterator` (owned, `&`, `&mut`) | Conversions. |
| `Clone`, `PartialEq`, `Eq`, `Debug`, `Default` | Iterative. |

```rust
use jsonx_lib::{Array, Value};

let mut a: Array = (1..=3).collect();
a.push(Value::from("four"));
assert_eq!(a.len(), 4);
assert_eq!(Value::Array(a).to_string(), r#"[1,2,3,"four"]"#);
```

## `Object`

```text
#[derive(Default)]
pub struct Object<'a> { /* Vec<(Cow<'a, str>, Value<'a>)> */ }
```

Members in document order. Lookup is a linear scan, which beats hashing for
the small objects JSON is mostly made of; for a large object read once,
iterate it instead of looking each key up.

| Method | Description |
|---|---|
| `const fn new()`, `fn with_capacity(usize)` | Empty object. |
| `fn len(&self)`, `fn is_empty(&self)` | Size. |
| `fn get(&self, &str) -> Option<&Value<'a>>` / `get_mut` | The first member named so. |
| `fn contains_key(&self, &str) -> bool` | Whether a member is named so. |
| `fn insert(&mut self, key, value) -> Option<Value<'a>>` | Map semantics: replaces in place and returns the old value, or appends. |
| `fn push(&mut self, key, value)` | Appends without looking (constant time; duplicates possible). |
| `fn remove(&mut self, &str) -> Option<Value<'a>>` | Removes the first member named so; order kept. |
| `fn iter(&self) -> Iter`, `fn iter_mut(&mut self) -> IterMut` | `(key, value)` pairs in order. |
| `fn keys(&self)`, `fn values(&self)` | In order. |
| `fn as_slice(&self) -> &[(Cow<'a, str>, Value<'a>)]` | The members. |
| `fn into_vec(self)`, `fn into_owned(self)` | Take the members; copy everything borrowed. |
| `Index<&str>` | The member, or `null`. |
| `From<Vec<(Cow, Value)>>`, `FromIterator<(K, V)>`, `Extend<(K, V)>`, `IntoIterator` | Append in order; duplicates kept. |
| `Clone`, `PartialEq` (members in order), `Eq`, `Debug`, `Default` | Iterative. |

```rust
use jsonx_lib::{Object, Value};

let mut o = Object::new();
o.insert("b", 1);
o.insert("a", true);
assert_eq!(o.insert("b", 2), Some(Value::from(1)));
assert_eq!(o.keys().collect::<Vec<_>>(), ["b", "a"]);
assert_eq!(Value::Object(o).to_string(), r#"{"b":2,"a":true}"#);
```

### `Iter`, `IterMut`

The iterators from `Object::iter` and `Object::iter_mut`: `(&str, &Value)`
and `(&str, &mut Value)` pairs, double-ended, exact-size, fused.

## `json!`

```text
macro_rules! json { ... }
```

Builds a [`Value`](#value) from JSON-like syntax: `null`, `true`, `false`,
`[...]`, and `{...}` nest as in JSON; anything else is a Rust expression
converted with `Value::from`. A key is a string literal or a parenthesized
expression that converts into `Cow<str>`. Members are appended in order, so a
repeated key appears twice. An expression with a top-level comma must be
parenthesized; very long literals may need a higher `recursion_limit`.

```rust
use jsonx_lib::json;

let field = "line";
let n = 3;
let v = json!({ "id": 1, "params": { (field): n, "flags": [true, null, -1.5] } });
assert_eq!(v.to_string(), r#"{"id":1,"params":{"line":3,"flags":[true,null,-1.5]}}"#);
```

## Writing

### `write_value`

`pub fn write_value(out: &mut String, value: &Value<'_>)` appends compact
JSON to `out`. Reusing one buffer saves an allocation per document.

### `write_value_pretty`

`pub fn write_value_pretty(out: &mut String, value: &Value<'_>)` appends
indented JSON (the [`to_string_pretty`](#to_string_pretty) layout).

### `write_string`

`pub fn write_string(out: &mut String, text: &str)` appends `text` as one
JSON string literal, quotes included. The escaping is complete and the same
every time:

- `"` and `\` become `\"` and `\\`;
- U+0000 to U+001F are escaped, as `\b \f \n \r \t` where JSON has a short
  form and `\u00XX` (lowercase hex) otherwise;
- U+2028 and U+2029 become `\u2028` and `\u2029`: JSON allows them raw, but
  JavaScript before ES2019 ends a line at them;
- everything else, including `/` and DEL, is copied as UTF-8.

No string can end the literal early or put a raw line break in it: the output
is always exactly one string token that decodes back to `text`
(property-tested against an independent parser). Unescaped stretches are found
eight bytes at a time and copied in bulk.

```rust
let mut out = String::from("{\"msg\":");
jsonx_lib::write_string(&mut out, "say \"hi\"\n\u{2028}");
out.push('}');
assert_eq!(out, r#"{"msg":"say \"hi\"\n\u2028"}"#);
```

### `to_writer`

`pub fn to_writer<W: std::io::Write>(writer: W, value: &Value<'_>) -> std::io::Result<()>`
(feature `std`) writes compact JSON through an 8 KiB buffer, so a large
document is never held as one string. **Errors:** any error from `writer`.

### `to_writer_pretty`

As [`to_writer`](#to_writer), with the pretty layout.

```rust
let mut out = Vec::new();
jsonx_lib::to_writer(&mut out, &jsonx_lib::json!({ "ok": true }))?;
assert_eq!(out, br#"{"ok":true}"#);
# Ok::<(), std::io::Error>(())
```

### `Display` and `Debug`

`Value` implements `Display` as compact JSON and `{:#}` as pretty JSON, so
`value.to_string()` is [`to_string`](#to_string). `Debug` for `Value` is the
compact JSON too; `Array` and `Object` debug as a list and a map of values.

## `Error`

```text
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Error { /* kind, offset */ }
```

| Method | Description |
|---|---|
| `const fn kind(&self) -> ErrorKind` | What went wrong. |
| `const fn offset(&self) -> usize` | The byte offset (see [Errors and byte offsets](#errors-and-byte-offsets)). |
| `Display` | `"<message> at byte <offset>"`. |
| `core::error::Error` | Implemented. |

## `ErrorKind`

`#[non_exhaustive]`: match with a wildcard arm. `is_budget(self) -> bool`
tells the five budget kinds (the same input may parse with larger limits)
from malformed input.

| Kind | Meaning | Offset |
|---|---|---|
| `UnexpectedEnd` | The input ended inside a value, or held no value. | The input's length. |
| `ExpectedValue` | This byte cannot start a value. | The byte. |
| `ExpectedColon` | A key must be followed by `:`. | The byte. |
| `ExpectedCommaOrBracket` | In an array, `,` or `]` must follow a value. | The byte. |
| `ExpectedCommaOrBrace` | In an object, `,` or `}` must follow a member. | The byte. |
| `ExpectedKey` | A member must start with a string. | The byte. |
| `TrailingComma` | `,` directly before the `]` or `}` that closes its container. | The `]` or `}`. |
| `TrailingCharacters` | Non-whitespace after the document's value. | The first such byte. |
| `InvalidLiteral` | Started `true`, `false`, or `null` but did not finish it. | The first wrong byte. |
| `InvalidNumber` | Broke the number grammar (leading zero, missing digit). | The offending byte. |
| `InvalidEscape` | A backslash followed by something other than `" \ / b f n r t u`. | The byte after the backslash. |
| `InvalidUnicodeEscape` | `\u` not followed by four hex digits. | The first non-hex byte. |
| `LoneSurrogate` | A `\u` escape names half a surrogate pair without the other half. | The unpaired escape's backslash. |
| `ControlCharacter` | An unescaped U+0000 to U+001F in a string. | The byte. |
| `InvalidUtf8` | Not UTF-8 (`parse_bytes`, `reader_bytes`). | The first byte of the invalid sequence. |
| `DuplicateKey` | A repeated key under `DuplicateKeys::Error`. | The second occurrence's opening quote. |
| `DepthLimitExceeded` | Nested deeper than `max_depth`. | The `[` or `{` that went too deep. |
| `InputTooLarge` | Longer than `max_input_bytes`. | The limit. |
| `StringTooLong` | A string or key over `max_string_bytes`. | Its opening quote. |
| `NumberTooLong` | A number over `max_number_bytes`. | Its first byte. |
| `TooManyElements` | More values than `max_elements`. | The value that went over. |

```rust
use jsonx_lib::{parse, ErrorKind};

match parse("[1, 2").unwrap_err().kind() {
    ErrorKind::UnexpectedEnd => { /* truncated: wait for more input */ }
    kind if kind.is_budget() => { /* valid, but over a limit */ }
    _ => { /* malformed */ }
}
```

## Feature flags

| Flag | Default | Adds |
|---|---|---|
| `std` | yes | [`to_writer`](#to_writer), [`to_writer_pretty`](#to_writer_pretty). Without it the crate is `no_std` and needs only `alloc`. |

## Guide: moving a JSON-RPC server off serde_json

The serde_json features a language server leans on, and their counterparts:

| serde_json | jsonx-lib |
|---|---|
| `serde_json::from_str::<Value>(s)` | [`parse(s)`](#parse) (borrowing; `into_owned()` to keep) |
| `&RawValue` fields (keep `params` unparsed) | [`Reader::read_raw`](#reader): the exact, validated slice |
| `#[derive(Deserialize)]` envelope with `Option<&RawValue>` members | A `Reader` loop over `Key` events (see [`Reader`](#reader)); `Some("null")` for a present `null`, `None` for an absent member |
| `Cow<'a, str>` with `#[serde(borrow)]` | Strings and keys are already `Cow`, borrowed unless escaped |
| `Value`, `Map<String, Value>` | [`Value`](#value), [`Object`](#object) (ordered) |
| `json!` | [`json!`](#json) |
| `to_string`, `to_writer` | [`to_string`](#to_string), [`to_writer`](#to_writer) |
| Duplicate struct fields are errors | `DuplicateKeys::Error`, the default |
| `#[derive(Serialize)]` for replies | Build a `Value` with `json!` or `Object`, then write it |

There is no derive: reading into your own struct is a few lines of accessor
calls, and every one of them says what it does when the shape is wrong.

## Stability

Pre-1.0. The surface above may change in a 0.x minor release, recorded in the
CHANGELOG. It is frozen at `1.0.0` after a real consumer (lsp-lang, then the
LexerSketch API) has exercised it end to end. MSRV 1.85; raising it is a minor
release.
