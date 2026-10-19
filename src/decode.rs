//! Typed input: [`FromJson`], read straight from a [`Reader`].
//!
//! A `FromJson` type reads itself from the reader's events: no [`Value`] is
//! built, strings borrow from the input unless they had escapes, and a
//! mismatch is a [`DecodeError`] that says what was expected, what was
//! found, at which byte, and where in the document (a JSON Pointer path).

#[cfg(target_has_atomic = "ptr")]
use alloc::sync::Arc;
use alloc::{
    borrow::Cow,
    boxed::Box,
    collections::{BTreeMap, BTreeSet, VecDeque},
    format,
    rc::Rc,
    string::String,
    vec::Vec,
};
use core::fmt;

use crate::{
    Array, DuplicateKeys, Kind, Limits, Number, Numbers, Object, Parser, Reader, ToJson, Value,
    ValueWriter, Written,
    error::{Error, ErrorKind},
    reader::Event,
};

// ---- Errors ------------------------------------------------------------------

/// One step of the path to a value: an object member's key, or an array
/// item's index.
///
/// # Examples
///
/// ```
/// use jsonx_lib::PathSegment;
///
/// assert_eq!(PathSegment::Key("line".into()).to_string(), "line");
/// assert_eq!(PathSegment::Index(3).to_string(), "3");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathSegment {
    /// The member with this key.
    Key(String),
    /// The item at this index.
    Index(usize),
}

impl fmt::Display for PathSegment {
    /// The segment as it reads in a path: the key, or the index in decimal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(key) => f.write_str(key),
            Self::Index(index) => write!(f, "{index}"),
        }
    }
}

