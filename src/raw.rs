//! [`RawValue`]: one JSON value kept as its validated text.

use alloc::{borrow::Cow, string::String};
use core::fmt;

use crate::{
    DecodeError, DuplicateKeys, Error, FromJson, Limits, Parser, Reader, ToJson, Value,
    ValueWriter, Written, decode::DecodeErrorKind, error::ErrorKind,
};

/// One complete JSON value kept as its exact source text: validated once,
/// then stored, compared, forwarded, or read later.
///
/// This is `serde_json`'s `RawValue`. A JSON-RPC server keeps `params` as a
/// `RawValue` until the handler for the method knows what type to read it
/// as; a proxy forwards a `result` without decoding it.
///
/// A `RawValue` always holds exactly one value with no whitespace around
/// it. Every way of making one validates, or takes text this crate has
/// just validated or written: [`new`](Self::new) and
/// [`from_string`](Self::from_string) check the text,
/// [`Reader::read_raw_value`] and `FromJson` take it from a read that
/// checked it, and [`encode`](Self::encode) takes it from this crate's
/// writer. So writing one ([`ToJson`]) copies the text with no second
/// check, and the output stays valid JSON. Under
/// [`Format::html_safe`](crate::Format::html_safe) or
/// [`Format::ascii`](crate::Format::ascii) the characters those formats
/// escape are escaped inside the text's strings as well (see
/// [`ValueWriter::raw`](crate::ValueWriter::raw)); otherwise the text is
/// copied unchanged.
///
/// Valid means grammatical JSON. A `RawValue` taken under
/// [`DuplicateKeys::KeepAll`](crate::DuplicateKeys::KeepAll), or made by
/// [`encode`](Self::encode) from a `ToJson` that writes a key twice, can
/// hold an object with a repeated key: [`decode`](Self::decode) (default
/// policy) refuses that text, [`parse`](Self::parse) keeps every member.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{DecodeError, FromJson, RawValue, Reader};
///
/// struct Request<'a> { method: &'a str, params: RawValue<'a> }
///
/// impl<'de> FromJson<'de> for Request<'de> {
///     fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
///         let (mut method, mut params) = (None, None);
///         let mut o = reader.read_object()?;
///         while let Some(key) = o.next_key()? {
///             match &*key {
///                 "method" => o.field(&mut method)?,
///                 "params" => o.field(&mut params)?,
///                 _ => {}
///             }
///         }
///         Ok(Request { method: o.required(method, "method")?, params: o.required(params, "params")? })
///     }
/// }
///
/// let req: Request = jsonx_lib::from_str(r#"{"method": "add", "params": [2, 3]}"#)?;
/// assert_eq!(req.params.as_str(), "[2, 3]"); // not decoded yet
/// let (a, b): (i32, i32) = req.params.decode()?; // now, once the method is known
/// assert_eq!(a + b, 5);
/// # Ok::<(), DecodeError>(())
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RawValue<'a>(Cow<'a, str>);

impl<'a> RawValue<'a> {
    /// Validates `text` as one JSON value under the default [`Limits`] and
    /// keeps it, minus surrounding whitespace, as a borrowed slice.
    ///
    /// # Errors
    ///
    /// The parse error of text that is not exactly one JSON value.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::RawValue;
    ///
    /// assert_eq!(RawValue::new(" {\"a\": 1} ")?.as_str(), "{\"a\": 1}");
    /// assert!(RawValue::new("[1,").is_err());
    /// assert!(RawValue::new("1 2").is_err());
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn new(text: &'a str) -> Result<Self, Error> {
        let mut reader = Reader::new(text);
        match reader.read_raw_value()? {
            Some(raw) => {
                reader.read_to_end()?;
                Ok(raw)
            }
            // `read_raw` sees no value only in input that holds none.
            None => Err(Error::new(ErrorKind::UnexpectedEnd, text.len())),
        }
    }

    /// Validates an owned string, as [`new`](Self::new), keeping its buffer
    /// when it has no surrounding whitespace.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    ///
    /// # Examples
    ///
    /// ```
    /// let raw = jsonx_lib::RawValue::from_string(String::from("[true]"))?;
    /// assert_eq!(raw.as_str(), "[true]");
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn from_string(text: String) -> Result<RawValue<'static>, Error> {
        let len = RawValue::new(&text)?.as_str().len();
        Ok(if len == text.len() {
            RawValue(Cow::Owned(text))
        } else {
            RawValue::new(&text)?.into_owned()
        })
    }

    /// Writes `value` as compact JSON and keeps the text.
    ///
    /// # Examples
    ///
    /// ```
    /// let raw = jsonx_lib::RawValue::encode(&[1, 2]);
    /// assert_eq!(raw.as_str(), "[1,2]");
    /// ```
    #[must_use]
    pub fn encode<T: ToJson + ?Sized>(value: &T) -> RawValue<'static> {
        RawValue(Cow::Owned(crate::to_string(value)))
    }

    /// Wraps text this crate has just validated as one value.
    #[inline]
    pub(crate) const fn trusted(text: &'a str) -> Self {
        Self(Cow::Borrowed(text))
    }

    /// The JSON text.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::RawValue::new("null")?.as_str(), "null");
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Copies borrowed text so the value no longer borrows from the input.
    ///
    /// # Examples
    ///
    /// ```
    /// let owned = jsonx_lib::RawValue::new(&String::from("1"))?.into_owned();
    /// assert_eq!(owned.as_str(), "1");
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn into_owned(self) -> RawValue<'static> {
        RawValue(Cow::Owned(self.0.into_owned()))
    }

    /// Parses the text into a [`Value`]. The text was validated when the
    /// `RawValue` was made, so no budget applies, and every member is kept
    /// as written.
    ///
    /// # Errors
    ///
    /// None in practice: the text is valid JSON.
    ///
    /// # Examples
    ///
    /// ```
    /// let raw = jsonx_lib::RawValue::new(r#"{"line": 3}"#)?;
    /// assert_eq!(raw.parse()?["line"], 3);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn parse(&self) -> Result<Value<'_>, Error> {
        Parser::new()
            .limits(Limits::unlimited())
            .duplicate_keys(DuplicateKeys::KeepAll)
            .parse(&self.0)
    }

    /// Reads the text as a `T` ([`FromJson`]), with the default [`Limits`]
    /// (which bound a recursive type's depth) and duplicate keys refused.
    ///
    /// # Errors
    ///
    /// The [`DecodeError`] of `T`; offsets count from the start of the raw
    /// text.
    ///
    /// # Examples
    ///
    /// ```
    /// let raw = jsonx_lib::RawValue::new("[1, 2, 3]")?;
    /// assert_eq!(raw.decode::<Vec<u8>>()?, [1, 2, 3]);
    /// # Ok::<(), jsonx_lib::DecodeError>(())
    /// ```
    pub fn decode<'s, T: FromJson<'s>>(&'s self) -> Result<T, DecodeError> {
        Parser::new()
            .limits(Limits::new().with_max_input_bytes(usize::MAX))
            .decode(&self.0)
    }
}

impl fmt::Display for RawValue<'_> {
    /// Writes the JSON text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for RawValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> FromJson<'de> for RawValue<'de> {
    fn from_json(reader: &mut Reader<'de>) -> Result<Self, DecodeError> {
        match reader.read_raw_value()? {
            Some(raw) => Ok(raw),
            None => Err(DecodeError::new(
                DecodeErrorKind::Syntax(ErrorKind::ExpectedValue),
                reader.offset(),
            )),
        }
    }
}

impl ToJson for RawValue<'_> {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.raw(self)
    }
}
