//! # jsonx_lib
//!
//! Strict RFC 8259 JSON: a parser that borrows from its input, a pull reader,
//! and a deterministic writer, with budgets that refuse hostile input before
//! it costs memory. No dependencies; `no_std` with `alloc`.
//!
//! ## Quick start
//!
//! ```
//! use jsonx_lib::{json, parse, to_string};
//!
//! let doc = parse(r#"{"name": "jsonx", "version": [0, 2, 0], "stable": false}"#)?;
//! assert_eq!(doc["name"], "jsonx");
//! assert_eq!(doc["version"][1].as_u64(), Some(2));
//!
//! let reply = json!({ "ok": true, "items": [1, 2, 3] });
//! assert_eq!(to_string(&reply), r#"{"ok":true,"items":[1,2,3]}"#);
//! # Ok::<(), jsonx_lib::Error>(())
//! ```
//!
//! ## The three ways in
//!
//! 1. **[`parse`] and [`to_string`]** (plus [`parse_bytes`] and
//!    [`to_string_pretty`]): one call each, default budgets, duplicate keys
//!    refused.
//! 2. **[`Parser`]**: the same parse with your own [`Limits`] and
//!    [`DuplicateKeys`] policy, kept as a `const` and reused.
//! 3. **[`Reader`]**: the document as a stream of [`Event`]s, with
//!    [`read_raw`](Reader::read_raw) to keep a value as its source text and
//!    [`read_value`](Reader::read_value) to build just one part of it.
//!
//! ## What it guarantees
//!
//! - **Exactly RFC 8259.** Everything the grammar allows is accepted;
//!   everything else is refused with an [`Error`] that names the first byte at
//!   which the input stops being valid. Unpaired UTF-16 surrogate escapes,
//!   which the grammar allows but which decode to no Unicode text, are
//!   refused too.
//! - **Borrowing.** Strings without escapes and every number are slices of the
//!   input; only strings with escapes are copied, once, into a buffer of
//!   exactly the right size.
//! - **Exact numbers.** A [`Number`] keeps its text; conversion to `i64`,
//!   `u64`, or `f64` happens when asked and never rounds silently into an
//!   integer.
//! - **Budgets before allocation.** Depth, input size, string length, number
//!   length, and value count are all checked before the memory they guard is
//!   used ([`Limits`]).
//! - **No recursion on depth.** Parsing, writing, cloning, and comparing loop
//!   over explicit stacks; dropping recurses at most a fixed 32 levels. A
//!   million levels of nesting cannot overflow the stack.
//! - **Round trip.** Written output parses back to an equal value, and the
//!   escaping cannot be broken out of by any string ([`write_string`]).
//! - **No panics** on any input, and `#![forbid(unsafe_code)]`.
//!
//! ## Feature flags
//!
//! - `std` (default): adds [`to_writer`] and [`to_writer_pretty`]. Without it
//!   the crate is `no_std` and needs only `alloc`.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
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
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

extern crate alloc;

mod array;
mod error;
mod lexer;
mod limits;
mod machine;
mod macros;
mod number;
mod object;
mod parser;
mod reader;
mod scan;
mod tree;
mod value;
mod write;

pub use array::Array;
pub use error::{Error, ErrorKind};
pub use limits::Limits;
pub use number::Number;
pub use object::{Iter, IterMut, Object};
pub use parser::{DuplicateKeys, Parser};
pub use reader::{Event, Reader};
pub use value::{Kind, Value};
pub use write::{to_string, to_string_pretty, write_string, write_value, write_value_pretty};
#[cfg(feature = "std")]
pub use write::{to_writer, to_writer_pretty};

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