/// What went wrong reading a typed value; see [`DecodeError`].
///
/// The enum is `#[non_exhaustive]`: match with a wildcard arm.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{DecodeErrorKind, Kind};
///
/// let err = jsonx_lib::from_str::<u8>("\"7\"").unwrap_err();
/// assert_eq!(err.kind(), &DecodeErrorKind::WrongType { expected: "u8", found: Kind::String });
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeErrorKind {
    /// The text is not valid JSON, or is over a budget: the [`ErrorKind`] a
    /// parse of the same text reports.
    Syntax(ErrorKind),
    /// The value is of the wrong kind: `found` where `expected` (a type
    /// name such as `"u32"`, or a description such as `"an object"`)
    /// belongs.
    WrongType {
        /// What the target type accepts.
        expected: &'static str,
        /// What the document has.
        found: Kind,
    },
    /// A number that does not fit the target type: out of its range, or not
    /// an integer for an integer type (`1.5` or `1e3` for a `u32`; integers
    /// are read from integer text only, as [`Number::as_i64`] does).
    OutOfRange {
        /// The target type.
        expected: &'static str,
    },
    /// A string with escape sequences was read into a borrowed `&str`, which
    /// can only point into the input. Read it into `String` or `Cow<str>`.
    StringNotBorrowable,
    /// An array of the wrong length for a fixed-size array or a tuple.
    WrongLength {
        /// The length the target needs.
        expected: usize,
        /// The length the document has.
        found: usize,
    },
    /// An object lacks a member its type requires. The offset is the
    /// object's opening brace.
    MissingField(&'static str),
    /// An object has a member its type does not know, refused with
    /// [`ObjectReader::unknown_field`]. The key is the last path segment.
    UnknownField,
    /// An object repeats a member, found by [`ObjectReader::field`] or a map
    /// under [`DuplicateKeys::Error`]. The key is the last path segment.
    DuplicateField,
    /// The value has the right kind but not an acceptable value; the text
    /// says what was expected (`"a single character"`, `"a URI"`).
    InvalidValue(&'static str),
    /// An error of the implementation's own, with its message.
    Custom(Cow<'static, str>),
}

impl fmt::Display for DecodeErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(kind) => write!(f, "{kind}"),
            Self::WrongType { expected, found } => write!(f, "expected {expected}, found {found}"),
            Self::OutOfRange { expected } => write!(f, "number out of range for {expected}"),
            Self::StringNotBorrowable => {
                f.write_str("a string with escapes cannot be borrowed as `&str`")
            }
            Self::WrongLength { expected, found } => {
                write!(f, "expected {expected} items, found {found}")
            }
            Self::MissingField(name) => write!(f, "missing field `{name}`"),
            Self::UnknownField => f.write_str("unknown field"),
            Self::DuplicateField => f.write_str("duplicate field"),
            Self::InvalidValue(expected) => write!(f, "invalid value: expected {expected}"),
            Self::Custom(message) => f.write_str(message),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Inner {
    kind: DecodeErrorKind,
    offset: usize,
    /// Outermost first.
    path: Vec<PathSegment>,
}

/// Why a typed value could not be read: what went wrong, at which byte, and
/// where in the document.
///
/// The path is built as the error travels out through the containers it
/// was found in, so it costs nothing until something fails.
/// [`pointer`](Self::pointer) gives it as an RFC 6901 JSON Pointer, which is
/// what [`Display`](fmt::Display) shows after the offset:
/// `expected u32, found string at byte 31 (at /position/line)`. That is a
/// ready-made message for a JSON-RPC `-32602 Invalid params` reply.
///
/// The error is one pointer wide, so `Result<T, DecodeError>` stays small.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{DecodeErrorKind, Kind, PathSegment};
///
/// let err = jsonx_lib::from_str::<Vec<Vec<u8>>>("[[1], [2, true]]").unwrap_err();
/// assert_eq!(err.kind(), &DecodeErrorKind::WrongType { expected: "u8", found: Kind::Bool });
/// assert_eq!(err.offset(), 10);
/// assert_eq!(err.path(), [PathSegment::Index(1), PathSegment::Index(1)]);
/// assert_eq!(err.pointer(), "/1/1");
/// assert_eq!(err.to_string(), "expected u8, found boolean at byte 10 (at /1/1)");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub struct DecodeError(Box<Inner>);

impl DecodeError {
    /// An error of `kind` at byte `offset`, with an empty path.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{DecodeError, DecodeErrorKind};
    ///
    /// let err = DecodeError::new(DecodeErrorKind::InvalidValue("a port"), 12);
    /// assert_eq!(err.to_string(), "invalid value: expected a port at byte 12");
    /// ```
    pub fn new(kind: DecodeErrorKind, offset: usize) -> Self {
        Self(Box::new(Inner {
            kind,
            offset,
            path: Vec::new(),
        }))
    }

    /// A [`DecodeErrorKind::WrongType`] error.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{DecodeError, Kind};
    ///
    /// let err = DecodeError::wrong_type("a position", Kind::Array, 4);
    /// assert_eq!(err.to_string(), "expected a position, found array at byte 4");
    /// ```
    pub fn wrong_type(expected: &'static str, found: Kind, offset: usize) -> Self {
        Self::new(DecodeErrorKind::WrongType { expected, found }, offset)
    }

    /// A [`DecodeErrorKind::InvalidValue`] error.
    ///
    /// # Examples
    ///
    /// ```
    /// let err = jsonx_lib::DecodeError::invalid_value("a URI", 0);
    /// assert_eq!(err.to_string(), "invalid value: expected a URI at byte 0");
    /// ```
    pub fn invalid_value(expected: &'static str, offset: usize) -> Self {
        Self::new(DecodeErrorKind::InvalidValue(expected), offset)
    }

    /// A [`DecodeErrorKind::Custom`] error with `message`.
    ///
    /// # Examples
    ///
    /// ```
    /// let err = jsonx_lib::DecodeError::custom("end before start", 9);
    /// assert_eq!(err.to_string(), "end before start at byte 9");
    /// ```
    pub fn custom(message: impl Into<Cow<'static, str>>, offset: usize) -> Self {
        Self::new(DecodeErrorKind::Custom(message.into()), offset)
    }

    /// A [`DecodeErrorKind::MissingField`] error.
    ///
    /// # Examples
    ///
    /// ```
    /// let err = jsonx_lib::DecodeError::missing_field("uri", 0);
    /// assert_eq!(err.to_string(), "missing field `uri` at byte 0");
    /// ```
    pub fn missing_field(name: &'static str, offset: usize) -> Self {
        Self::new(DecodeErrorKind::MissingField(name), offset)
    }

    /// The error as seen from the object that holds it under `key`: the key
    /// goes in front of the path. The object readers do this for you.
    ///
    /// # Examples
    ///
    /// ```
    /// let err = jsonx_lib::DecodeError::invalid_value("a URI", 7).at_key("uri").at_key("textDocument");
    /// assert_eq!(err.pointer(), "/textDocument/uri");
    /// ```
    pub fn at_key(mut self, key: impl Into<String>) -> Self {
        self.0.path.insert(0, PathSegment::Key(key.into()));
        self
    }

    /// The error as seen from the array that holds it at `index`.
    ///
    /// # Examples
    ///
    /// ```
    /// let err = jsonx_lib::DecodeError::invalid_value("a URI", 7).at_index(2);
    /// assert_eq!(err.pointer(), "/2");
    /// ```
    pub fn at_index(mut self, index: usize) -> Self {
        self.0.path.insert(0, PathSegment::Index(index));
        self
    }

    /// What went wrong.
    #[must_use]
    pub fn kind(&self) -> &DecodeErrorKind {
        &self.0.kind
    }

    /// The byte offset in the input where it went wrong: the value that did
    /// not fit, or for a syntax error the byte a parse reports.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.0.offset
    }

    /// Where in the document, outermost first; empty for the top-level
    /// value.
    #[must_use]
    pub fn path(&self) -> &[PathSegment] {
        &self.0.path
    }

    /// The path as an [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901)
    /// JSON Pointer (`/params/items/3`, `""` for the top level), usable with
    /// [`Value::pointer`].
    ///
    /// # Examples
    ///
    /// ```
    /// let err = jsonx_lib::DecodeError::invalid_value("x", 0).at_key("a/b").at_key("~");
    /// assert_eq!(err.pointer(), "/~0/a~1b");
    /// ```
    #[must_use]
    pub fn pointer(&self) -> String {
        let mut out = String::new();
        for segment in &self.0.path {
            out.push('/');
            match segment {
                PathSegment::Key(key) => {
                    for ch in key.chars() {
                        match ch {
                            '~' => out.push_str("~0"),
                            '/' => out.push_str("~1"),
                            _ => out.push(ch),
                        }
                    }
                }
                PathSegment::Index(index) => out.push_str(&format!("{index}")),
            }
        }
        out
    }

    /// The error's line and column in `input`, from 1, the column in
    /// characters; see [`Error::line_col`].
    ///
    /// # Examples
    ///
    /// ```
    /// let text = "{\n  \"n\": \"seven\"\n}";
    /// let err = jsonx_lib::from_str::<std::collections::BTreeMap<String, u8>>(text).unwrap_err();
    /// assert_eq!(err.line_col(text), (2, 8));
    /// ```
    #[must_use]
    pub fn line_col(&self, input: impl AsRef<[u8]>) -> (usize, usize) {
        let at = crate::error::locate(input.as_ref(), self.0.offset);
        (at.line + 1, at.chars + 1)
    }

    /// The error's position in `input` as LSP counts it (from 0, the
    /// character in UTF-16 units); see [`Error::utf16_position`].
    ///
    /// # Examples
    ///
    /// ```
    /// let text = "[1,\n \"x\"]";
    /// let err = jsonx_lib::from_str::<Vec<u8>>(text).unwrap_err();
    /// assert_eq!(err.utf16_position(text), (1, 1));
    /// ```
    #[must_use]
    pub fn utf16_position(&self, input: impl AsRef<[u8]>) -> (usize, usize) {
        let at = crate::error::locate(input.as_ref(), self.0.offset);
        (at.line, at.utf16)
    }

    /// The parse error, for a [`DecodeErrorKind::Syntax`] error.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::ErrorKind;
    ///
    /// let err = jsonx_lib::from_str::<Vec<u8>>("[1,]").unwrap_err();
    /// assert_eq!(err.syntax_error().map(|e| e.kind()), Some(ErrorKind::TrailingComma));
    /// ```
    #[must_use]
    pub fn syntax_error(&self) -> Option<Error> {
        match self.0.kind {
            DecodeErrorKind::Syntax(kind) => Some(Error::new(kind, self.0.offset)),
            _ => None,
        }
    }

    /// The same error `by` bytes further into a larger input.
    pub(crate) fn shifted(mut self, by: usize) -> Self {
        self.0.offset += by;
        self
    }
}

impl From<Error> for DecodeError {
    fn from(e: Error) -> Self {
        Self::new(DecodeErrorKind::Syntax(e.kind()), e.offset())
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.0.kind, self.0.offset)?;
        if !self.0.path.is_empty() {
            write!(f, " (at {})", self.pointer())?;
        }
        Ok(())
    }
}

impl core::error::Error for DecodeError {}

// ---- The traits --------------------------------------------------------------

/// A type that reads itself from JSON, straight from a [`Reader`].
///
/// The explicit, hand-written counterpart of [`ToJson`]: no derive and no
/// reflection, so an implementation says in code which members a type has
/// and how each is read. Nothing is built in between, and the lifetime
/// `'de` is the input's, so a type can borrow from it: a `&'de str` or
/// `Cow<'de, str>` field points into the text when the string had no
/// escapes.
///
/// # Implementing it
///
/// `from_json` is called with the reader positioned where a value comes
/// next, and must read exactly that one value. Read a scalar with
/// [`reader.decode()`](Reader::decode); an object with
/// [`read_object`](Reader::read_object), which hands out its members; an
/// array with [`read_array`](Reader::read_array). For a struct:
///
/// ```
/// use std::borrow::Cow;
/// use jsonx_lib::{DecodeError, FromJson, Reader};
///
/// struct Position { line: u32, character: u32 }
///
/// impl<'de> FromJson<'de> for Position {
///     fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
///         let (mut line, mut character) = (None, None);
///         let mut object = reader.read_object()?;
///         while let Some(key) = object.next_key()? {
///             match &*key {
///                 "line" => object.field(&mut line)?,
///                 "character" => object.field(&mut character)?,
///                 _ => {} // unknown members are skipped (and still validated)
///             }
///         }
///         Ok(Position {
///             line: object.required(line, "line")?,
///             character: object.required(character, "character")?,
///         })
///     }
/// }
///
/// struct Location<'a> { uri: Cow<'a, str>, position: Position, label: Option<String> }
///
/// impl<'de> FromJson<'de> for Location<'de> {
///     fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
///         let (mut uri, mut position, mut label) = (None, None, None);
///         let mut object = reader.read_object()?;
///         while let Some(key) = object.next_key()? {
///             match &*key {
///                 "uri" => object.field(&mut uri)?,
///                 "position" => object.field(&mut position)?,
///                 "label" => object.field(&mut label)?,
///                 _ => {}
///             }
///         }
///         Ok(Location {
///             uri: object.required(uri, "uri")?,
///             position: object.required(position, "position")?,
///             label: object.required(label, "label")?, // absent reads as None
///         })
///     }
/// }
///
/// let text = r#"{"uri": "file:///a.rs", "position": {"line": 3, "character": 14}}"#;
/// let loc: Location = jsonx_lib::from_str(text)?;
/// assert!(matches!(loc.uri, Cow::Borrowed("file:///a.rs"))); // no copy
/// assert_eq!((loc.position.line, loc.position.character), (3, 14));
/// assert_eq!(loc.label, None);
///
/// let err = jsonx_lib::from_str::<Location>(r#"{"uri": "x", "position": {"line": "3"}}"#).err();
/// assert_eq!(
///     err.map(|e| e.to_string()).as_deref(),
///     Some("expected u32, found string at byte 34 (at /position/line)")
/// );
/// # Ok::<(), DecodeError>(())
/// ```
///
/// What the object reader does for you:
///
/// - **Duplicates are refused.** [`field`](ObjectReader::field) refuses a
///   second value for a field it already has
///   ([`DecodeErrorKind::DuplicateField`]), and under the default
///   [`DuplicateKeys::Error`] the reader refuses any repeated key, known or
///   not, when the object closes. A second value can never be smuggled
///   past the first.
/// - **Unknown members are skipped**, after being validated, which is what
///   LSP and MCP ask of receivers. To refuse them, return
///   [`unknown_field`](ObjectReader::unknown_field) for the keys you do not
///   know.
/// - **Absent members:** [`required`](ObjectReader::required) turns a field
///   never seen into [`DecodeErrorKind::MissingField`], except for a type
///   whose [`absent`](Self::absent) gives a value: `Option<T>` (`None`) and
///   [`Nullable<T>`] (`Absent`).
/// - **Paths:** an error inside a member's value gets the member's key in
///   its path; an error inside an array item gets the index.
///
/// An enum whose variants have different shapes looks before it reads,
/// with [`peek_kind`](Reader::peek_kind):
///
/// ```
/// use jsonx_lib::{DecodeError, FromJson, Kind, Reader};
///
/// enum Id { Number(i64), Text(String) }
///
/// impl<'de> FromJson<'de> for Id {
///     fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
///         match reader.peek_kind()? {
///             Some(Kind::Number) => Ok(Id::Number(reader.decode()?)),
///             Some(Kind::String) => Ok(Id::Text(reader.decode()?)),
///             Some(found) => Err(DecodeError::wrong_type("a number or a string", found, reader.offset())),
///             None => Err(reader.decode::<()>().unwrap_err()), // let the reader say what is wrong
///         }
///     }
/// }
///
/// assert!(matches!(jsonx_lib::from_str::<Id>("7")?, Id::Number(7)));
/// assert!(matches!(jsonx_lib::from_str::<Id>("\"a\"")?, Id::Text(s) if s == "a"));
/// assert!(jsonx_lib::from_str::<Id>("true").is_err());
/// # Ok::<(), DecodeError>(())
/// ```
///
/// # Depth
///
/// A type that nests as deep as its data (a tree of its own nodes) recurses
/// once per level. The reader's [`Limits::max_depth`] (128 by default)
/// refuses anything deeper before `from_json` is called for it, so the
/// recursion is bounded by the budget, not by the input.
///
/// # Provided implementations
///
/// `bool`; every integer type (from integer text in range); `f32`, `f64`
/// (any finite number); `String`, `Box<str>`, `Cow<'de, str>`, `&'de str`
/// (only for strings without escapes), `char` (a one-character string);
/// `()` (`null`); `Option<T>` (`null` or absent is `None`), [`Nullable<T>`];
/// `Box<T>`, `Rc<T>`, `Arc<T>`; `Vec<T>`, `VecDeque<T>`, `Box<[T]>`,
/// `[T; N]`, `BTreeSet<T>` and with `std` `HashSet<T, S>` (arrays; a set
/// keeps one of equal items); tuples of up to eight (arrays of that
/// length); `BTreeMap<K, V>` and with `std` `HashMap<K, V, S>` (objects,
/// keys through [`FromJsonKey`], duplicates per the reader's
/// [`DuplicateKeys`] policy); and this crate's [`Value`], [`Number`],
/// [`Array`], [`Object`], and [`RawValue`](crate::RawValue).
pub trait FromJson<'de>: Sized {
    /// Reads one value from `reader`, which is positioned where a value
    /// comes next.
    ///
    /// # Errors
    ///
    /// A [`DecodeError`] for a value that is not of this type's shape, or
    /// any error of the reader.
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError>;

    /// The value for an object member that is absent, if absence is
    /// allowed: `None` (absence is an error) by default, `Some(None)` for
    /// `Option<T>`, and `Some(Nullable::Absent)` for [`Nullable<T>`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::FromJson;
    ///
    /// assert_eq!(<Option<u8> as FromJson>::absent(), Some(None));
    /// assert_eq!(<u8 as FromJson>::absent(), None);
    /// ```
    #[inline]
    fn absent() -> Option<Self> {
        None
    }
}

