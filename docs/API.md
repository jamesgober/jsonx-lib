# jsonx-lib &mdash; API Reference

> Complete reference for every public item in `jsonx-lib`, with examples.
> **Status: 0.5.0, pre-1.0.** The surface is designed across the 0.x series
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
  - [`parse`](#parse) &middot; [`parse_bytes`](#parse_bytes) &middot; [`from_str`](#from_str) &middot; [`from_bytes`](#from_bytes) &middot; [`from_value`, `to_value`](#from_value-to_value) &middot; [`to_string`](#to_string) &middot; [`to_string_pretty`](#to_string_pretty) &middot; [`to_string_with`](#to_string_with)
- [`Parser`](#parser)
- [`Limits`](#limits)
- [`DuplicateKeys`](#duplicatekeys)
- [`Numbers`](#numbers)
- [`Reader`](#reader)
- [`Event`](#event)
- [`StreamReader` and `Step`](#streamreader-and-step)
- [Typed reading: `FromJson`](#typed-reading-fromjson)
  - [`ObjectReader`](#objectreader) &middot; [`ArrayReader`](#arrayreader) &middot; [`FromJsonKey`](#fromjsonkey) &middot; [`Nullable`](#nullable)
  - [`DecodeError`, `DecodeErrorKind`, `PathSegment`](#decodeerror-decodeerrorkind-pathsegment)
- [Typed writing: `ToJson`](#typed-writing-tojson)
  - [`ValueWriter`](#valuewriter) &middot; [`ObjectWriter`](#objectwriter) &middot; [`ArrayWriter`](#arraywriter) &middot; [`Written`](#written) &middot; [`ToJsonKey`](#tojsonkey)
- [`json_struct!`](#json_struct)
- [`RawValue`](#rawvalue)
- [`Value`](#value)
- [`Kind`](#kind)
- [`Number`](#number)
- [`Array`](#array)
- [`Object`](#object), [`Entry`](#entry-occupiedentry-vacantentry), [`Iter`, `IterMut`](#iter-itermut)
- [`json!`](#json)
- [Writing](#writing): [`Format`](#format), [`write_value`](#write_value), [`write_string`](#write_string), [`to_writer`](#to_writer), [`Writer`](#writer), [`WriteError`](#writeerror), [`Display`](#display-and-debug)
- [`Error`](#error) and [`ErrorKind`](#errorkind)
- [Feature flags](#feature-flags)
- [Guide: moving a JSON-RPC server off serde_json](#guide-moving-a-json-rpc-server-off-serde_json)
- [Stability](#stability)

## Overview

`jsonx-lib` owns JSON text. It reads the RFC 8259 grammar, refuses
everything outside it with the byte offset of the mistake, borrows from the
input wherever it can, keeps numbers as their exact text, and writes
deterministic output whose escaping no string can break out of. Budgets on
depth, size, string length, number length, value count, and the memory a
parse holds are checked before the memory they guard is used, and nothing in
the crate recurses on nesting depth.

It does not map JSON onto your types by reflection: there is no derive.
Typed access is explicit, through [`FromJson`](#typed-reading-fromjson) and
[`ToJson`](#typed-writing-tojson) impls you write (or have
[`json_struct!`](#json_struct) write for a plain struct), through accessors
on [`Value`](#value), or through the [`Reader`](#reader)'s events.

| Tier | Entry point | For |
|---|---|---|
| 1 | [`parse`](#parse), [`from_str`](#from_str), [`to_string`](#to_string) | One call; default budgets, duplicate keys refused. |
| 2 | [`Parser`](#parser) | Your own [`Limits`](#limits), [`DuplicateKeys`](#duplicatekeys) policy, and [`Numbers`](#numbers) option, kept in a `const`. |
| 3 | [`Reader`](#reader), [`StreamReader`](#streamreader-and-step) | A document as events; raw slices; one part as a tree or a type; input arriving in chunks; sequences of documents. |
| Typed | [`FromJson`](#typed-reading-fromjson), [`ToJson`](#typed-writing-tojson) | Your types, read and written with no tree in between. |
| Push | [`Writer`](#writer) | Output event by event, checked, to a `String`, `io::Write`, or `fmt::Write`. |

## Installation

```toml
[dependencies]
jsonx-lib = "0.5"

# no_std (alloc only)
jsonx-lib = { version = "0.5", default-features = false }
```

MSRV: Rust 1.85. No dependencies.

## Quick start

```rust
use jsonx_lib::{json, json_struct, parse, to_string};

let doc = parse(r#"{"name": "jsonx", "version": [0, 5, 0]}"#)?;
assert_eq!(doc["name"], "jsonx");
assert_eq!(doc["version"][1].as_u64(), Some(5));
assert!(doc["missing"].is_null()); // indexing never panics

struct Position { line: u32, character: u32 }
json_struct!(Position { line, character });

let p: Position = jsonx_lib::from_str(r#"{"line": 3, "character": 14}"#)?;
assert_eq!(to_string(&p), r#"{"line":3,"character":14}"#);

let reply = json!({ "ok": true, "items": [1, 2, 3] });
assert_eq!(to_string(&reply), r#"{"ok":true,"items":[1,2,3]}"#);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Concepts

### Borrowing

A parsed [`Value<'a>`](#value) borrows from the text it came from. A string
with no escape sequence is a slice of the input (`Cow::Borrowed`); a string
with escapes is decoded once into a buffer sized from its escaped source
text (`Cow::Owned`), which is never grown. Numbers are always slices. Keys
follow the same rule as strings. [`Value::into_owned`](#value) copies what is
borrowed when the value must outlive the text. Typed reads borrow the same
way: a `&'de str` or `Cow<'de, str>` field points into the input.

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
integer: `as_i64`, `as_u64`, `as_i128`, and `as_u128` succeed only for
integer text in range; `as_f64` and `as_f32` are correctly rounded and `None`
when the value is not finite; `whole_value` reads any spelling of a whole
number (`1e2`, `100.0`) exactly. Equality is by text: `1` and `1.0` are
different numbers, as they are different documents. Typed reads of integer
types follow the [`Numbers`](#numbers) option.

### Errors and byte offsets

Every failure to read is an [`Error`](#error): an [`ErrorKind`](#errorkind)
and a byte offset. For syntax errors the offset is the first byte at which
the input stops being the beginning of any valid JSON text: the mistake
itself, not the start of the value that contains it. Running out of input is
always [`UnexpectedEnd`](#errorkind) at the input's length. Some kinds point
at the start of something larger instead: an unpaired surrogate at the
backslash of its `\u` escape, a duplicate key at its second occurrence's
opening quote, and budget errors at the value that went over (input size: at
the limit). [`Error::line_col`](#error) and
[`Error::utf16_position`](#error) turn the offset into a line and column, for
a person or for an LSP diagnostic.

Typed reads fail with a [`DecodeError`](#decodeerror-decodeerrorkind-pathsegment),
which adds what was expected and found, and the path to the value.

```rust
use jsonx_lib::{parse, ErrorKind};

let text = "{\"a\": [1, 2,]}";
let err = parse(text).unwrap_err();
assert_eq!((err.kind(), err.offset()), (ErrorKind::TrailingComma, 12));
assert_eq!(err.to_string(), "trailing comma before `}` or `]` at byte 12");
assert_eq!(err.line_col(text), (1, 13));
```

### Budgets

[`Limits`](#limits) bounds what a document may cost: nesting depth, input
bytes, bytes of any one string or key, bytes of any one number, the total
count of values, and the memory a parse holds for the tree it builds. Each is
checked before the memory it guards is allocated: an 8 MiB string over a
1 KiB budget is refused with under 1 KiB allocated (a counting-allocator
test checks it), and a string within budget is decoded into one buffer sized
from its source text, never grown.

A tree costs more than its text: `size_of::<Value>()` bytes per value (32
on a 64-bit target) and 56 per object member, plus decoded strings. Under
the defaults a hostile document can make one parse hold at most about 64 MiB
(`max_memory`), whatever its shape; a server that runs many parses at once
should lower `max_memory` and `max_input_bytes` to what its messages need.
Reading events or typed values builds no tree: only the reader's own state
(a byte per open container, and 16 bytes per key of each open object while
duplicates are checked).

### Duplicate keys

RFC 8259 says object names SHOULD be unique and leaves duplicates to the
implementation. Readers that disagree about them are a known way to smuggle
one value past a validator and another past the consumer, so the default
[`DuplicateKeys::Error`](#duplicatekeys) refuses them, on every way of
reading: [`parse`](#parse), a [`Reader`](#reader)'s events and raw slices,
the [`StreamReader`](#streamreader-and-step), and typed reads. The check
happens when an object closes, so a loop over a `Reader`'s events sees the
error in place of the object's `}`, before it can act on a smuggled second
value. Keys are compared after unescaping. Each key is fingerprinted as it
arrives and tested against a 64-bit filter of the object's earlier
fingerprints, which are compared one by one only when the filter says one may
match (for objects of up to 16 keys); an object whose fingerprints collide,
or with more keys, is checked by sorting (`O(n log n)`). For a message of
ordinary shape (8 open objects, 16 of their keys) this state is kept inline
in the reader, so the check allocates nothing. `First`, `Last`, and `KeepAll`
are available as opt-ins.

### No recursion

Parsing, the readers, writing (the value writers and the [`Writer`](#writer)),
`Clone`, `PartialEq`, `Hash`, `into_owned`, and `Drop` never recurse once per
nesting level. Drop recurses at most a fixed 32 levels and flattens anything
deeper onto an explicit stack; everything else is a loop over an explicit
stack. A million levels of nesting are tested on a thread with a 256 KiB
stack. The depth budget exists for code that recurses over the value:
yours, and a `FromJson` impl for a recursive type of your own, whose
recursion it bounds.

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

### `from_str`

```text
pub fn from_str<'de, T: FromJson<'de>>(text: &'de str) -> Result<T, DecodeError>
```

Reads `text` as one `T` ([`FromJson`](#typed-reading-fromjson)) with the
default budgets, duplicate keys refused. The whole text must be that one
value. `Parser::decode` is the configured form.

```rust
use std::collections::BTreeMap;

let scores: BTreeMap<String, Vec<u32>> = jsonx_lib::from_str(r#"{"ada": [3, 5]}"#)?;
assert_eq!(scores["ada"], [3, 5]);
let err = jsonx_lib::from_str::<u8>("300").unwrap_err();
assert_eq!(err.to_string(), "number out of range for u8 at byte 0");
# Ok::<(), jsonx_lib::DecodeError>(())
```

### `from_bytes`

```text
pub fn from_bytes<'de, T: FromJson<'de>>(bytes: &'de [u8]) -> Result<T, DecodeError>
```

As [`from_str`](#from_str), for bytes that must be UTF-8.

### `from_value`, `to_value`

```text
pub fn from_value<T: for<'de> FromJson<'de>>(value: &Value<'_>) -> Result<T, DecodeError>
pub fn to_value<T: ToJson + ?Sized>(value: &T) -> Value<'static>
```

Bridges between a [`Value`](#value) already built and a typed value. Both
write the value as text and read it back, so each costs a write and a scan;
`from_value` applies the default budgets, which bound a recursive type's
depth, and its error offsets point into that text (the path is what to
read). `to_value` keeps every member as written.

```rust
let v = jsonx_lib::json!({ "list": [1, 2] });
let list: Vec<u8> = jsonx_lib::from_value(&v["list"])?;
assert_eq!(list, [1, 2]);
assert_eq!(jsonx_lib::to_value(&(1, "two")), jsonx_lib::json!([1, "two"]));
# Ok::<(), jsonx_lib::DecodeError>(())
```

### `to_string`

```text
pub fn to_string<T: ToJson + ?Sized>(value: &T) -> String
```

Compact JSON, no whitespace, for a [`Value`](#value) or any
[`ToJson`](#typed-writing-tojson) type (written directly, no tree built).
Deterministic: members in order, numbers as their exact (or shortest
round-trip) text, strings escaped as [`write_string`](#write_string)
describes. For a `Value`, the output parses back to an equal value.

```rust
let v = jsonx_lib::json!({ "id": 1, "tags": ["a"] });
assert_eq!(jsonx_lib::to_string(&v), r#"{"id":1,"tags":["a"]}"#);
assert_eq!(jsonx_lib::to_string(&[Some(1.5), None]), "[1.5,null]");
```

### `to_string_pretty`

```text
pub fn to_string_pretty<T: ToJson + ?Sized>(value: &T) -> String
```

Indented JSON: two spaces per level, one member or item per line, `": "`
after keys, `[]` and `{}` for empty containers, no trailing newline.

```rust
let v = jsonx_lib::json!({ "a": [1], "b": {} });
assert_eq!(jsonx_lib::to_string_pretty(&v), "{\n  \"a\": [\n    1\n  ],\n  \"b\": {}\n}");
```

### `to_string_with`

```text
pub fn to_string_with<T: ToJson + ?Sized>(value: &T, format: Format) -> String
```

The output in a given [`Format`](#format): an indent width, HTML-safe
escaping, ASCII-only escaping.

## `Parser`

```text
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Parser { /* limits, duplicate-key policy, number option */ }
```

A configuration: [`Limits`](#limits), a [`DuplicateKeys`](#duplicatekeys)
policy, and a [`Numbers`](#numbers) option. Small, `Copy`, and buildable in a
`const`, so it is defined once and reused for every document.
[`parse`](#parse) is `Parser::new().parse`.

| Method | Description |
|---|---|
| `const fn new() -> Parser` | Default limits, `DuplicateKeys::Error`, `Numbers::IntegerText`. |
| `const fn limits(self, Limits) -> Parser` | Replaces the budgets. |
| `const fn duplicate_keys(self, DuplicateKeys) -> Parser` | Sets the policy. |
| `const fn numbers(self, Numbers) -> Parser` | Sets how typed reads take integers. |
| `const fn get_limits(&self)`, `get_duplicate_keys(&self)`, `get_numbers(&self)` | The settings. |
| `fn parse<'a>(&self, &'a str) -> Result<Value<'a>, Error>` | As [`parse`](#parse). |
| `fn parse_bytes<'a>(&self, &'a [u8]) -> Result<Value<'a>, Error>` | As [`parse_bytes`](#parse_bytes). |
| `fn decode<'de, T: FromJson<'de>>(&self, &'de str) -> Result<T, DecodeError>` | As [`from_str`](#from_str). |
| `fn decode_bytes<'de, T: FromJson<'de>>(&self, &'de [u8]) -> Result<T, DecodeError>` | As [`from_bytes`](#from_bytes). |
| `fn reader<'a>(&self, &'a str) -> Reader<'a>` | A [`Reader`](#reader) with this configuration. |
| `fn reader_bytes<'a>(&self, &'a [u8]) -> Result<Reader<'a>, Error>` | The same over bytes; size and UTF-8 checked first. |
| `fn stream_reader(&self) -> StreamReader` | A [`StreamReader`](#streamreader-and-step) with this configuration. |

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
    pub max_memory: usize,
}
```

| Budget | Default | Counts | Error |
|---|---:|---|---|
| `max_depth` | 128 | nested arrays and objects; a scalar document is depth 0, `[]` is 1 | `DepthLimitExceeded` at the bracket that went too deep |
| `max_input_bytes` | 64 MiB | bytes of input (for a `StreamReader`, all bytes fed) | `InputTooLarge` at the limit |
| `max_string_bytes` | 16 MiB | source bytes between a string's quotes, keys included (an escape counts at its written length) | `StringTooLong` at the opening quote |
| `max_number_bytes` | 1 KiB | bytes of a number's text | `NumberTooLong` at the number's first byte |
| `max_elements` | 1 Mi | values: every scalar, array, and object counts one; keys do not. Per document in a sequence | `TooManyElements` at the value that went over |
| `max_memory` | 64 MiB | heap a parse holds for its tree and working stacks (see [Budgets](#budgets)) | `MemoryLimitExceeded` at the value whose storage went over |

The depth default matches `serde_json`'s recursion limit. `max_memory`
counts buffers at their capacity, checked before a stack grows (the moment
both its old and new buffer exist included) and before each array or object
is allocated; a decoded string is counted once made, so a parse may hold up
to one string budget beyond it; the allocator's own overhead is not counted.
It applies wherever a tree is built (`parse`, `read_value`, `Value` read
through `FromJson`). `#[non_exhaustive]`: start from `Limits::new()` or
`Limits::unlimited()` and set fields or chain the `with_*` methods.

| Method | Description |
|---|---|
| `const fn new() -> Limits` / `Default` | The defaults above. |
| `const fn unlimited() -> Limits` | Every budget `usize::MAX`, for trusted input (still no stack risk). |
| `const fn with_max_depth(self, usize)`, `with_max_input_bytes`, `with_max_string_bytes`, `with_max_number_bytes`, `with_max_elements`, `with_max_memory` | Set one budget. |

```rust
use jsonx_lib::{ErrorKind, Limits, Parser};

let p = Parser::new().limits(Limits::new().with_max_elements(3));
assert!(p.parse("[1, 2]").is_ok()); // the array plus two items
assert_eq!(p.parse("[1, 2, 3]").unwrap_err().kind(), ErrorKind::TooManyElements);

let small = Parser::new().limits(Limits::new().with_max_memory(4096));
let big = format!("[{}0]", "0,".repeat(1000));
assert_eq!(small.parse(&big).unwrap_err().kind(), ErrorKind::MemoryLimitExceeded);
```

## `DuplicateKeys`

```text
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DuplicateKeys { Error /* default */, First, Last, KeepAll }
```

| Policy | Result for `{"a":1,"b":2,"a":3}` |
|---|---|
| `Error` | `ErrorKind::DuplicateKey` at the second `"a"`'s opening quote (offset 13), on every way of reading. Found when the object closes, so a syntax error earlier in the document is reported first; an inner object closes, and is checked, before its parent. |
| `First` | `{"a":1,"b":2}`: the first occurrence and its value. |
| `Last` | `{"a":3,"b":2}`: the last value, at the first occurrence's position, as inserting into an ordered map would. |
| `KeepAll` | `{"a":1,"b":2,"a":3}`: every member as written; the writer reproduces them. `Object::get` then finds the *first* (serde_json's map keeps the last). |

Under `First`, `Last`, and `KeepAll` a reader's events carry every member as
written; the trees a parse or `read_value` builds, and typed reads, apply
the policy.

```rust
use jsonx_lib::{DuplicateKeys, Parser};

let text = r#"{"a":1,"b":2,"a":3}"#;
let last = Parser::new().duplicate_keys(DuplicateKeys::Last).parse(text)?;
assert_eq!(last.to_string(), r#"{"a":3,"b":2}"#);
assert_eq!(Parser::new().parse(text).unwrap_err().offset(), 13);
# Ok::<(), jsonx_lib::Error>(())
```

## `Numbers`

```text
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Numbers { IntegerText /* default */, WholeValues }
```

How typed reads of integer types treat a number written as a fraction or
with an exponent. `IntegerText` reads integers from integer text only, so a
value meant as a float is never taken as an integer. `WholeValues` also
accepts any spelling of a whole number (`1e2`, `100.0`), read exactly
([`Number::whole_value`](#number)); a fractional value is still refused.

```rust
use jsonx_lib::{Numbers, Parser};

assert!(Parser::new().decode::<u32>("1e2").is_err());
assert_eq!(Parser::new().numbers(Numbers::WholeValues).decode::<u32>("1e2")?, 100);
# Ok::<(), jsonx_lib::DecodeError>(())
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
offsets. It costs a byte of state per nesting level and, while duplicates are
checked, 16 bytes per key of each open object. Under `DuplicateKeys::Error` it
refuses a repeated key when the object closes, in place of its `EndObject`.
After an error the reader is finished: every later call returns the same
error, and the iterator yields it once and then ends.

| Method | Description |
|---|---|
| `fn new(&'a str) -> Reader<'a>` | Default limits. For others, [`Parser::reader`](#parser). |
| `fn documents(self) -> Reader<'a>` | Read a sequence of documents (NDJSON, concatenated JSON) instead of one. |
| `fn next_event(&mut self) -> Result<Option<Event<'a>>, Error>` | The next event; `Ok(None)` once the value is complete and only whitespace follows. |
| `fn read_raw(&mut self) -> Result<Option<&'a str>, Error>` | Consumes the next value whole and returns its exact source text, validated; nothing of it decoded or copied. |
| `fn read_raw_value(&mut self) -> Result<Option<RawValue<'a>>, Error>` | The same, as a [`RawValue`](#rawvalue) (validated by this read, not scanned again). |
| `fn read_value(&mut self) -> Result<Option<Value<'a>>, Error>` | Consumes the next value whole and builds it, applying the duplicate-key policy and the memory budget. |
| `fn decode<T: FromJson<'a>>(&mut self) -> Result<T, DecodeError>` | Reads the next value as a `T`. |
| `fn read_object(&mut self) -> Result<ObjectReader<'_, 'a>, DecodeError>` | Starts reading an object member by member ([`ObjectReader`](#objectreader)). |
| `fn read_array(&mut self) -> Result<ArrayReader<'_, 'a>, DecodeError>` | Starts reading an array item by item ([`ArrayReader`](#arrayreader)). |
| `fn peek_kind(&mut self) -> Result<Option<Kind>, Error>` | The next value's kind, from its first byte, consuming nothing. |
| `fn find_pointer(&mut self, &str) -> Result<Option<&'a str>, Error>` | The raw text of the value at a JSON Pointer, building no tree; reads only as far as the target. |
| `fn finish(self) -> Result<(), Error>` | Reads and validates the rest of the document. |
| `fn depth(&self)`, `fn offset(&self)`, `fn event_offset(&self)` | Containers open; bytes consumed; where the last event's token began. |

**Positioning of the whole-value reads.** Call `read_raw`, `read_value`,
`peek_kind`, and the others where a value comes next: at the start, after a
`Key`, or between array items. When no value comes next (the current
container ends, the document has ended, or a key comes next), the
`Option`-returning ones return `Ok(None)` and consume nothing, so the next
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

// A repeated member is refused, in place of the object's `}`.
let mut r = Reader::new(r#"{"method": "a", "method": "b"}"#);
r.next_event()?;
while let Ok(Some(Event::Key(_))) = r.next_event() {
    r.read_raw()?;
}
assert!(r.finish().is_err());

// Straight to one value.
assert_eq!(Reader::new(message).find_pointer("/params/line")?, Some("3"));
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
they had escapes. [`Writer::event`](#writer) writes them back.

```rust
use jsonx_lib::{Event, Reader};

let events: Vec<Event> = Reader::new(r#"{"a": [true]}"#).collect::<Result<_, _>>()?;
assert_eq!(events, [
    Event::StartObject, Event::Key("a".into()), Event::StartArray,
    Event::Bool(true), Event::EndArray, Event::EndObject,
]);
# Ok::<(), jsonx_lib::Error>(())
```

## `StreamReader` and `Step`

```text
#[derive(Clone, Debug)]
pub struct StreamReader { /* ... */ }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub enum Step<T> { Ready(T), NeedInput }
```

The incremental reader: bytes go in with `feed` as they arrive (a chunk may
end anywhere, even inside a token or a UTF-8 character), events come out as
soon as they are complete. It is the same reader as `Reader`, over a buffer:
the same grammar, budgets, errors, and offsets (counted from the first byte
fed), whatever the chunking. A call that needs bytes not yet there returns
`Step::NeedInput`; after `end_input`, running out is `UnexpectedEnd`.
Consumed input is dropped as more is fed, and a token cut by a chunk boundary
continues where it stopped, so a string fed one byte at a time costs linear
time. Invalid UTF-8 is reported when the reader reaches it (a syntax error
before it comes first).

Events borrow the reader's buffer: drop each before the next call, and feed
after the borrow ends (`if let Step::Ready(e) = r.next_event()? { ... } else
{ r.feed(...)?; }` on Rust 2024, whose `if let` drops its temporaries before
the `else`; a `match` keeps the borrow in every arm).

| Method | Description |
|---|---|
| `fn new() -> StreamReader` / `Default` | Default limits; [`Parser::stream_reader`](#parser) for others. |
| `fn documents(self) -> StreamReader` | Read a sequence of documents. |
| `fn feed(&mut self, &[u8]) -> Result<(), Error>` | Adds input. `InputTooLarge` over `max_input_bytes` in total; `InputAfterEnd` after `end_input`. |
| `fn end_input(&mut self)` | Nothing more will be fed. |
| `fn read_from(&mut self, &mut dyn io::Read) -> io::Result<usize>` | (`std`) Reads up to 8 KiB and feeds it; at the source's end, ends the input. |
| `fn next_event(&mut self) -> Result<Step<Option<Event<'_>>>, Error>` | As `Reader::next_event`. |
| `fn read_raw(&mut self) -> Result<Step<Option<&str>>, Error>` | As `Reader::read_raw`; the value must be buffered whole, and `NeedInput` keeps the progress. |
| `fn read_value(&mut self) -> Result<Step<Option<Value<'_>>>, Error>` | As `Reader::read_value`. |
| `fn decode<'s, T: FromJson<'s>>(&'s mut self) -> Result<Step<Option<T>>, DecodeError>` | The next value as a `T`. |
| `fn peek_kind(&mut self) -> Result<Step<Option<Kind>>, Error>` | As `Reader::peek_kind`. |
| `fn depth(&self)`, `fn offset(&self)`, `fn event_offset(&self)`, `fn buffered(&self)` | As on `Reader`; and the bytes held. |
| `Step::ready(self) -> Option<T>`, `Step::is_ready(&self)`, `Step::map(self, f)` | Helpers. |

```rust
use jsonx_lib::{Step, StreamReader};

let mut r = StreamReader::new().documents();
let mut ids = Vec::new();
for chunk in [&b"{\"id\": 1}\n{\"i"[..], b"d\": 2}\n"] {
    r.feed(chunk)?;
    while let Step::Ready(Some(doc)) = r.read_value()? {
        ids.push(doc["id"].as_i64());
    }
}
r.end_input();
assert_eq!(r.read_value()?, Step::Ready(None));
assert_eq!(ids, [Some(1), Some(2)]);
# Ok::<(), jsonx_lib::Error>(())
```

## Typed reading: `FromJson`

```text
pub trait FromJson<'de>: Sized {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError>;
    fn absent() -> Option<Self> { None }
}
```

A type that reads itself from a [`Reader`](#reader), with no tree built.
`from_json` is called with the reader where a value comes next and reads
exactly that value; `'de` is the input's lifetime, so a type can borrow from
it. `absent` is the value for an object member that is missing: `None`
(absence is an error) except `Option<T>` (`Some(None)`) and
[`Nullable<T>`](#nullable) (`Some(Absent)`).

A hand impl for a struct, the pattern every object type follows:

```rust
use std::borrow::Cow;
use jsonx_lib::{DecodeError, FromJson, Reader};

struct Location<'a> { uri: Cow<'a, str>, line: u32, label: Option<String> }

impl<'de> FromJson<'de> for Location<'de> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let (mut uri, mut line, mut label) = (None, None, None);
        let mut object = reader.read_object()?;
        while let Some(key) = object.next_key()? {
            match &*key {
                "uri" => object.field(&mut uri)?,
                "line" => object.field(&mut line)?,
                "label" => object.field(&mut label)?,
                _ => {} // unknown members: skipped, still validated
            }
        }
        Ok(Location {
            uri: object.required(uri, "uri")?,
            line: object.required(line, "line")?,
            label: object.required(label, "label")?, // absent reads as None
        })
    }
}

let loc: Location = jsonx_lib::from_str(r#"{"line": 3, "uri": "file:///a.rs"}"#)?;
assert!(matches!(loc.uri, Cow::Borrowed(_)));
assert_eq!((loc.line, loc.label), (3, None));

let err = jsonx_lib::from_str::<Location>(r#"{"uri": "x", "line": "3"}"#).err();
assert_eq!(err.map(|e| e.to_string()).as_deref(), Some("expected u32, found string at byte 21 (at /line)"));
# Ok::<(), DecodeError>(())
```

What the object reader does for you: a repeated field is refused
(`DuplicateField`) by `field`, and any repeated key, known or not, by the
reader when the object closes (under the default policy); unknown members
are skipped (LSP and MCP require receivers to ignore them), or refused with
`unknown_field`; `required` turns a missing member into `MissingField` unless
the type allows absence; errors inside a member get its key in their path.
An enum with variants of different shapes looks first with
`reader.peek_kind()`. A recursive type recurses once per level, bounded by
the reader's depth budget.

**Provided implementations:** `bool`; every integer type (integer text in
range, or per [`Numbers`](#numbers)); `f32`, `f64` (any finite number);
`String`, `Box<str>`, `Cow<'de, str>`, `&'de str` (only a string without
escapes: `StringNotBorrowable` otherwise), `char` (a one-character string);
`()` (`null`); `Option<T>`, `Nullable<T>`; `Box<T>`, `Rc<T>`, `Arc<T>`;
`Vec<T>`, `VecDeque<T>`, `Box<[T]>`, `[T; N]` (exact length), `BTreeSet<T>`
and (`std`) `HashSet<T, S>` (a set keeps one of equal items); tuples of up to
eight (arrays of that length); `BTreeMap<K, V>` and (`std`) `HashMap<K, V, S>`
(keys through [`FromJsonKey`](#fromjsonkey); a key two texts make equal
follows the reader's duplicate policy); `Value`, `Number`, `Array`,
`Object`, [`RawValue`](#rawvalue).

### `ObjectReader`

From `Reader::read_object` (which consumes the `{`; `WrongType` if the value
is not an object, with nothing consumed).

| Method | Description |
|---|---|
| `fn next_key(&mut self) -> Result<Option<Cow<'de, str>>, DecodeError>` | The next member's key, or `None` after the `}`. A value left unread is skipped first. |
| `fn field<T: FromJson<'de>>(&mut self, &mut Option<T>) -> Result<(), DecodeError>` | Reads the value into the slot; refuses a second value for the same slot (under `First`: skips it; under `Last`/`KeepAll`: replaces). |
| `fn value<T: FromJson<'de>>(&mut self) -> Result<T, DecodeError>` | Reads the value as a `T`. |
| `fn value_with<T>(&mut self, impl FnOnce(&mut Reader<'de>) -> Result<T, DecodeError>) -> Result<T, DecodeError>` | Reads the value with a closure. |
| `fn skip(&mut self) -> Result<(), DecodeError>`, `fn raw(&mut self) -> Result<&'de str, DecodeError>` | Skips the value (validated), or returns its text. |
| `fn unknown_field(&self) -> DecodeError` | The error for an unknown member, for strict types. |
| `fn required<T: FromJson<'de>>(&self, Option<T>, &'static str) -> Result<T, DecodeError>` | The slot's value, or `T::absent()`, or `MissingField` at the `{`. |
| `fn offset(&self) -> usize` | The `{`. |

### `ArrayReader`

From `Reader::read_array` (consumes the `[`).

| Method | Description |
|---|---|
| `fn next_item<T: FromJson<'de>>(&mut self) -> Result<Option<T>, DecodeError>` | The next item, or `None` after the `]`; errors get the index in their path. |
| `fn next_with<T>(&mut self, impl FnOnce(&mut Reader<'de>) -> Result<T, DecodeError>) -> Result<Option<T>, DecodeError>` | The next item through a closure. |
| `fn skip(&mut self) -> Result<bool, DecodeError>` | Skips an item; `false` after the last. |
| `fn end(self) -> Result<(), DecodeError>` | `WrongLength` if items remain (for fixed lengths). |
| `fn index(&self)`, `fn offset(&self)` | Items so far; the `[`. |

A `from_json` that returns before reading all of a container leaves nothing
behind: the reader finishes the container (validating it) before the next
read.

### `FromJsonKey`

```text
pub trait FromJsonKey<'de>: Sized {
    fn from_json_key(key: Cow<'de, str>) -> Result<Self, DecodeErrorKind>;
}
```

Map keys from their decoded text. Implemented for `String`, `Box<str>`,
`Cow<'de, str>`, and `&'de str`; implement it for a key newtype.

### `Nullable`

```text
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Nullable<T> { Absent /* default */, Null, Value(T) }
```

A member that may be absent, `null`, or a value, keeping the three apart
(LSP's `key?: T | null`). Reads `null` as `Null` and an absent member as
`Absent`; writes `Absent` nowhere when written with `ObjectWriter::optional`
(and as `null` otherwise). `into_option`, `as_ref`, `is_null`.

```rust
use jsonx_lib::{json_struct, Nullable};

struct Settings { tab_size: Nullable<u8> }
json_struct!(Settings { tab_size: "tabSize" });

for text in ["{}", r#"{"tabSize":null}"#, r#"{"tabSize":4}"#] {
    let s: Settings = jsonx_lib::from_str(text)?;
    assert_eq!(jsonx_lib::to_string(&s), text);
}
# Ok::<(), jsonx_lib::DecodeError>(())
```

### `DecodeError`, `DecodeErrorKind`, `PathSegment`

```text
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub struct DecodeError { /* boxed: one pointer wide */ }

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeErrorKind {
    Syntax(ErrorKind), WrongType { expected: &'static str, found: Kind },
    OutOfRange { expected: &'static str }, StringNotBorrowable,
    WrongLength { expected: usize, found: usize }, MissingField(&'static str),
    UnknownField, DuplicateField, InvalidValue(&'static str), Custom(Cow<'static, str>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathSegment { Key(String), Index(usize) }
```

| Kind | Meaning |
|---|---|
| `Syntax(kind)` | Not JSON, or over a budget: the [`ErrorKind`](#errorkind) a parse reports. |
| `WrongType { expected, found }` | A value of the wrong kind. |
| `OutOfRange { expected }` | A number that does not fit (or, under `Numbers::IntegerText`, is not integer text for an integer type). |
| `StringNotBorrowable` | A string with escapes read as `&str`. |
| `WrongLength { expected, found }` | A fixed-size array or tuple of the wrong length. |
| `MissingField(name)` | A required member is absent; the offset is the object's `{`. |
| `UnknownField`, `DuplicateField` | The key is the path's last segment. |
| `InvalidValue(what)`, `Custom(message)` | From an implementation. |

| Method | Description |
|---|---|
| `fn new(DecodeErrorKind, usize)`, `wrong_type`, `invalid_value`, `custom`, `missing_field` | Make one (in your own impls). |
| `fn at_key(self, impl Into<String>)`, `fn at_index(self, usize)` | Put a segment in front of the path (the readers do this). |
| `fn kind(&self)`, `fn offset(&self)`, `fn path(&self) -> &[PathSegment]` | What, at which byte, where (outermost first). |
| `fn pointer(&self) -> String` | The path as an RFC 6901 JSON Pointer. |
| `fn line_col(&self, input)`, `fn utf16_position(&self, input)` | As on [`Error`](#error). |
| `fn syntax_error(&self) -> Option<Error>` | The parse error, for `Syntax`. |
| `From<Error>`, `Display` (`"<what> at byte N (at /path)"`), `core::error::Error` | |

```rust
use jsonx_lib::{DecodeErrorKind, Kind, PathSegment};

let err = jsonx_lib::from_str::<Vec<Vec<u8>>>("[[1], [2, true]]").unwrap_err();
assert_eq!(err.kind(), &DecodeErrorKind::WrongType { expected: "u8", found: Kind::Bool });
assert_eq!(err.path(), [PathSegment::Index(1), PathSegment::Index(1)]);
assert_eq!(err.to_string(), "expected u8, found boolean at byte 10 (at /1/1)");
```

## Typed writing: `ToJson`

```text
pub trait ToJson {
    fn to_json(&self, out: ValueWriter<'_>) -> Written;
    fn is_absent(&self) -> bool { false }
}
```

A type that writes itself straight into the output. Each
[`ValueWriter`](#valuewriter) method writes one value and consumes the
writer, returning the [`Written`](#written) that `to_json` must return, so an
impl that compiles writes exactly one value. Members go only into objects
(through [`ObjectWriter`](#objectwriter)), items only into arrays
([`ArrayWriter`](#arraywriter)); escaping and layout are the writer's job.
Writing never fails from the impl's point of view: an output error is kept
and returned by the call that started the write. `ToJson` is object-safe:
`&dyn ToJson` works. `is_absent` decides whether `ObjectWriter::optional`
leaves a member out: `true` for `None` and `Nullable::Absent`.

```rust
use jsonx_lib::{ToJson, ValueWriter, Written};

enum Id { Number(i64), Text(String) }
struct Reply<'a> { id: &'a Id, items: Vec<&'a str>, error: Option<String> }

impl ToJson for Id {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        match self {
            Id::Number(n) => out.i64(*n),
            Id::Text(s) => out.str(s),
        }
    }
}

impl ToJson for Reply<'_> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.object(|o| {
            o.member("jsonrpc", "2.0");
            o.member("id", self.id);
            o.member("result", &self.items);
            o.optional("error", &self.error); // left out when None
        })
    }
}

let reply = Reply { id: &Id::Number(7), items: vec!["a"], error: None };
assert_eq!(jsonx_lib::to_string(&reply), r#"{"jsonrpc":"2.0","id":7,"result":["a"]}"#);
```

**Provided implementations:** `bool`; every integer type; `f32`, `f64`
(shortest round-trip text; NaN and the infinities as `null`); `str`,
`String`, `Cow<str>`, `Box<str>`, `char`; `()`; `Option<T>`; `&T`, `&mut T`,
`Box<T>`, `Rc<T>`, `Arc<T>`; `[T]`, `[T; N]`, `Vec<T>`, `VecDeque<T>`,
`BTreeSet<T>`, (`std`) `HashSet<T, S>`; tuples of up to eight; `BTreeMap<K,
V>`, (`std`) `HashMap<K, V, S>` (keys through [`ToJsonKey`](#tojsonkey); a
`HashMap` writes in its own order); `Value`, `Number`, `Array`, `Object`,
`RawValue`, `Nullable<T>`.

### `ValueWriter`

| Method (each `self -> Written`) | Writes |
|---|---|
| `null()`, `bool(bool)` | The literal. |
| `i64`, `u64`, `i128`, `u128` | The integer in decimal. |
| `f64`, `f32` | Shortest round-trip text (`1.0`, `0.1`); NaN and the infinities as `null`. |
| `number(&Number)` | The number's exact text. |
| `str(&str)` | The string, escaped. |
| `raw(&RawValue)` | The raw text: unchanged in the plain formats; under `html_safe` / `ascii`, with those characters escaped inside its strings. |
| `value(&Value)` | A value, iteratively, in the writer's layout. |
| `json(&T)` | `T`'s own `ToJson`. |
| `object(impl FnOnce(&mut ObjectWriter<'_>))`, `array(impl FnOnce(&mut ArrayWriter<'_>))` | A container built in the closure. |

### `ObjectWriter`

`member(key, &value)` (an `Option` that is `None` writes `null`),
`optional(key, &value)` (left out when `value.is_absent()`),
`member_with(key, |w| ...)`. Members are written in the order added; add each
key once.

### `ArrayWriter`

`item(&value)`, `item_with(|w| ...)`, `items(iterator)`.

### `Written`

The proof a `ValueWriter` wrote its value; only its methods make one. Return
it from `to_json`.

### `ToJsonKey`

```text
pub trait ToJsonKey { fn to_json_key(&self) -> Cow<'_, str>; }
```

Map keys for `ToJson` on maps. Implemented for `str`, `String`, `Cow<str>`,
`Box<str>`, `char`, and `&T`.

## `json_struct!`

```text
json_struct!(Name { field, field: "jsonKey", ... });
json_struct!(Name<'a> { ... });
```

Writes the `FromJson` and `ToJson` impls of a struct whose fields map one to
one onto members: exactly the hand impls above. Keys are the field names or
the strings given. Reading: members in any order, unknown members skipped, a
repeated field refused, a missing field refused unless its type allows
absence (`Option`, `Nullable`). Writing: members in the order listed, `None`
and `Absent` fields left out. A `macro_rules!` macro over the list you write:
no proc-macro, no reflection.

```rust
use std::borrow::Cow;
use jsonx_lib::json_struct;

struct Position { line: u32, character: u32 }
json_struct!(Position { line, character });

struct TextEdit<'a> { range: [Position; 2], new_text: Cow<'a, str> }
json_struct!(TextEdit<'a> { range, new_text: "newText" });

let edit: TextEdit = jsonx_lib::from_str(
    r#"{"newText": "x", "range": [{"line": 1, "character": 0}, {"line": 1, "character": 3}]}"#,
)?;
assert_eq!(edit.new_text, "x");
assert_eq!(edit.range[1].character, 3);
# Ok::<(), jsonx_lib::DecodeError>(())
```

## `RawValue`

```text
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RawValue<'a> { /* Cow<'a, str> */ }
```

One complete JSON value kept as its exact text, with no whitespace around it:
validated once, then stored, forwarded, or read later (`serde_json`'s
`RawValue`). Every way of making one validates or takes text this crate has
just validated or written, so writing one copies the text with no second
check and the output stays valid JSON. The strict [`Format`](#format)s
apply to it too: under `html_safe` or `ascii` the characters they escape
are escaped inside the raw text's strings, so `</script>` in a forwarded
result cannot reach a page that asked for HTML-safe output. Valid means
grammatical: a `RawValue` read under `KeepAll`, or encoded from a `ToJson`
that writes a key twice, can hold a repeated key, which `decode` (default
policy) refuses and `parse` keeps.

| Item | Description |
|---|---|
| `fn new(&'a str) -> Result<RawValue<'a>, Error>` | Validates (default limits, duplicates refused) and trims surrounding whitespace. |
| `fn from_string(String) -> Result<RawValue<'static>, Error>` | The same for an owned string. |
| `fn encode<T: ToJson + ?Sized>(&T) -> RawValue<'static>` | The compact JSON of a value. |
| `Reader::read_raw_value`, `FromJson` | From a read, with no second scan. |
| `fn as_str(&self)`, `fn into_owned(self)` | The text; detached from the input. |
| `fn parse(&self) -> Result<Value<'_>, Error>` | A tree (no budgets: the text was validated; every member kept). |
| `fn decode<'s, T: FromJson<'s>>(&'s self) -> Result<T, DecodeError>` | A `T` (default budgets). |
| `ToJson`, `Display`, `Debug` | The text. |

```rust
use jsonx_lib::RawValue;

let params = RawValue::new(" [2, 3] ")?;
assert_eq!(params.as_str(), "[2, 3]");
let (a, b): (i32, i32) = params.decode()?;
assert_eq!(a + b, 5);
assert_eq!(jsonx_lib::to_string(&params), "[2, 3]");
# Ok::<(), Box<dyn std::error::Error>>(())
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

Four machine words. `Clone`, `PartialEq`, `Eq`, `Hash`, `Debug`, `Display`,
`Default` (`Null`), `FromJson`, `ToJson`. Equality is structural and exact
(numbers by text, object members in order), and `Hash` agrees with it.

**Accessors** (each `None` or `false` when the value is another kind):

| Method | Returns |
|---|---|
| `kind(&self) -> Kind` | Which of the six kinds. |
| `is_null`, `is_bool`, `is_number`, `is_string`, `is_array`, `is_object` | Whether it is that kind. |
| `as_bool(&self) -> Option<bool>` | The boolean. |
| `as_number(&self) -> Option<&Number<'a>>` | The number. |
| `as_i64` / `as_u64` / `as_i128` / `as_u128` / `as_f64` | The number converted, see [`Number`](#number). |
| `as_str(&self) -> Option<&str>` | The string. |
| `as_array` / `as_array_mut` | The [`Array`](#array). |
| `as_object` / `as_object_mut` | The [`Object`](#object). |
| `get(&self, key: &str)` / `get_mut` | The first member named `key`, if an object. |
| `get_index(&self, i: usize)` | The item at `i`, if an array. |
| `pointer(&self, &str)` / `pointer_mut` | RFC 6901 JSON Pointer lookup (`""`, `/a/0`, `~0`, `~1`). |
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

**Comparisons.** `Value` compares with `str`, `&str`, `String`, `bool`, `f64`
and `f32` (by value), and every integer type up to 128 bits (true when the
number is integer text equal to it: `1e2 == 100` is false; use
`Number::whole_value` to compare values).

```rust
use jsonx_lib::{parse, Value};

let mut doc = parse(r#"{"user": {"name": "ada", "langs": ["en", "fr"]}, "n": 7}"#)?;
assert_eq!(doc["user"]["langs"][1], "fr");
assert_eq!(doc["n"], 7);
assert_eq!(doc.pointer("/user/name").and_then(Value::as_str), Some("ada"));
assert!(doc["user"]["age"].is_null());
if let Some(n) = doc.pointer_mut("/n") {
    *n = Value::from(8);
}
let owned: Value<'static> = doc.into_owned();
assert_eq!(owned["n"], 8);
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
| `fn as_i64`, `as_u64`, `as_i128`, `as_u128` | Integer text in range; nothing rounded; `-0` is `0` for the unsigned ones. |
| `fn whole_value(&self) -> Option<i128>` | The value if it is a whole number in `i128`'s range, however written (`1e2`, `100.0`): exact decimal arithmetic. |
| `fn as_f64(&self)`, `fn as_f32(&self)` | Correctly rounded from the text; `None` if not finite; underflow is `0.0`. |
| `fn into_owned(self) -> Number<'static>` | Copies borrowed text. |

`Display` and `Debug` write the text.

```rust
use jsonx_lib::Number;

let n = Number::from_text("12345678901234567890123").unwrap();
assert_eq!(n.as_u64(), None);
assert_eq!(n.as_f64(), Some(1.2345678901234568e22));
assert_eq!(Number::from_text("2.50e1").unwrap().whole_value(), Some(25));
assert_eq!(Number::from_f64(0.1 + 0.2).unwrap().as_str(), "0.30000000000000004");
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
| `Clone`, `PartialEq`, `Eq`, `Debug`, `Default`, `FromJson`, `ToJson` | Iterative. |

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
| `fn retain(&mut self, FnMut(&str, &mut Value) -> bool)` | Keeps the members the closure says to. |
| `fn sort_keys(&mut self)` | Sorts members by key (stable). |
| `fn entry(&mut self, key) -> Entry<'_, 'a>` | The member, to read, change, or insert in one lookup. |
| `fn iter(&self) -> Iter`, `fn iter_mut(&mut self) -> IterMut` | `(key, value)` pairs in order. |
| `fn keys(&self)`, `fn values(&self)` | In order. |
| `fn as_slice(&self) -> &[(Cow<'a, str>, Value<'a>)]` | The members. |
| `fn into_vec(self)`, `fn into_owned(self)` | Take the members; copy everything borrowed. |
| `Index<&str>` | The member, or `null`. |
| `From<Vec<(Cow, Value)>>`, `FromIterator<(K, V)>`, `Extend<(K, V)>`, `IntoIterator` | Append in order; duplicates kept. |
| `Clone`, `PartialEq` (members in order), `Eq`, `Hash`, `Debug`, `Default`, `FromJson`, `ToJson` | Iterative. |

```rust
use jsonx_lib::{Object, Value};

let mut o = Object::new();
o.insert("b", 1);
o.insert("a", true);
assert_eq!(o.insert("b", 2), Some(Value::from(1)));
*o.entry("hits").or_insert(0) = Value::from(1);
o.sort_keys();
assert_eq!(o.keys().collect::<Vec<_>>(), ["a", "b", "hits"]);
```

### `Entry`, `OccupiedEntry`, `VacantEntry`

From `Object::entry`. `Entry` has `or_insert(value)`, `or_insert_with(f)`,
`or_default()` (inserts `null`), `and_modify(f)`, and `key()`.
`OccupiedEntry` has `key`, `get`, `get_mut`, `into_mut`, `insert` (returns the
old value), and `remove` (keeps the other members' order); `VacantEntry` has
`key` and `insert`.

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
parenthesized. Single-token items go up to 32 at a time and literal-keyed
members up to 16 at a time, so literals of thousands of items fit the default
`recursion_limit` (2,000 items and 1,000 members are tested); two-token items
(a negative number) go eight at a time; longer items cost a level per token,
so a literal holds a few dozen of those.

```rust
use jsonx_lib::json;

let field = "line";
let n = 3;
let v = json!({ "id": 1, "params": { (field): n, "flags": [true, null, -1.5] } });
assert_eq!(v.to_string(), r#"{"id":1,"params":{"line":3,"flags":[true,null,-1.5]}}"#);
```

## Writing

### `Format`

```text
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Format { /* layout, escaping */ }
```

| Item | Description |
|---|---|
| `const fn compact()` / `Default` | No whitespace; the minimal escaping of [`write_string`](#write_string). |
| `const fn pretty()` | The `to_string_pretty` layout. |
| `const fn with_indent(self, spaces: u8)` | The indented layout, `spaces` per level. |
| `const fn html_safe(self, bool)` | Also escape `<`, `>`, `&` (for JSON inside HTML). |
| `const fn ascii(self, bool)` | Escape every non-ASCII character (surrogate pairs above U+FFFF). |
| `const fn is_pretty(&self)` | Whether indented. |

Every format writes valid JSON that reads back as the same value.

```rust
use jsonx_lib::{json, to_string_with, Format};

let v = json!({ "html": "</script>", "word": "caf\u{e9}" });
let safe = to_string_with(&v, Format::compact().html_safe(true).ascii(true));
assert!(!safe.contains('<') && safe.is_ascii());
assert_eq!(jsonx_lib::parse(&safe)?, v);
# Ok::<(), jsonx_lib::Error>(())
```

### `write_value`

`pub fn write_value<T: ToJson + ?Sized>(out: &mut String, value: &T)` appends
compact JSON to `out`. Reusing one buffer saves an allocation per document.
`write_value_pretty` and `write_value_with(out, value, Format)` are the other
layouts.

### `write_string`

`pub fn write_string(out: &mut String, text: &str)` appends `text` as one
JSON string literal, quotes included. The escaping is complete and the same
every time:

- `"` and `\` become `\"` and `\\`;
- U+0000 to U+001F are escaped, as `\b \f \n \r \t` where JSON has a short
  form and `\u00XX` (lowercase hex) otherwise;
- U+2028 and U+2029 are escaped: JSON allows them raw, but JavaScript before
  ES2019 ends a line at them;
- everything else, including `/` and DEL, is copied as UTF-8.

No string can end the literal early or put a raw line break in it: the output
is always exactly one string token that decodes back to `text`
(property-tested against an independent parser). Unescaped stretches are
found eight or sixteen bytes at a time and copied in bulk. This escaping is
not HTML-safe (`<`, `>`, `&` are copied); see [`Format`](#format).

```rust
let mut out = String::from("{\"msg\":");
jsonx_lib::write_string(&mut out, "say \"hi\"\n\u{2028}");
out.push('}');
assert_eq!(out, "{\"msg\":\"say \\\"hi\\\"\\n\\u2028\"}");
```

### `to_writer`

`pub fn to_writer<W: std::io::Write, T: ToJson + ?Sized>(writer: W, value: &T) -> std::io::Result<()>`
(feature `std`) writes compact JSON through an 8 KiB buffer, so a large
document is never held as one string. `to_writer_pretty` and
`to_writer_with(writer, value, Format)` are the other layouts. **Errors:**
any error from `writer`.

```rust
let mut out = Vec::new();
jsonx_lib::to_writer(&mut out, &jsonx_lib::json!({ "ok": true }))?;
assert_eq!(out, br#"{"ok":true}"#);
# Ok::<(), std::io::Error>(())
```

### `Writer`

```text
pub struct Writer<'o> { /* ... */ }
```

A push writer: a document written one event at a time, checked as it goes.
For output whose shape is known only at run time, for transcoding a
`Reader`'s events (any depth: an explicit stack), and for streaming to an
`io::Write` without holding the document. It mixes with `ToJson`: `value`
takes any `ToJson`.

| Method | Description |
|---|---|
| `fn new(&'o mut String)` | Appends to a string. |
| `fn io(&'o mut dyn io::Write)` | (`std`) Writes through an 8 KiB buffer. |
| `fn fmt(&'o mut dyn fmt::Write)` | Writes to a formatter, in pieces; works without `std`. |
| `fn pretty(self)`, `fn with_format(self, Format)` | The layout; call before writing. |
| `fn begin_object`, `end_object`, `begin_array`, `end_array` | Containers. |
| `fn key(&mut self, &str)` | An object member's key. |
| `fn value<T: ToJson + ?Sized>(&mut self, &T)` | A whole value. |
| `fn event(&mut self, &Event)` | One `Reader` event. |
| `fn depth(&self)`, `fn is_complete(&self)` | State. |
| `fn finish(self)` | Checks the document is complete and passes on the rest of the output. |

Every method returns `Result<(), WriteError>`; a refused call writes nothing.

```rust
use jsonx_lib::{Reader, Writer};

let text = r#"{"a":[1,{"b":null}]}"#;
let mut out = String::new();
let mut w = Writer::new(&mut out).pretty();
for event in Reader::new(text) {
    w.event(&event?)?;
}
w.finish()?;
assert_eq!(out, jsonx_lib::to_string_pretty(&jsonx_lib::parse(text)?));
# Ok::<(), Box<dyn std::error::Error>>(())
```

### `WriteError`

```text
#[derive(Debug)]
#[non_exhaustive]
pub enum WriteError { Io(std::io::Error) /* std */, Fmt, ExpectedKey, UnexpectedKey, MismatchedEnd, DocumentComplete, Incomplete }
```

| Variant | Meaning |
|---|---|
| `Io(e)`, `Fmt` | The output failed; the writer is finished (later calls return an error of the same kind). |
| `ExpectedKey` | A value where an object member's key belongs. |
| `UnexpectedKey` | A key outside an object, or twice in a row. |
| `MismatchedEnd` | An end that does not close the innermost container (or with a key waiting for its value). |
| `DocumentComplete` | A second top-level value. |
| `Incomplete` | `finish` with containers open or no value. |

`Display` and `core::error::Error` (with the I/O error as its source).

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
| `fn line_col(&self, input: impl AsRef<[u8]>) -> (usize, usize)` | Line and column from 1, the column in characters: for a person. |
| `fn utf16_position(&self, input: impl AsRef<[u8]>) -> (usize, usize)` | Line and character from 0, in UTF-16 code units: an LSP `Position`. |
| `Display` | `"<message> at byte <offset>"`. |
| `core::error::Error` | Implemented. |

A line ends at `\n`, `\r\n`, or `\r`.

```rust
let text = "[\"\u{1f600}\",\n tru]";
let err = jsonx_lib::parse(text).unwrap_err();
assert_eq!(err.line_col(text), (2, 5));
assert_eq!(err.utf16_position(text), (1, 4));
```

## `ErrorKind`

`#[non_exhaustive]`: match with a wildcard arm. `is_budget(self) -> bool`
tells the six budget kinds (the same input may parse with larger limits)
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
| `InvalidUtf8` | Not UTF-8 (`parse_bytes`, `reader_bytes`, `StreamReader`). | The first byte of the invalid sequence. |
| `DuplicateKey` | A repeated key under `DuplicateKeys::Error`. | The second occurrence's opening quote. |
| `DepthLimitExceeded` | Nested deeper than `max_depth`. | The `[` or `{` that went too deep. |
| `InputTooLarge` | Longer than `max_input_bytes`. | The limit. |
| `StringTooLong` | A string or key over `max_string_bytes`. | Its opening quote. |
| `NumberTooLong` | A number over `max_number_bytes`. | Its first byte. |
| `TooManyElements` | More values than `max_elements`. | The value that went over. |
| `MemoryLimitExceeded` | The tree would hold more than `max_memory`. | The value whose storage went over. |
| `InputAfterEnd` | Bytes fed to a `StreamReader` after `end_input` (a mistake in the calling code). | The bytes accepted before the end. |

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
| `std` | yes | [`to_writer`](#to_writer), `to_writer_pretty`, `to_writer_with`, `Writer::io`, `StreamReader::read_from`, `WriteError::Io`, and the `HashMap`/`HashSet` impls of the typed traits. Without it the crate is `no_std` and needs only `alloc`. |

## Guide: moving a JSON-RPC server off serde_json

The serde_json features a language server leans on, and their counterparts:

| serde_json | jsonx-lib |
|---|---|
| `serde_json::from_str::<Value>(s)` | [`parse(s)`](#parse) (borrowing; `into_owned()` to keep) |
| `#[derive(Deserialize)] struct Params<'a>` | An [`impl FromJson<'de>`](#typed-reading-fromjson) (a few lines per struct), or [`json_struct!`](#json_struct) for a plain one; read with [`from_str`](#from_str) or `reader.decode()` |
| `#[derive(Serialize)]` for replies | An [`impl ToJson`](#typed-writing-tojson) or `json_struct!`; write with [`write_value`](#write_value) into your buffer: no tree built |
| `#[serde(rename = "textDocument")]` | The key in `match` / `member`, or `text_document: "textDocument"` in `json_struct!` |
| `Option<T>` fields with `skip_serializing_if = "Option::is_none"` | `ObjectWriter::optional`, which `json_struct!` uses for every field |
| `#[serde(default)]` / absent vs `null` | `FromJson::absent` (`Option`: `None`); [`Nullable<T>`](#nullable) to tell absent from `null` |
| `#[serde(untagged)]` enums | `reader.peek_kind()` then read the matching variant |
| `&RawValue` / `Box<RawValue>` fields | [`RawValue<'a>`](#rawvalue) fields (zero-copy from the reader) |
| `Cow<'a, str>` with `#[serde(borrow)]` | `Cow<'de, str>` / `&'de str` fields, borrowed unless escaped |
| `Value`, `Map<String, Value>` | [`Value`](#value), [`Object`](#object) (ordered) |
| `json!` | [`json!`](#json) |
| `to_string`, `to_writer` | [`to_string`](#to_string), [`to_writer`](#to_writer), for any `ToJson` |
| `serde_json::Error` (line, column) | [`DecodeError`](#decodeerror-decodeerrorkind-pathsegment): kind, byte offset, JSON Pointer path, `line_col`, `utf16_position` |
| `StreamDeserializer` | [`Reader::documents`](#reader), [`StreamReader::documents`](#streamreader-and-step) |
| Duplicate struct fields are errors | Refused by default on every reading path: `field` refuses a repeated field, the reader any repeated key |

## Stability

Pre-1.0. The surface above may change in a 0.x minor release, recorded in the
CHANGELOG with a migration note. It is frozen at `1.0.0` after a real
consumer (lsp-lang, then the LexerSketch API) has exercised it end to end.
MSRV 1.85; raising it is a minor release.
