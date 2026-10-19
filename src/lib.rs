//! # jsonx_lib
//!
//! Strict RFC 8259 JSON: a parser that borrows from its input, pull and
//! incremental readers, typed reading and writing through hand-written
//! [`FromJson`] and [`ToJson`] impls, and deterministic writers, with
//! budgets that refuse hostile input before it costs memory. No
//! dependencies; `no_std` with `alloc`.
//!
//! ## Quick start
//!
//! ```
//! use jsonx_lib::{json, json_struct, parse, to_string};
//!
//! // A document as a tree.
//! let doc = parse(r#"{"name": "jsonx", "version": [0, 5, 0], "stable": false}"#)?;
//! assert_eq!(doc["name"], "jsonx");
//! assert_eq!(doc["version"][1].as_u64(), Some(5));
//!
//! // Or straight into your own types, and back, with no tree in between.
//! struct Position { line: u32, character: u32 }
//! json_struct!(Position { line, character });
//!
//! let p: Position = jsonx_lib::from_str(r#"{"line": 3, "character": 14}"#)?;
//! assert_eq!((p.line, p.character), (3, 14));
//! assert_eq!(to_string(&p), r#"{"line":3,"character":14}"#);
//!
//! let reply = json!({ "ok": true, "items": [1, 2, 3] });
//! assert_eq!(to_string(&reply), r#"{"ok":true,"items":[1,2,3]}"#);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## The ways in
//!
//! 1. **[`parse`], [`from_str`], and [`to_string`]** (plus [`parse_bytes`],
//!    [`from_bytes`], [`to_string_pretty`], and [`to_string_with`]): one call
//!    each, default budgets, duplicate keys refused.
//! 2. **[`Parser`]**: the same with your own [`Limits`], [`DuplicateKeys`]
//!    policy, and [`Numbers`] option, kept as a `const` and reused.
//! 3. **[`Reader`]**: the document as a stream of [`Event`]s, with
//!    [`read_raw`](Reader::read_raw) to keep a value as its source text,
//!    [`read_value`](Reader::read_value) to build one part of it,
//!    [`decode`](Reader::decode) to read one part as a type, and
//!    [`find_pointer`](Reader::find_pointer) to go straight to one value.
//!    [`StreamReader`] is the same reader over input that arrives in chunks;
//!    both read sequences of documents (NDJSON) on request.
//! 4. **Typed values**: [`FromJson`] reads a type from a [`Reader`],
//!    borrowing strings from the input; [`ToJson`] writes a type through a
//!    [`ValueWriter`], building nothing. Both are implemented by hand (or by
//!    the [`json_struct!`] macro for plain structs): no derive, no
//!    reflection. Errors are [`DecodeError`]s: what, at which byte, and where
//!    (a JSON Pointer path).
//! 5. **[`Writer`]**: a push writer that takes events, checked as it goes,
//!    into a `String`, an `io::Write`, or a `fmt::Write`, in any [`Format`].
//!
//! ## What it guarantees
//!
//! - **RFC 8259 grammar.** Everything outside the grammar is refused with an
//!   [`Error`] that names the first byte at which the input stops being
//!   valid. Two grammatical inputs are refused by choice: unpaired UTF-16
//!   surrogate escapes, which decode to no Unicode text, and (under the
//!   default [`DuplicateKeys::Error`]) objects that repeat a key, on every
//!   way of reading.
//! - **Borrowing.** Strings without escapes and every number are slices of the
//!   input; a string with escapes is decoded once into a buffer no larger than
//!   its escaped source text.
//! - **Exact numbers.** A [`Number`] keeps its text; conversion to a Rust
//!   number happens when asked and never rounds silently into an integer.
//! - **Budgets.** Depth, input size, string length, number length, value
//!   count, and the memory a parse holds for its tree are all bounded
//!   ([`Limits`]), each checked before the memory it guards is used.
//! - **No recursion on depth.** Parsing, the readers, writing, cloning,
//!   comparing, and hashing loop over explicit stacks; dropping recurses at
//!   most a fixed 32 levels. A million levels of nesting cannot overflow the
//!   stack. (A [`FromJson`] or [`ToJson`] impl for a recursive type of your
//!   own recurses; the depth budget bounds that on the reading side.)
//! - **Round trip.** Written output parses back to an equal value, in every
//!   [`Format`], and no string can break out of its quotes ([`write_string`]).
//! - **No panics** on any input, and `#![forbid(unsafe_code)]`.
//!
//! ## Feature flags
//!
//! - `std` (default): adds `to_writer`, `to_writer_pretty`, `to_writer_with`,
//!   `Writer::io`, `StreamReader::read_from`, and the `HashMap`/`HashSet`
//!   impls of the typed traits. Without it the crate is `no_std` and needs
//!   only `alloc`.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![deny(warnings)]
#![deny(missing_docs)]
#![deny(unused_must_use)]
#![deny(unused_results)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::print_stdout)]
#![deny(clippy::print_stderr)]
#![deny(clippy::dbg_macro)]
#![deny(clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