/// A type usable as a JSON object key, for [`FromJson`] on maps.
///
/// # Examples
///
/// ```
/// use std::{borrow::Cow, collections::BTreeMap};
/// use jsonx_lib::{DecodeErrorKind, FromJsonKey};
///
/// #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
/// struct Lang(String);
///
/// impl<'de> FromJsonKey<'de> for Lang {
///     fn from_json_key(key: Cow<'de, str>) -> Result<Self, DecodeErrorKind> {
///         if key.len() == 2 { Ok(Lang(key.into_owned())) } else { Err(DecodeErrorKind::InvalidValue("a language code")) }
///     }
/// }
///
/// let names: BTreeMap<Lang, String> = jsonx_lib::from_str(r#"{"en": "hello"}"#)?;
/// assert_eq!(names[&Lang("en".into())], "hello");
/// let err = jsonx_lib::from_str::<BTreeMap<Lang, String>>(r#"{"eng": "x"}"#).unwrap_err();
/// assert_eq!(err.pointer(), "/eng");
/// # Ok::<(), jsonx_lib::DecodeError>(())
/// ```
pub trait FromJsonKey<'de>: Sized {
    /// Makes the key from its decoded text.
    ///
    /// # Errors
    ///
    /// The reason the text is not a valid key; the map reports it at the
    /// key, with the key in the path.
    fn from_json_key(key: Cow<'de, str>) -> Result<Self, DecodeErrorKind>;
}

// ---- Reading through the reader ----------------------------------------------

/// Reads one value as `T`, then completes any container `T` left open, so
/// an implementation that stops early (returning once it has what it
/// wants) cannot leave the reader in the middle of a value.
pub(crate) fn decode_value<'de, T: FromJson<'de>>(
    reader: &mut Reader<'de>,
) -> Result<T, DecodeError> {
    let depth = reader.depth();
    let value = T::from_json(reader)?;
    while reader.depth() > depth {
        if reader.next_event()?.is_none() {
            break;
        }
    }
    Ok(value)
}

