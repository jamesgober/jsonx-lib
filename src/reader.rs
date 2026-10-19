//! The pull reader: a document as a stream of events, without building a tree.

use alloc::borrow::Cow;
use core::iter::FusedIterator;

use crate::{
    DuplicateKeys, Kind, Limits, Number, Value,
    error::{Error, ErrorKind},
    machine::Machine,
    parser,
};

/// One step through a document, from [`Reader::next_event`].
///
/// A document is one value. A scalar value is one event; an array is
/// [`StartArray`](Self::StartArray), its items, then [`EndArray`](Self::EndArray);
/// an object is [`StartObject`](Self::StartObject), then for each member a
/// [`Key`](Self::Key) followed by the member's value, then
/// [`EndObject`](Self::EndObject).
///
/// # Examples
///
/// ```
/// use jsonx_lib::{Event, Reader};
///
/// let events: Vec<Event> = Reader::new(r#"{"a": [true]}"#).collect::<Result<_, _>>()?;
/// assert_eq!(events, [
///     Event::StartObject,
///     Event::Key("a".into()),
///     Event::StartArray,
///     Event::Bool(true),
///     Event::EndArray,
///     Event::EndObject,
/// ]);
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// `{`: an object begins.
    StartObject,
    /// `}`: the innermost open object ends.
    EndObject,
    /// `[`: an array begins.
    StartArray,
    /// `]`: the innermost open array ends.
    EndArray,
    /// A member's key (the `:` after it has been read too). Borrowed unless it
    /// had escapes.
    Key(Cow<'a, str>),
    /// A string value. Borrowed unless it had escapes.
    String(Cow<'a, str>),
    /// A number value, as its exact text.
    Number(Number<'a>),
    /// `true` or `false`.
    Bool(bool),
    /// `null`.
    Null,
}

/// A pull parser: reads a document one [`Event`] at a time, building nothing.
///
/// The reader is the layer every parse in this crate goes through: it checks
/// the full RFC 8259 grammar and every budget in [`Limits`] as it goes, with
/// the same errors and byte offsets as [`parse`](crate::parse). Use it to pick
/// a few fields out of a large document, to stream a document's contents
/// somewhere without holding a tree, or to keep part of a message as raw text
/// to be read later ([`read_raw`](Self::read_raw)).
///
/// The reader works over a complete document held in memory, and borrows
/// strings and numbers from it exactly as [`parse`](crate::parse) does.
/// Reading is iterative: depth costs one byte of reader state per level, never
/// stack.
///
/// **Duplicate keys.** Under [`DuplicateKeys::Error`], the default, the
/// reader refuses an object that repeats a key, whichever way it is read:
/// event by event, through [`read_raw`](Self::read_raw), or through
/// [`read_value`](Self::read_value). The check happens when the object
/// closes, so the `}` of an object with a repeated key is never returned as
/// [`Event::EndObject`]: the reader returns [`ErrorKind::DuplicateKey`] at
/// the repeat's opening quote instead. A loop that collects members and acts
/// only after the object ended (as below) can therefore never act on a
/// smuggled second value. To check, the reader keeps each open object's keys
/// (a slice of the input, or a decoded copy for a key with escapes) until
/// the object closes. Under the other policies the reader passes every
/// member through as written; [`read_value`](Self::read_value) applies
/// `First` and `Last` to the values it builds.
///
/// After an error the reader is finished: every later call returns the same
/// error.
///
/// # Examples
///
/// Read a JSON-RPC envelope, keeping `params` as raw text for later:
///
/// ```
/// use jsonx_lib::{Event, Reader};
///
/// let message = r#"{"jsonrpc": "2.0", "id": 4, "method": "hover", "params": {"line": 3}}"#;
/// let mut reader = Reader::new(message);
/// let (mut method, mut params) = (None, None);
/// assert_eq!(reader.next_event()?, Some(Event::StartObject));
/// while let Some(Event::Key(key)) = reader.next_event()? {
///     match &*key {
///         "method" => method = reader.read_value()?,
///         "params" => params = reader.read_raw()?,
///         _ => { let _skipped = reader.read_raw()?; }
///     }
/// }
/// reader.finish()?; // the rest of the document must be valid too
/// assert_eq!(method.as_ref().and_then(|m| m.as_str()), Some("hover"));
/// assert_eq!(params, Some(r#"{"line": 3}"#));
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    src: &'a str,
    machine: Machine,
    failed: Option<Error>,
}

impl<'a> Reader<'a> {
    /// A reader over `text` with the default [`Limits`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Event, Reader};
    ///
    /// let mut r = Reader::new("  42 ");
    /// assert_eq!(r.next_event()?.map(|e| e == Event::Number(42.into())), Some(true));
    /// assert_eq!(r.next_event()?, None);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        Self::with_config(text, Limits::new(), DuplicateKeys::Error)
    }

    /// A reader with the given budgets and duplicate-key policy; see
    /// [`Parser::reader`](crate::Parser::reader).
    pub(crate) fn with_config(text: &'a str, limits: Limits, duplicates: DuplicateKeys) -> Self {
        let failed = (text.len() > limits.max_input_bytes)
            .then(|| Error::new(ErrorKind::InputTooLarge, limits.max_input_bytes));
        Self {
            src: text,
            machine: Machine::new(limits, duplicates, true),
            failed,
        }
    }

    /// Reads the next event, or `Ok(None)` once the document's value is
    /// complete and nothing but whitespace follows it.
    ///
    /// # Errors
    ///
    /// The first syntax error or exceeded budget, at its byte offset (see
    /// [`Error`]). Once an error is returned, every later call returns it too.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Event, Reader};
    ///
    /// let mut r = Reader::new("[1,]");
    /// assert_eq!(r.next_event()?, Some(Event::StartArray));
    /// assert!(r.next_event()?.is_some()); // 1
    /// let err = r.next_event().unwrap_err();
    /// assert_eq!((err.kind(), err.offset()), (ErrorKind::TrailingComma, 3));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn next_event(&mut self) -> Result<Option<Event<'a>>, Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        self.machine
            .step(self.src)
            .inspect_err(|&e| self.failed = Some(e))
    }

    /// Reads the next value whole and returns its exact source text, without
    /// decoding or copying it.
    ///
    /// Call it where a value comes next: at the start, after a [`Key`](Event::Key),
    /// or between array items. The value is fully validated. If no value comes
    /// next (the current array or object ends, or the document already ended),
    /// returns `Ok(None)` and consumes nothing, so the next
    /// [`next_event`](Self::next_event) returns the end event as usual. Calling
    /// it where a key comes next also returns `Ok(None)`.
    ///
    /// The text is a slice of the input: it can be stored, compared, or parsed
    /// later, as `serde_json`'s `RawValue` is used. Nothing of the value is
    /// allocated; the reader's own state grows with the value's nesting (a
    /// byte per level) and, under [`DuplicateKeys::Error`], with the keys of
    /// its open objects.
    ///
    /// # Errors
    ///
    /// As [`next_event`](Self::next_event), for any error inside the value,
    /// including a repeated key under [`DuplicateKeys::Error`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Event, Reader};
    ///
    /// let mut r = Reader::new(r#"[{"deep": [1, 2]}, "x"]"#);
    /// assert_eq!(r.next_event()?, Some(Event::StartArray));
    /// assert_eq!(r.read_raw()?, Some(r#"{"deep": [1, 2]}"#));
    /// assert_eq!(r.read_raw()?, Some(r#""x""#));
    /// assert_eq!(r.read_raw()?, None); // the array ends here
    /// assert_eq!(r.next_event()?, Some(Event::EndArray));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn read_raw(&mut self) -> Result<Option<&'a str>, Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        if !self.machine.at_value(self.src)? {
            return Ok(None);
        }
        self.machine.skipping = true;
        let result = self.skip_value();
        self.machine.skipping = false;
        result.map(Some)
    }

    /// Reads the next value whole and builds it as a [`Value`], applying the
    /// reader's [`DuplicateKeys`] policy.
    ///
    /// Positioning is as for [`read_raw`](Self::read_raw): `Ok(None)`, with
    /// nothing consumed, when no value comes next.
    ///
    /// # Errors
    ///
    /// As [`next_event`](Self::next_event), plus [`ErrorKind::DuplicateKey`]
    /// under [`DuplicateKeys::Error`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Event, Reader};
    ///
    /// let mut r = Reader::new(r#"{"skip": [0], "keep": {"a": 1}}"#);
    /// r.next_event()?; // {
    /// r.next_event()?; // "skip"
    /// r.read_raw()?;
    /// assert_eq!(r.next_event()?, Some(Event::Key("keep".into())));
    /// let keep = r.read_value()?.unwrap_or_default();
    /// assert_eq!(keep["a"], 1);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn read_value(&mut self) -> Result<Option<Value<'a>>, Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        if !self.machine.at_value(self.src)? {
            return Ok(None);
        }
        let duplicates = self.machine.duplicates;
        parser::build(self, duplicates)
            .inspect_err(|&e| self.failed = Some(e))
            .map(Some)
    }

    /// The kind of the next value, without consuming anything: `None` when
    /// no value comes next (positioning as for [`read_raw`](Self::read_raw)),
    /// or when the next byte cannot start one (the next
    /// [`next_event`](Self::next_event) reports why). Decided by the value's
    /// first byte alone, so `tru` peeks as [`Kind::Bool`].
    ///
    /// Useful for a value that may be one of several shapes, such as LSP's
    /// `string | number` ids.
    ///
    /// # Errors
    ///
    /// Any error the reader already stopped at.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Kind, Reader};
    ///
    /// let mut r = Reader::new(r#"[7, "seven"]"#);
    /// r.next_event()?;
    /// assert_eq!(r.peek_kind()?, Some(Kind::Number));
    /// r.read_raw()?;
    /// assert_eq!(r.peek_kind()?, Some(Kind::String));
    /// r.read_raw()?;
    /// assert_eq!(r.peek_kind()?, None); // the array ends
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn peek_kind(&mut self) -> Result<Option<Kind>, Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        Ok(if self.machine.at_value(self.src)? {
            self.machine.peek_kind(self.src)
        } else {
            None
        })
    }

    /// Reads to the end of the document, checking that the rest of it is
    /// valid and that nothing but whitespace follows its value.
    ///
    /// # Errors
    ///
    /// The first error in the rest of the document.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Event, Reader};
    ///
    /// let mut r = Reader::new(r#"{"id": 1, "x": [} "#);
    /// assert_eq!(r.next_event()?, Some(Event::StartObject));
    /// let err = r.finish().unwrap_err();
    /// assert_eq!((err.kind(), err.offset()), (ErrorKind::ExpectedValue, 16));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn finish(mut self) -> Result<(), Error> {
        while self.next_event()?.is_some() {}
        Ok(())
    }

    /// The number of arrays and objects open at this point.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new("[[]]");
    /// r.next_event()?;
    /// r.next_event()?;
    /// assert_eq!(r.depth(), 2);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn depth(&self) -> usize {
        self.machine.depth()
    }

    /// The byte offset just past the input consumed so far.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new("[10, 20]");
    /// r.next_event()?;
    /// r.next_event()?;
    /// assert_eq!(r.offset(), 3);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn offset(&self) -> usize {
        self.machine.pos
    }

    /// The byte offset where the most recent event's token began: the quote of
    /// a key or string, the first byte of a number or literal, the bracket or
    /// brace of a start or end event. Useful for pointing a diagnostic of your
    /// own at the value it is about.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::Reader::new(r#"{ "a" : 1 }"#);
    /// r.next_event()?;
    /// r.next_event()?;
    /// assert_eq!(r.event_offset(), 2); // the key's opening quote
    /// r.next_event()?;
    /// assert_eq!(r.event_offset(), 8); // the number
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn event_offset(&self) -> usize {
        self.machine.last
    }

    // ---- internals -----------------------------------------------------------

    /// Consumes one whole value (the reader is positioned at it) and returns
    /// its text.
    fn skip_value(&mut self) -> Result<&'a str, Error> {
        let depth = self.machine.depth();
        let first = self.next_event()?;
        let start = self.machine.last;
        if matches!(first, Some(Event::StartArray | Event::StartObject)) {
            while self.machine.depth() > depth {
                if self.next_event()?.is_none() {
                    return Err(Error::new(ErrorKind::UnexpectedEnd, self.src.len()));
                }
            }
        }
        Ok(self.src.get(start..self.machine.pos).unwrap_or(""))
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Result<Event<'a>, Error>;

    /// The next event; after the end or an error, `None`. An error is yielded
    /// once.
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed.is_some() && self.machine.is_done() {
            return None;
        }
        match self.next_event() {
            Ok(Some(event)) => Some(Ok(event)),
            Ok(None) => None,
            Err(e) => {
                // Yield the error once, then stop.
                self.machine.finish_iteration();
                Some(Err(e))
            }
        }
    }
}

impl FusedIterator for Reader<'_> {}