extern crate alloc;

mod array;
mod decode;
mod dupes;
mod encode;
mod error;
mod format;
mod lexer;
mod limits;
mod machine;
mod macros;
mod number;
mod object;
mod parser;
mod raw;
mod reader;
mod scan;
mod small;
mod stream;
mod tree;
mod value;
mod write;
mod writer;

pub use array::Array;
pub use decode::{
    ArrayReader, DecodeError, DecodeErrorKind, FromJson, FromJsonKey, Nullable, ObjectReader,
    PathSegment, from_bytes, from_str, from_value, to_value,
};
pub use encode::{ArrayWriter, ObjectWriter, ToJson, ToJsonKey, ValueWriter, Written};
pub use error::{Error, ErrorKind};
pub use format::Format;
pub use limits::Limits;
pub use number::Number;
pub use object::{Entry, Iter, IterMut, Object, OccupiedEntry, VacantEntry};
pub use parser::{DuplicateKeys, Numbers, Parser};
pub use raw::RawValue;
pub use reader::{Event, Reader};
pub use stream::{Step, StreamReader};
pub use value::{Kind, Value};
pub use write::{
    to_string, to_string_pretty, to_string_with, write_string, write_value, write_value_pretty,
    write_value_with,
};
#[cfg(feature = "std")]
pub use write::{to_writer, to_writer_pretty, to_writer_with};
pub use writer::{WriteError, Writer};

/// Parses one JSON document with the default [`Limits`], refusing duplicate
/// keys.
///
/// The result borrows from `text`: strings without escapes and all numbers are
/// slices of it. Call [`Value::into_owned`] to keep the value longer than the
/// text. To change a budget or the duplicate-key policy, use [`Parser`].
///
/// # Errors
///
/// The first syntax error or exceeded budget, at its byte offset. See
/// [`ErrorKind`] for every case.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{parse, ErrorKind};
///
/// let v = parse(r#"{"list": [1, 2.5, "three"]}"#)?;
/// assert_eq!(v["list"][2], "three");
///
/// let err = parse(r#"{"a": tru}"#).unwrap_err();
/// assert_eq!(err.kind(), ErrorKind::InvalidLiteral);
/// assert_eq!(err.offset(), 9);
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
pub fn parse(text: &str) -> Result<Value<'_>, Error> {
    Parser::new().parse(text)
}

/// Parses one JSON document from bytes, which must be UTF-8, with the default
/// [`Limits`]. See [`Parser::parse_bytes`].
///
/// # Errors
///
/// [`ErrorKind::InvalidUtf8`] for bytes that are not UTF-8; otherwise as
/// [`parse`].
///
/// # Examples
///
/// ```
/// let body: &[u8] = br#"{"id": 1}"#;
/// assert_eq!(jsonx_lib::parse_bytes(body)?["id"], 1);
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
pub fn parse_bytes(bytes: &[u8]) -> Result<Value<'_>, Error> {
    Parser::new().parse_bytes(bytes)
}

/// The examples in `README.md`, compiled and run as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;

/// The examples in `docs/API.md`, compiled and run as doctests (with `std`,
/// which its `to_writer` example needs).
#[cfg(all(doctest, feature = "std"))]
#[doc = include_str!("../docs/API.md")]
pub struct ApiDoctests;

/// Support for the [`json!`] macro. Not public API.
#[doc(hidden)]
pub mod __private {
    use alloc::borrow::Cow;

    pub use alloc::vec;

    use crate::Value;

    /// Makes one object member.
    #[inline]
    pub fn member<'a>(key: impl Into<Cow<'a, str>>, value: Value<'a>) -> (Cow<'a, str>, Value<'a>) {
        (key.into(), value)
    }
}