impl<'a> Reader<'a> {
    /// Reads the next value as a `T` ([`FromJson`]).
    ///
    /// # Errors
    ///
    /// The [`DecodeError`] of `T`, or of the reader.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Event, Reader};
    ///
    /// let mut r = Reader::new(r#"{"id": 7, "tags": ["a", "b"]}"#);
    /// r.next_event()?; // {
    /// r.next_event()?; // "id"
    /// assert_eq!(r.decode::<u64>()?, 7);
    /// r.next_event()?; // "tags"
    /// assert_eq!(r.decode::<Vec<&str>>()?, ["a", "b"]);
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn decode<T: FromJson<'a>>(&mut self) -> Result<T, DecodeError> {
        decode_value(self)
    }

    /// Starts reading an object whose members are read one by one through
    /// the returned [`ObjectReader`]. Consumes the `{`.
    ///
    /// # Errors
    ///
    /// [`DecodeErrorKind::WrongType`] if the next value is not an object
    /// (nothing is consumed), or an error of the reader.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new(r#"{"a": 1, "b": [2]}"#);
    /// let mut object = r.read_object()?;
    /// let mut keys = Vec::new();
    /// while let Some(key) = object.next_key()? {
    ///     keys.push(key.into_owned()); // values not read are skipped
    /// }
    /// assert_eq!(keys, ["a", "b"]);
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn read_object(&mut self) -> Result<ObjectReader<'_, 'a>, DecodeError> {
        let start = self.open(Kind::Object, "an object")?;
        Ok(ObjectReader {
            reader: self,
            start,
            key_offset: start,
            pending: false,
            done: false,
        })
    }

    /// Starts reading an array whose items are read one by one through the
    /// returned [`ArrayReader`]. Consumes the `[`.
    ///
    /// # Errors
    ///
    /// [`DecodeErrorKind::WrongType`] if the next value is not an array, or
    /// an error of the reader.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new("[1, 2, 3]");
    /// let mut items = r.read_array()?;
    /// let mut sum = 0u32;
    /// while let Some(n) = items.next_item::<u32>()? {
    ///     sum += n;
    /// }
    /// assert_eq!(sum, 6);
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn read_array(&mut self) -> Result<ArrayReader<'_, 'a>, DecodeError> {
        let start = self.open(Kind::Array, "an array")?;
        Ok(ArrayReader {
            reader: self,
            start,
            index: 0,
            done: false,
        })
    }

    /// Reads the next value as a [`RawValue`](crate::RawValue): its exact
    /// source text, validated by this read and not scanned again. As
    /// [`read_raw`](Self::read_raw), `Ok(None)` when no value comes next.
    ///
    /// # Errors
    ///
    /// As [`read_raw`](Self::read_raw).
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new(r#"{"params": {"line": 3}}"#);
    /// r.next_event()?;
    /// r.next_event()?;
    /// let params = r.read_raw_value()?.unwrap();
    /// assert_eq!(params.as_str(), r#"{"line": 3}"#);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn read_raw_value(&mut self) -> Result<Option<crate::RawValue<'a>>, Error> {
        Ok(self.read_raw()?.map(crate::RawValue::trusted))
    }

    /// Checks that the next value is of `kind` and consumes its first event;
    /// returns its offset.
    fn open(&mut self, kind: Kind, expected: &'static str) -> Result<usize, DecodeError> {
        if self.enter(kind == Kind::Object)? {
            return Ok(self.event_offset());
        }
        // Looked at before it is read: a value of another type is left
        // unread.
        match self.peek_kind()? {
            Some(found) if found == kind => {}
            Some(found) => return Err(DecodeError::wrong_type(expected, found, self.offset())),
            None if !self.at_a_value()? => {
                // A container ends, or the document did: there is no value.
                return Err(DecodeError::new(
                    DecodeErrorKind::Syntax(ErrorKind::ExpectedValue),
                    self.offset(),
                ));
            }
            // A byte that cannot start a value: the step below says why.
            None => {}
        }
        match self.next_event()? {
            Some(Event::StartObject | Event::StartArray) => Ok(self.event_offset()),
            Some(other) => Err(DecodeError::wrong_type(
                expected,
                event_kind(&other),
                self.event_offset(),
            )),
            None => Err(Error::new(ErrorKind::UnexpectedEnd, self.offset()).into()),
        }
    }

    /// Reads a scalar value's event for a target described by `expected`:
    /// one step of the reader. A container is the wrong type (its first
    /// event is consumed, which does not matter: the read has failed); no
    /// value at all is [`ErrorKind::ExpectedValue`] (or the reader's own
    /// error, such as a trailing comma).
    #[inline]
    fn scalar(&mut self, expected: &'static str) -> Result<(Event<'a>, usize), DecodeError> {
        match self.next_event()? {
            Some(event @ (Event::StartArray | Event::StartObject)) => Err(DecodeError::wrong_type(
                expected,
                event_kind(&event),
                self.event_offset(),
            )),
            Some(Event::EndArray | Event::EndObject | Event::Key(_)) => Err(DecodeError::new(
                DecodeErrorKind::Syntax(ErrorKind::ExpectedValue),
                self.event_offset(),
            )),
            Some(event) => Ok((event, self.event_offset())),
            None => Err(DecodeError::new(
                DecodeErrorKind::Syntax(ErrorKind::ExpectedValue),
                self.offset(),
            )),
        }
    }
}

/// The kind of value an event starts.
fn event_kind(event: &Event<'_>) -> Kind {
    match event {
        Event::StartObject | Event::EndObject | Event::Key(_) => Kind::Object,
        Event::StartArray | Event::EndArray => Kind::Array,
        Event::String(_) => Kind::String,
        Event::Number(_) => Kind::Number,
        Event::Bool(_) => Kind::Bool,
        Event::Null => Kind::Null,
    }
}

/// Reads the members of an object one by one; from
/// [`Reader::read_object`].
///
/// Call [`next_key`](Self::next_key) until it returns `None`; after each
/// key, read its value with [`field`](Self::field), [`value`](Self::value),
/// [`value_with`](Self::value_with), [`raw`](Self::raw), or
/// [`skip`](Self::skip), or do nothing and the next `next_key` skips it.
/// Errors in a value get the member's key in their path. See [`FromJson`]
/// for the whole pattern.
pub struct ObjectReader<'r, 'de> {
    reader: &'r mut Reader<'de>,
    start: usize,
    /// The current key's opening quote (its text is read again only for an
    /// error's path).
    key_offset: usize,
    /// A key was read and its value not yet.
    pending: bool,
    done: bool,
}

