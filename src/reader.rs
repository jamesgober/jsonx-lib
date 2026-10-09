//! The pull reader: a document as a stream of events, without building a tree.

use alloc::{borrow::Cow, vec::Vec};
use core::iter::FusedIterator;

use crate::{
    DuplicateKeys, Limits, Number, Value,
    error::{Error, ErrorKind},
    lexer, parser, scan,
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

/// What the reader expects next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// A value: at the start, or after a member's `:`.
    Value,
    /// A value after `,` in an array; `]` here is a trailing comma.
    Element,
    /// Just after `[`: a value or `]`.
    ArrayFirst,
    /// After an array item: `,` or `]`.
    ArrayNext,
    /// Just after `{`: a key or `}`.
    ObjectFirst,
    /// After `,` in an object: a key; `}` here is a trailing comma.
    Key,
    /// After a member's value: `,` or `}`.
    ObjectNext,
    /// The document's value is complete: only whitespace may follow.
    End,
    /// The end was reached and reported.
    Done,
}

/// Which container is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    Array,
    Object,
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
/// stack. The reader does not look for duplicate keys, which needs memory
/// proportional to an object's size; [`read_value`](Self::read_value) applies
/// the configured [`DuplicateKeys`] policy to the values it builds.
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
    pos: usize,
    /// Where the most recent event's token began.
    last: usize,
    state: State,
    stack: Vec<Container>,
    elements: usize,
    limits: Limits,
    duplicates: DuplicateKeys,
    failed: Option<Error>,
    /// Set while [`Reader::read_raw`] skips a value: strings are not decoded.
    skipping: bool,
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
            pos: 0,
            last: 0,
            state: State::Value,
            stack: Vec::new(),
            elements: 0,
            limits,
            duplicates,
            failed,
            skipping: false,
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
        self.step().inspect_err(|&e| self.failed = Some(e))
    }

    /// Reads the next value whole and returns its exact source text, without
    /// decoding or allocating anything.
    ///
    /// Call it where a value comes next: at the start, after a [`Key`](Event::Key),
    /// or between array items. The value is fully validated. If no value comes
    /// next (the current array or object ends, or the document already ended),
    /// returns `Ok(None)` and consumes nothing, so the next
    /// [`next_event`](Self::next_event) returns the end event as usual. Calling
    /// it where a key comes next also returns `Ok(None)`.
    ///
    /// The text is a slice of the input: it can be stored, compared, or parsed
    /// later, as `serde_json`'s `RawValue` is used.
    ///
    /// # Errors
    ///
    /// As [`next_event`](Self::next_event), for any error inside the value.
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
        if !self.at_value()? {
            return Ok(None);
        }
        self.skipping = true;
        let result = self.skip_value();
        self.skipping = false;
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
        if !self.at_value()? {
            return Ok(None);
        }
        let duplicates = self.duplicates;
        parser::build(self, duplicates)
            .inspect_err(|&e| self.failed = Some(e))
            .map(Some)
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
        self.stack.len()
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
        self.pos
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
        self.last
    }

    // ---- internals -----------------------------------------------------------

    #[inline]
    fn bytes(&self) -> &'a [u8] {
        self.src.as_bytes()
    }

    #[inline]
    fn skip_whitespace(&mut self) {
        let bytes = self.bytes();
        while bytes
            .get(self.pos)
            .copied()
            .is_some_and(scan::is_whitespace)
        {
            self.pos += 1;
        }
    }

    #[inline]
    fn end_error(&self) -> Error {
        Error::new(ErrorKind::UnexpectedEnd, self.src.len())
    }

    /// Moves past whitespace and a separating `,` and reports whether a value
    /// comes next, without consuming any token of the value.
    fn at_value(&mut self) -> Result<bool, Error> {
        self.skip_whitespace();
        let next = self.bytes().get(self.pos).copied();
        Ok(match self.state {
            State::Value | State::Element => true,
            State::ArrayFirst => next != Some(b']'),
            State::ArrayNext if next == Some(b',') => {
                self.pos += 1;
                self.state = State::Element;
                true
            }
            _ => false,
        })
    }

    /// Consumes one whole value (the reader is positioned at it) and returns
    /// its text.
    fn skip_value(&mut self) -> Result<&'a str, Error> {
        let depth = self.stack.len();
        let first = self.next_event()?;
        let start = self.last;
        if matches!(first, Some(Event::StartArray | Event::StartObject)) {
            while self.stack.len() > depth {
                if self.next_event()?.is_none() {
                    return Err(self.end_error());
                }
            }
        }
        Ok(self.src.get(start..self.pos).unwrap_or(""))
    }

    /// The state after a value completes in the current container.
    #[inline]
    fn after_value(&mut self) {
        self.state = match self.stack.last() {
            None => State::End,
            Some(Container::Array) => State::ArrayNext,
            Some(Container::Object) => State::ObjectNext,
        };
    }

    /// Closes the innermost container, whose closing byte is at `at`.
    #[inline]
    fn close(&mut self, at: usize, event: Event<'a>) -> Event<'a> {
        self.pos = at + 1;
        self.last = at;
        let _closed = self.stack.pop();
        self.after_value();
        event
    }

    fn step(&mut self) -> Result<Option<Event<'a>>, Error> {
        loop {
            self.skip_whitespace();
            let at = self.pos;
            let next = self.bytes().get(at).copied();
            let err = |kind| Err(Error::new(kind, at));
            match self.state {
                State::Value | State::Element | State::ArrayFirst => match next {
                    None => return Err(self.end_error()),
                    Some(b']') if self.state == State::ArrayFirst => {
                        return Ok(Some(self.close(at, Event::EndArray)));
                    }
                    Some(b']') if self.state == State::Element => {
                        return err(ErrorKind::TrailingComma);
                    }
                    Some(b) => return self.value(b, at).map(Some),
                },
                State::ArrayNext => match next {
                    None => return Err(self.end_error()),
                    Some(b',') => {
                        self.pos += 1;
                        self.state = State::Element;
                    }
                    Some(b']') => return Ok(Some(self.close(at, Event::EndArray))),
                    Some(_) => return err(ErrorKind::ExpectedCommaOrBracket),
                },
                State::ObjectFirst | State::Key => match next {
                    None => return Err(self.end_error()),
                    Some(b'"') => return self.key(at).map(Some),
                    Some(b'}') if self.state == State::ObjectFirst => {
                        return Ok(Some(self.close(at, Event::EndObject)));
                    }
                    Some(b'}') => return err(ErrorKind::TrailingComma),
                    Some(_) => return err(ErrorKind::ExpectedKey),
                },
                State::ObjectNext => match next {
                    None => return Err(self.end_error()),
                    Some(b',') => {
                        self.pos += 1;
                        self.state = State::Key;
                    }
                    Some(b'}') => return Ok(Some(self.close(at, Event::EndObject))),
                    Some(_) => return err(ErrorKind::ExpectedCommaOrBrace),
                },
                State::End => {
                    return match next {
                        None => {
                            self.state = State::Done;
                            Ok(None)
                        }
                        Some(_) => err(ErrorKind::TrailingCharacters),
                    };
                }
                State::Done => return Ok(None),
            }
        }
    }

    /// Counts one more value, refusing it if the budget is spent.
    #[inline]
    fn count_value(&mut self, at: usize) -> Result<(), Error> {
        if self.elements >= self.limits.max_elements {
            return Err(Error::new(ErrorKind::TooManyElements, at));
        }
        self.elements += 1;
        Ok(())
    }

    /// Reads the value starting with byte `b` at `at`.
    fn value(&mut self, b: u8, at: usize) -> Result<Event<'a>, Error> {
        let bytes = self.bytes();
        let event = match b {
            b'{' | b'[' => {
                self.count_value(at)?;
                if self.stack.len() >= self.limits.max_depth {
                    return Err(Error::new(ErrorKind::DepthLimitExceeded, at));
                }
                self.pos = at + 1;
                if b == b'{' {
                    self.stack.push(Container::Object);
                    self.state = State::ObjectFirst;
                    Event::StartObject
                } else {
                    self.stack.push(Container::Array);
                    self.state = State::ArrayFirst;
                    Event::StartArray
                }
            }
            b'"' => {
                self.count_value(at)?;
                Event::String(self.string(at)?)
            }
            b'-' | b'0'..=b'9' => {
                self.count_value(at)?;
                let end = lexer::scan_number(bytes, at, self.limits.max_number_bytes)?;
                self.pos = end;
                self.after_value();
                Event::Number(Number::from_valid(self.src.get(at..end).unwrap_or("0")))
            }
            b't' | b'f' | b'n' => {
                self.count_value(at)?;
                let (word, event): (&[u8], _) = match b {
                    b't' => (b"true", Event::Bool(true)),
                    b'f' => (b"false", Event::Bool(false)),
                    _ => (b"null", Event::Null),
                };
                self.pos = lexer::scan_literal(bytes, at, word)?;
                self.after_value();
                event
            }
            _ => return Err(Error::new(ErrorKind::ExpectedValue, at)),
        };
        self.last = at;
        Ok(event)
    }

    /// Reads the string value whose quote is at `at`.
    fn string(&mut self, at: usize) -> Result<Cow<'a, str>, Error> {
        let text = self.string_body(at)?;
        self.after_value();
        Ok(text)
    }

    /// Reads the key whose quote is at `at`, and the `:` after it.
    fn key(&mut self, at: usize) -> Result<Event<'a>, Error> {
        let key = self.string_body(at)?;
        self.skip_whitespace();
        match self.bytes().get(self.pos) {
            Some(b':') => self.pos += 1,
            Some(_) => return Err(Error::new(ErrorKind::ExpectedColon, self.pos)),
            None => return Err(self.end_error()),
        }
        self.state = State::Value;
        self.last = at;
        Ok(Event::Key(key))
    }

    /// Scans the string at `at` and returns its contents: borrowed when it has
    /// no escapes (or while skipping), decoded otherwise.
    fn string_body(&mut self, at: usize) -> Result<Cow<'a, str>, Error> {
        let token = lexer::scan_string(self.bytes(), at, self.limits.max_string_bytes)?;
        self.pos = token.end;
        let body = self.src.get(at + 1..token.end - 1).unwrap_or("");
        Ok(if token.escaped && !self.skipping {
            Cow::Owned(lexer::decode(body))
        } else {
            Cow::Borrowed(body)
        })
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Result<Event<'a>, Error>;

    /// The next event; after the end or an error, `None`. An error is yielded
    /// once.
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed.is_some() && self.state == State::Done {
            return None;
        }
        match self.next_event() {
            Ok(Some(event)) => Some(Ok(event)),
            Ok(None) => None,
            Err(e) => {
                // Yield the error once, then stop.
                self.state = State::Done;
                Some(Err(e))
            }
        }
    }
}

impl FusedIterator for Reader<'_> {}