impl<'de> ObjectReader<'_, 'de> {
    /// The next member's key (borrowed from the input unless it had
    /// escapes), or `None` after the last member, when the object's `}` has
    /// been read. A value left unread is skipped first (and validated).
    ///
    /// # Errors
    ///
    /// Any error of the reader, including a repeated key under
    /// [`DuplicateKeys::Error`] (reported when the object closes).
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new(r#"{"a": [1, 2], "b": 3}"#);
    /// let mut o = r.read_object()?;
    /// assert_eq!(o.next_key()?.as_deref(), Some("a"));
    /// assert_eq!(o.next_key()?.as_deref(), Some("b")); // `[1, 2]` skipped
    /// assert_eq!(o.value::<u8>()?, 3);
    /// assert_eq!(o.next_key()?, None);
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn next_key(&mut self) -> Result<Option<Cow<'de, str>>, DecodeError> {
        if self.done {
            return Ok(None);
        }
        if self.pending {
            self.skip()?;
        }
        match self.reader.next_event()? {
            Some(Event::Key(key)) => {
                self.key_offset = self.reader.event_offset();
                self.pending = true;
                Ok(Some(key))
            }
            Some(Event::EndObject) => {
                self.done = true;
                Ok(None)
            }
            _ => Err(DecodeError::new(
                DecodeErrorKind::Syntax(ErrorKind::ExpectedKey),
                self.reader.event_offset(),
            )),
        }
    }

    /// Reads the current member's value as a `T`.
    ///
    /// # Errors
    ///
    /// The [`DecodeError`] of `T`, with this member's key in its path.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new(r#"{"n": "x"}"#);
    /// let mut o = r.read_object()?;
    /// o.next_key()?;
    /// assert_eq!(o.value::<u8>().unwrap_err().pointer(), "/n");
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    #[inline]
    pub fn value<T: FromJson<'de>>(&mut self) -> Result<T, DecodeError> {
        self.value_with(T::from_json)
    }

    /// Reads the current member's value with `read`, for a value that needs
    /// more than a type (a choice of shapes, a value read in place). Errors
    /// get the key in their path.
    ///
    /// # Errors
    ///
    /// Whatever `read` returns, with this member's key in its path.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new(r#"{"n": [1, 2, 3]}"#);
    /// let mut o = r.read_object()?;
    /// o.next_key()?;
    /// let count = o.value_with(|r| {
    ///     let mut items = r.read_array()?;
    ///     let mut count = 0;
    ///     while items.skip()? {
    ///         count += 1;
    ///     }
    ///     Ok(count)
    /// })?;
    /// assert_eq!(count, 3);
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn value_with<T>(
        &mut self,
        read: impl FnOnce(&mut Reader<'de>) -> Result<T, DecodeError>,
    ) -> Result<T, DecodeError> {
        self.pending = false;
        let depth = self.reader.depth();
        let result = read(self.reader).and_then(|value| {
            // Finish a container `read` left open.
            while self.reader.depth() > depth {
                if self.reader.next_event()?.is_none() {
                    break;
                }
            }
            Ok(value)
        });
        result.map_err(|e| self.in_member(e))
    }

    /// `e` with the current member's key in its path.
    #[cold]
    #[inline(never)]
    fn in_member(&self, e: DecodeError) -> DecodeError {
        e.at_key(self.reader.key_at(self.key_offset))
    }

    /// Reads the current member's value into `slot`, the pattern for a
    /// struct field: refused as [`DecodeErrorKind::DuplicateField`] (at the
    /// key) if `slot` already holds a value, so a second value for a field
    /// cannot override the first. Under [`DuplicateKeys::First`] the second
    /// value is skipped instead, under `Last` and `KeepAll` it replaces the
    /// first.
    ///
    /// # Errors
    ///
    /// [`DecodeErrorKind::DuplicateField`], or the [`DecodeError`] of `T`,
    /// with this member's key in the path.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::DecodeErrorKind;
    ///
    /// let mut r = jsonx_lib::Reader::new(r#"{"id": 1, "id": 2}"#);
    /// let mut o = r.read_object()?;
    /// let mut id: Option<u8> = None;
    /// o.next_key()?;
    /// o.field(&mut id)?;
    /// o.next_key()?;
    /// let err = o.field(&mut id).unwrap_err();
    /// assert_eq!((err.kind(), err.offset(), err.pointer()), (&DecodeErrorKind::DuplicateField, 10, "/id".into()));
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn field<T: FromJson<'de>>(&mut self, slot: &mut Option<T>) -> Result<(), DecodeError> {
        if slot.is_some() {
            match self.reader.duplicate_policy() {
                DuplicateKeys::First => return self.skip(),
                DuplicateKeys::Last | DuplicateKeys::KeepAll => {}
                _ => {
                    let e = DecodeError::new(DecodeErrorKind::DuplicateField, self.key_offset);
                    return Err(self.in_member(e));
                }
            }
        }
        *slot = Some(self.value()?);
        Ok(())
    }

    /// Skips the current member's value, validating it.
    ///
    /// # Errors
    ///
    /// Any error of the reader inside the value.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new(r#"{"big": [[1], {}], "n": 1}"#);
    /// let mut o = r.read_object()?;
    /// o.next_key()?;
    /// o.skip()?;
    /// assert_eq!(o.next_key()?.as_deref(), Some("n"));
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn skip(&mut self) -> Result<(), DecodeError> {
        self.raw().map(|_| ())
    }

    /// The current member's value as its exact source text, validated.
    ///
    /// # Errors
    ///
    /// Any error of the reader inside the value.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new(r#"{"params": {"line": 3}}"#);
    /// let mut o = r.read_object()?;
    /// o.next_key()?;
    /// assert_eq!(o.raw()?, r#"{"line": 3}"#);
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn raw(&mut self) -> Result<&'de str, DecodeError> {
        self.pending = false;
        match self.reader.read_raw() {
            Ok(Some(text)) => Ok(text),
            Ok(None) => {
                let e = DecodeError::new(
                    DecodeErrorKind::Syntax(ErrorKind::ExpectedValue),
                    self.reader.offset(),
                );
                Err(self.in_member(e))
            }
            Err(e) => Err(self.in_member(e.into())),
        }
    }

    /// The error for the current member being one this type does not know:
    /// [`DecodeErrorKind::UnknownField`] at its key, with the key in the
    /// path. For a type that refuses unknown members.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{DecodeError, FromJson, Reader};
    ///
    /// struct Strict { a: u8 }
    ///
    /// impl<'de> FromJson<'de> for Strict {
    ///     fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
    ///         let mut a = None;
    ///         let mut o = reader.read_object()?;
    ///         while let Some(key) = o.next_key()? {
    ///             match &*key {
    ///                 "a" => o.field(&mut a)?,
    ///                 _ => return Err(o.unknown_field()),
    ///             }
    ///         }
    ///         Ok(Strict { a: o.required(a, "a")? })
    ///     }
    /// }
    ///
    /// let err = jsonx_lib::from_str::<Strict>(r#"{"a": 1, "b": 2}"#).err();
    /// assert_eq!(err.map(|e| e.to_string()).as_deref(), Some("unknown field at byte 9 (at /b)"));
    /// ```
    pub fn unknown_field(&self) -> DecodeError {
        self.in_member(DecodeError::new(
            DecodeErrorKind::UnknownField,
            self.key_offset,
        ))
    }

    /// The value of a field read with [`field`](Self::field): `slot`'s
    /// value, or if the field was absent `T`'s [`absent`](FromJson::absent)
    /// value (`None` for an `Option`), or else
    /// [`DecodeErrorKind::MissingField`] at the object's `{`.
    ///
    /// # Errors
    ///
    /// [`DecodeErrorKind::MissingField`].
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new("{}");
    /// let mut o = r.read_object()?;
    /// assert_eq!(o.next_key()?, None);
    /// assert_eq!(o.required::<Option<u8>>(None, "maybe")?, None);
    /// assert_eq!(o.required::<u8>(None, "must").unwrap_err().to_string(), "missing field `must` at byte 0");
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn required<T: FromJson<'de>>(
        &self,
        slot: Option<T>,
        name: &'static str,
    ) -> Result<T, DecodeError> {
        slot.or_else(T::absent)
            .ok_or_else(|| DecodeError::missing_field(name, self.start))
    }

    /// The byte offset of the object's `{`.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.start
    }
}

/// Reads the items of an array one by one; from [`Reader::read_array`].
///
/// Errors in an item get its index in their path.
pub struct ArrayReader<'r, 'de> {
    reader: &'r mut Reader<'de>,
    start: usize,
    index: usize,
    done: bool,
}

impl<'de> ArrayReader<'_, 'de> {
    /// Whether another item comes; consumes the `]` when not.
    fn more(&mut self) -> Result<bool, DecodeError> {
        if self.done {
            return Ok(false);
        }
        if self.reader.at_a_value()? {
            return Ok(true);
        }
        match self.reader.next_event()? {
            Some(Event::EndArray) => {
                self.done = true;
                Ok(false)
            }
            _ => Err(DecodeError::new(
                DecodeErrorKind::Syntax(ErrorKind::ExpectedCommaOrBracket),
                self.reader.event_offset(),
            )),
        }
    }

    /// Reads the next item as a `T`, or `None` after the last one, when the
    /// array's `]` has been read.
    ///
    /// # Errors
    ///
    /// The [`DecodeError`] of `T`, with the item's index in its path.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new(r#"[1, "x"]"#);
    /// let mut items = r.read_array()?;
    /// assert_eq!(items.next_item::<u8>()?, Some(1));
    /// assert_eq!(items.next_item::<u8>().unwrap_err().pointer(), "/1");
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn next_item<T: FromJson<'de>>(&mut self) -> Result<Option<T>, DecodeError> {
        self.next_with(decode_value)
    }

    /// Reads the next item with `read`, or returns `None` after the last.
    ///
    /// # Errors
    ///
    /// Whatever `read` returns, with the item's index in its path.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new("[[1, 2], [3]]");
    /// let mut rows = r.read_array()?;
    /// let mut lengths = Vec::new();
    /// while let Some(n) = rows.next_with(|r| Ok(r.decode::<Vec<u8>>()?.len()))? {
    ///     lengths.push(n);
    /// }
    /// assert_eq!(lengths, [2, 1]);
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn next_with<T>(
        &mut self,
        read: impl FnOnce(&mut Reader<'de>) -> Result<T, DecodeError>,
    ) -> Result<Option<T>, DecodeError> {
        if !self.more()? {
            return Ok(None);
        }
        let index = self.index;
        self.index += 1;
        let depth = self.reader.depth();
        let result = read(self.reader).and_then(|value| {
            while self.reader.depth() > depth {
                if self.reader.next_event()?.is_none() {
                    break;
                }
            }
            Ok(value)
        });
        result.map(Some).map_err(|e| e.at_index(index))
    }

    /// Skips the next item, validating it; `false` after the last.
    ///
    /// # Errors
    ///
    /// Any error of the reader inside the item.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new("[{}, []]");
    /// let mut items = r.read_array()?;
    /// assert!(items.skip()? && items.skip()?);
    /// assert!(!items.skip()?);
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn skip(&mut self) -> Result<bool, DecodeError> {
        let index = self.index;
        if !self.more()? {
            return Ok(false);
        }
        self.index += 1;
        match self.reader.read_raw() {
            Ok(_) => Ok(true),
            Err(e) => Err(DecodeError::from(e).at_index(index)),
        }
    }

    /// Checks that no item is left: for a fixed-length array or a tuple,
    /// after its items were read.
    ///
    /// # Errors
    ///
    /// [`DecodeErrorKind::WrongLength`] at the `[` if items remain (all of
    /// them are skipped and counted), or an error of the reader.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::DecodeErrorKind;
    ///
    /// let mut r = jsonx_lib::Reader::new("[1, 2, 3]");
    /// let mut items = r.read_array()?;
    /// items.next_item::<u8>()?;
    /// let err = items.end().unwrap_err();
    /// assert_eq!(err.kind(), &DecodeErrorKind::WrongLength { expected: 1, found: 3 });
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn end(mut self) -> Result<(), DecodeError> {
        let expected = self.index;
        if !self.more()? {
            return Ok(());
        }
        while self.skip()? {}
        Err(DecodeError::new(
            DecodeErrorKind::WrongLength {
                expected,
                found: self.index,
            },
            self.start,
        ))
    }

    /// How many items have been read or skipped.
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    /// The byte offset of the array's `[`.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.start
    }
}

// ---- Entry points ------------------------------------------------------------

/// Reads `text` as one `T` ([`FromJson`]), with the default [`Limits`] and
/// duplicate keys refused. The whole text must be that one value.
///
/// # Errors
///
/// The [`DecodeError`] of `T`, or the parse error of the text (including
/// the rest of the document after the value).
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
///
/// let scores: BTreeMap<String, Vec<u32>> = jsonx_lib::from_str(r#"{"ada": [3, 5]}"#)?;
/// assert_eq!(scores["ada"], [3, 5]);
///
/// let err = jsonx_lib::from_str::<u8>("300").unwrap_err();
/// assert_eq!(err.to_string(), "number out of range for u8 at byte 0");
/// # Ok::<(), jsonx_lib::DecodeError>(())
/// ```
pub fn from_str<'de, T: FromJson<'de>>(text: &'de str) -> Result<T, DecodeError> {
    Parser::new().decode(text)
}

/// Reads `bytes`, which must be UTF-8, as one `T`; see [`from_str`].
///
/// # Errors
///
/// As [`from_str`], plus [`ErrorKind::InvalidUtf8`] and
/// [`ErrorKind::InputTooLarge`] as for [`parse_bytes`](crate::parse_bytes).
///
/// # Examples
///
/// ```
/// let pair: (bool, u8) = jsonx_lib::from_bytes(b"[true, 2]")?;
/// assert_eq!(pair, (true, 2));
/// # Ok::<(), jsonx_lib::DecodeError>(())
/// ```
pub fn from_bytes<'de, T: FromJson<'de>>(bytes: &'de [u8]) -> Result<T, DecodeError> {
    Parser::new().decode_bytes(bytes)
}

/// Reads an already built [`Value`] as a `T`, for a type that owns its data.
///
/// The value is written as compact text and read back, so this costs a write
/// and a scan; byte offsets in an error point into that text (the path is
/// the useful part). The default [`Limits`] apply, which bounds the
/// recursion of a deeply nested type.
///
/// # Errors
///
/// The [`DecodeError`] of `T`.
///
/// # Examples
///
/// ```
/// let v = jsonx_lib::json!({ "list": [1, 2] });
/// let list: Vec<u8> = jsonx_lib::from_value(&v["list"])?;
/// assert_eq!(list, [1, 2]);
/// # Ok::<(), jsonx_lib::DecodeError>(())
/// ```
pub fn from_value<T: for<'de> FromJson<'de>>(value: &Value<'_>) -> Result<T, DecodeError> {
    let text = crate::to_string(value);
    from_str(&text)
}

/// Builds a [`Value`] from any [`ToJson`] type, by writing it and reading it
/// back (members exactly as written, duplicates included).
///
/// # Examples
///
/// ```
/// let v = jsonx_lib::to_value(&(1, "two", [true]));
/// assert_eq!(v[2][0], true);
/// ```
#[must_use]
pub fn to_value<T: ToJson + ?Sized>(value: &T) -> Value<'static> {
    let text = crate::to_string(value);
    Parser::new()
        .limits(Limits::unlimited())
        .duplicate_keys(DuplicateKeys::KeepAll)
        .parse(&text)
        .map(Value::into_owned)
        .unwrap_or_default()
}

impl Parser {
    /// Reads `text` as one `T` with this parser's budgets and duplicate-key
    /// policy; see [`from_str`].
    ///
    /// # Errors
    ///
    /// As [`from_str`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Limits, Parser};
    ///
    /// let shallow = Parser::new().limits(Limits::new().with_max_depth(1));
    /// assert_eq!(shallow.decode::<Vec<u8>>("[1]")?, [1]);
    /// let err = shallow.decode::<Vec<Vec<u8>>>("[[1]]").unwrap_err();
    /// assert_eq!(err.syntax_error().map(|e| e.kind()), Some(ErrorKind::DepthLimitExceeded));
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn decode<'de, T: FromJson<'de>>(&self, text: &'de str) -> Result<T, DecodeError> {
        let mut reader = self.reader(text);
        let value = decode_value(&mut reader)?;
        reader.read_to_end()?;
        Ok(value)
    }

    /// Reads `bytes`, which must be UTF-8, as one `T`; see
    /// [`decode`](Self::decode).
    ///
    /// # Errors
    ///
    /// As [`from_bytes`].
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::Parser::new().decode_bytes::<&str>(b"\"x\"")?, "x");
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn decode_bytes<'de, T: FromJson<'de>>(&self, bytes: &'de [u8]) -> Result<T, DecodeError> {
        let mut reader = self.reader_bytes(bytes)?;
        let value = decode_value(&mut reader)?;
        reader.read_to_end()?;
        Ok(value)
    }
}

// ---- Nullable ----------------------------------------------------------------

/// A member that may be absent, `null`, or a value, keeping the three apart.
///
/// LSP and MCP use both TypeScript's optional `key?: T` and nullable
/// `key: T | null`, and sometimes `key?: T | null`, where "left out" and
/// "`null`" mean different things (`null` often clears a setting). An
/// `Option<T>` reads both as `None`; a `Nullable<T>` tells them apart, and
/// writes back what it read: [`ObjectWriter::optional`](crate::ObjectWriter::optional)
/// leaves out `Absent` and writes `Null` as `null`.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{DecodeError, FromJson, Nullable, Reader, ToJson, ValueWriter, Written};
///
/// struct Settings { tab_size: Nullable<u8> }
///
/// impl<'de> FromJson<'de> for Settings {
///     fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
///         let mut tab_size = None;
///         let mut o = reader.read_object()?;
///         while let Some(key) = o.next_key()? {
///             if key == "tabSize" { o.field(&mut tab_size)?; }
///         }
///         Ok(Settings { tab_size: o.required(tab_size, "tabSize")? })
///     }
/// }
///
/// impl ToJson for Settings {
///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
///         out.object(|o| o.optional("tabSize", &self.tab_size))
///     }
/// }
///
/// for text in ["{}", r#"{"tabSize":null}"#, r#"{"tabSize":4}"#] {
///     let s: Settings = jsonx_lib::from_str(text)?;
///     assert_eq!(jsonx_lib::to_string(&s), text);
/// }
/// # Ok::<(), DecodeError>(())
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Nullable<T> {
    /// The member is not there.
    #[default]
    Absent,
    /// The member is there, and `null`.
    Null,
    /// The member is there, with a value.
    Value(T),
}

impl<T> Nullable<T> {
    /// The value, if there is one: `Absent` and `Null` are both `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Nullable;
    ///
    /// assert_eq!(Nullable::Value(3).into_option(), Some(3));
    /// assert_eq!(Nullable::<u8>::Null.into_option(), None);
    /// ```
    #[inline]
    pub fn into_option(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Absent | Self::Null => None,
        }
    }

    /// A reference to the value, if there is one.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::Nullable::Value(3).as_ref(), jsonx_lib::Nullable::Value(&3));
    /// ```
    #[inline]
    pub const fn as_ref(&self) -> Nullable<&T> {
        match self {
            Self::Value(value) => Nullable::Value(value),
            Self::Null => Nullable::Null,
            Self::Absent => Nullable::Absent,
        }
    }

    /// Whether the member is there with `null`.
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(jsonx_lib::Nullable::<u8>::Null.is_null());
    /// ```
    #[inline]
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl<'de, T: FromJson<'de>> FromJson<'de> for Nullable<T> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        if reader.peek_kind()? == Some(Kind::Null) {
            let _null = reader.next_event()?;
            Ok(Self::Null)
        } else {
            decode_value(reader).map(Self::Value)
        }
    }

    #[inline]
    fn absent() -> Option<Self> {
        Some(Self::Absent)
    }
}

impl<T: ToJson> ToJson for Nullable<T> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        match self {
            Self::Value(value) => value.to_json(out),
            Self::Absent | Self::Null => out.null(),
        }
    }

    #[inline]
    fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

// ---- Implementations ---------------------------------------------------------

impl<'de> FromJson<'de> for bool {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        match reader.scalar("a boolean")? {
            (Event::Bool(b), _) => Ok(b),
            (other, at) => Err(DecodeError::wrong_type("a boolean", event_kind(&other), at)),
        }
    }
}

/// Reads a number value for the target type `name`.
fn number<'de>(
    reader: &mut Reader<'de>,
    name: &'static str,
) -> Result<(Number<'de>, usize), DecodeError> {
    match reader.scalar(name)? {
        (Event::Number(n), at) => Ok((n, at)),
        (other, at) => Err(DecodeError::wrong_type(name, event_kind(&other), at)),
    }
}

macro_rules! from_json_integer {
    ($($t:ty => $via:ident),* $(,)?) => {$(
        impl<'de> FromJson<'de> for $t {
            fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
                let lenient = reader.number_policy() == Numbers::WholeValues;
                let (n, at) = number(reader, stringify!($t))?;
                n.$via()
                    .and_then(|wide| <$t>::try_from(wide).ok())
                    .or_else(|| {
                        lenient.then(|| n.whole_value()).flatten().and_then(|w| <$t>::try_from(w).ok())
                    })
                    .ok_or_else(|| {
                        DecodeError::new(DecodeErrorKind::OutOfRange { expected: stringify!($t) }, at)
                    })
            }
        }
    )*};
}

from_json_integer!(
    i8 => as_i64, i16 => as_i64, i32 => as_i64, i64 => as_i64, isize => as_i128, i128 => as_i128,
    u8 => as_u64, u16 => as_u64, u32 => as_u64, u64 => as_u64, usize => as_u128, u128 => as_u128,
);

impl<'de> FromJson<'de> for f64 {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let (n, at) = number(reader, "f64")?;
        n.as_f64()
            .ok_or_else(|| DecodeError::new(DecodeErrorKind::OutOfRange { expected: "f64" }, at))
    }
}

impl<'de> FromJson<'de> for f32 {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let (n, at) = number(reader, "f32")?;
        n.as_f32()
            .ok_or_else(|| DecodeError::new(DecodeErrorKind::OutOfRange { expected: "f32" }, at))
    }
}

/// Reads a string value for the target type `name`.
fn string<'de>(
    reader: &mut Reader<'de>,
    name: &'static str,
) -> Result<(Cow<'de, str>, usize), DecodeError> {
    match reader.scalar(name)? {
        (Event::String(s), at) => Ok((s, at)),
        (other, at) => Err(DecodeError::wrong_type(name, event_kind(&other), at)),
    }
}

impl<'de> FromJson<'de> for Cow<'de, str> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        string(reader, "a string").map(|(s, _)| s)
    }
}

impl<'de> FromJson<'de> for &'de str {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        match string(reader, "a string")? {
            (Cow::Borrowed(s), _) => Ok(s),
            (Cow::Owned(_), at) => Err(DecodeError::new(DecodeErrorKind::StringNotBorrowable, at)),
        }
    }
}

impl<'de> FromJson<'de> for String {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        string(reader, "a string").map(|(s, _)| s.into_owned())
    }
}

impl<'de> FromJson<'de> for Box<str> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        String::from_json(reader).map(String::into_boxed_str)
    }
}

impl<'de> FromJson<'de> for char {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let (s, at) = string(reader, "a character")?;
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(DecodeError::invalid_value("a single character", at)),
        }
    }
}

impl<'de> FromJson<'de> for () {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        match reader.scalar("null")? {
            (Event::Null, _) => Ok(()),
            (other, at) => Err(DecodeError::wrong_type("null", event_kind(&other), at)),
        }
    }
}

impl<'de, T: FromJson<'de>> FromJson<'de> for Option<T> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        if reader.peek_kind()? == Some(Kind::Null) {
            let _null = reader.next_event()?;
            Ok(None)
        } else {
            decode_value(reader).map(Some)
        }
    }

    #[inline]
    fn absent() -> Option<Self> {
        Some(None)
    }
}

impl<'de, T: FromJson<'de>> FromJson<'de> for Box<T> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        T::from_json(reader).map(Box::new)
    }

    fn absent() -> Option<Self> {
        T::absent().map(Box::new)
    }
}

impl<'de, T: FromJson<'de>> FromJson<'de> for Rc<T> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        T::from_json(reader).map(Rc::new)
    }

    fn absent() -> Option<Self> {
        T::absent().map(Rc::new)
    }
}

#[cfg(target_has_atomic = "ptr")]
impl<'de, T: FromJson<'de>> FromJson<'de> for Arc<T> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        T::from_json(reader).map(Arc::new)
    }

    fn absent() -> Option<Self> {
        T::absent().map(Arc::new)
    }
}

/// Reads every item of an array into `push`.
fn items<'de, T: FromJson<'de>>(
    reader: &mut Reader<'de>,
    expected: &'static str,
    mut push: impl FnMut(T),
) -> Result<(), DecodeError> {
    let start = reader.open(Kind::Array, expected)?;
    let mut array = ArrayReader {
        reader,
        start,
        index: 0,
        done: false,
    };
    while let Some(item) = array.next_item()? {
        push(item);
    }
    Ok(())
}

impl<'de, T: FromJson<'de>> FromJson<'de> for Vec<T> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let mut out = Vec::new();
        items(reader, "an array", |item| out.push(item))?;
        Ok(out)
    }
}

impl<'de, T: FromJson<'de>> FromJson<'de> for Box<[T]> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        Vec::from_json(reader).map(Vec::into_boxed_slice)
    }
}

impl<'de, T: FromJson<'de>> FromJson<'de> for VecDeque<T> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let mut out = VecDeque::new();
        items(reader, "an array", |item| out.push_back(item))?;
        Ok(out)
    }
}

impl<'de, T: FromJson<'de> + Ord> FromJson<'de> for BTreeSet<T> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let mut out = BTreeSet::new();
        items(reader, "an array", |item| {
            let _new = out.insert(item);
        })?;
        Ok(out)
    }
}

#[cfg(feature = "std")]
impl<'de, T, S> FromJson<'de> for std::collections::HashSet<T, S>
where
    T: FromJson<'de> + Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Default,
{
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let mut out = Self::default();
        items(reader, "an array", |item| {
            let _new = out.insert(item);
        })?;
        Ok(out)
    }
}

impl<'de, T: FromJson<'de>, const N: usize> FromJson<'de> for [T; N] {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let mut array = reader.read_array()?;
        let mut out = Vec::with_capacity(N.min(1024));
        while out.len() < N {
            match array.next_item()? {
                Some(item) => out.push(item),
                None => {
                    return Err(DecodeError::new(
                        DecodeErrorKind::WrongLength {
                            expected: N,
                            found: out.len(),
                        },
                        array.offset(),
                    ));
                }
            }
        }
        array.end()?;
        out.try_into().map_err(|_unreachable: Vec<T>| {
            DecodeError::new(
                DecodeErrorKind::WrongLength {
                    expected: N,
                    found: N,
                },
                0,
            )
        })
    }
}

macro_rules! from_json_tuple {
    ($(($len:literal: $($name:ident),+))*) => {$(
        impl<'de, $($name: FromJson<'de>),+> FromJson<'de> for ($($name,)+) {
            fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
                let mut array = reader.read_array()?;
                let tuple = ($(
                    match array.next_item::<$name>()? {
                        Some(item) => item,
                        None => {
                            return Err(DecodeError::new(
                                DecodeErrorKind::WrongLength { expected: $len, found: array.index() },
                                array.offset(),
                            ));
                        }
                    },
                )+);
                array.end()?;
                Ok(tuple)
            }
        }
    )*};
}

from_json_tuple! {
    (1: A)
    (2: A, B)
    (3: A, B, C)
    (4: A, B, C, D)
    (5: A, B, C, D, E)
    (6: A, B, C, D, E, F)
    (7: A, B, C, D, E, F, G)
    (8: A, B, C, D, E, F, G, H)
}

/// A map a JSON object can be read into.
trait Map<K, V> {
    fn has(&self, key: &K) -> bool;
    fn put(&mut self, key: K, value: V);
}

impl<K: Ord, V> Map<K, V> for BTreeMap<K, V> {
    fn has(&self, key: &K) -> bool {
        self.contains_key(key)
    }
    fn put(&mut self, key: K, value: V) {
        let _old = self.insert(key, value);
    }
}

#[cfg(feature = "std")]
impl<K: Eq + core::hash::Hash, V, S: core::hash::BuildHasher> Map<K, V>
    for std::collections::HashMap<K, V, S>
{
    fn has(&self, key: &K) -> bool {
        self.contains_key(key)
    }
    fn put(&mut self, key: K, value: V) {
        let _old = self.insert(key, value);
    }
}

/// Reads every member of an object into `map`. A key that repeats one
/// already in the map (two spellings of one key, or two texts one
/// [`FromJsonKey`] makes equal) follows the reader's policy: refused under
/// `Error`, the later value skipped under `First`, kept under `Last` and
/// `KeepAll`.
fn members<'de, K: FromJsonKey<'de>, V: FromJson<'de>>(
    reader: &mut Reader<'de>,
    map: &mut impl Map<K, V>,
) -> Result<(), DecodeError> {
    let policy = reader.duplicate_policy();
    let mut object = reader.read_object()?;
    while let Some(text) = object.next_key()? {
        let key = K::from_json_key(text)
            .map_err(|kind| object.in_member(DecodeError::new(kind, object.key_offset)))?;
        if map.has(&key) {
            match policy {
                DuplicateKeys::First => {
                    object.skip()?;
                    continue;
                }
                DuplicateKeys::Last | DuplicateKeys::KeepAll => {}
                _ => {
                    let e = DecodeError::new(DecodeErrorKind::DuplicateField, object.key_offset);
                    return Err(object.in_member(e));
                }
            }
        }
        let value = object.value()?;
        map.put(key, value);
    }
    Ok(())
}

impl<'de, K: FromJsonKey<'de> + Ord, V: FromJson<'de>> FromJson<'de> for BTreeMap<K, V> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let mut out = BTreeMap::new();
        members(reader, &mut out)?;
        Ok(out)
    }
}

#[cfg(feature = "std")]
impl<'de, K, V, S> FromJson<'de> for std::collections::HashMap<K, V, S>
where
    K: FromJsonKey<'de> + Eq + core::hash::Hash,
    V: FromJson<'de>,
    S: core::hash::BuildHasher + Default,
{
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        let mut out = Self::default();
        members(reader, &mut out)?;
        Ok(out)
    }
}

impl<'de> FromJsonKey<'de> for String {
    #[inline]
    fn from_json_key(key: Cow<'de, str>) -> Result<Self, DecodeErrorKind> {
        Ok(key.into_owned())
    }
}

impl<'de> FromJsonKey<'de> for Box<str> {
    #[inline]
    fn from_json_key(key: Cow<'de, str>) -> Result<Self, DecodeErrorKind> {
        Ok(key.into_owned().into_boxed_str())
    }
}

impl<'de> FromJsonKey<'de> for Cow<'de, str> {
    #[inline]
    fn from_json_key(key: Cow<'de, str>) -> Result<Self, DecodeErrorKind> {
        Ok(key)
    }
}

impl<'de> FromJsonKey<'de> for &'de str {
    #[inline]
    fn from_json_key(key: Cow<'de, str>) -> Result<Self, DecodeErrorKind> {
        match key {
            Cow::Borrowed(key) => Ok(key),
            Cow::Owned(_) => Err(DecodeErrorKind::StringNotBorrowable),
        }
    }
}

impl<'de> FromJson<'de> for Value<'de> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        match reader.read_value()? {
            Some(value) => Ok(value),
            None => Err(DecodeError::new(
                DecodeErrorKind::Syntax(ErrorKind::ExpectedValue),
                reader.offset(),
            )),
        }
    }
}

impl<'de> FromJson<'de> for Number<'de> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        number(reader, "a number").map(|(n, _)| n)
    }
}

/// Reads a whole value of `kind` as a [`Value`].
fn tree<'de>(
    reader: &mut Reader<'de>,
    kind: Kind,
    expected: &'static str,
) -> Result<Value<'de>, DecodeError> {
    match reader.peek_kind()? {
        Some(found) if found != kind => {
            Err(DecodeError::wrong_type(expected, found, reader.offset()))
        }
        _ => Value::from_json(reader),
    }
}

impl<'de> FromJson<'de> for Array<'de> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        match tree(reader, Kind::Array, "an array")? {
            Value::Array(array) => Ok(array),
            other => Err(DecodeError::wrong_type(
                "an array",
                other.kind(),
                reader.event_offset(),
            )),
        }
    }
}

impl<'de> FromJson<'de> for Object<'de> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        match tree(reader, Kind::Object, "an object")? {
            Value::Object(object) => Ok(object),
            other => Err(DecodeError::wrong_type(
                "an object",
                other.kind(),
                reader.event_offset(),
            )),
        }
    }
}
