//! The incremental reader: a document fed in chunks as they arrive.

use alloc::string::String;

use crate::{
    DuplicateKeys, Kind, Limits, Parser, Value,
    error::{Error, ErrorKind},
    machine::Machine,
    reader::Event,
};

/// The answer of a [`StreamReader`] call: ready, or waiting for more input.
///
/// `NeedInput` is not an error: the input so far is a valid beginning, and
/// the call can be made again after [`feed`](StreamReader::feed) or
/// [`end_input`](StreamReader::end_input).
///
/// # Examples
///
/// ```
/// use jsonx_lib::{Event, Step, StreamReader};
///
/// let mut r = StreamReader::new();
/// assert_eq!(r.next_event()?, Step::NeedInput);
/// r.feed(b"[tr")?;
/// assert_eq!(r.next_event()?, Step::Ready(Some(Event::StartArray)));
/// assert_eq!(r.next_event()?, Step::NeedInput); // `tr` is not a value yet
/// r.feed(b"ue]")?;
/// assert_eq!(r.next_event()?.ready(), Some(Some(Event::Bool(true))));
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub enum Step<T> {
    /// The call completed with this result.
    Ready(T),
    /// The input so far ends inside what the call needs: feed more, or end
    /// the input, and call again.
    NeedInput,
}

impl<T> Step<T> {
    /// The result, if ready.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Step;
    ///
    /// assert_eq!(Step::Ready(3).ready(), Some(3));
    /// assert_eq!(Step::<u8>::NeedInput.ready(), None);
    /// ```
    #[inline]
    pub fn ready(self) -> Option<T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::NeedInput => None,
        }
    }

    /// Whether the call completed.
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(jsonx_lib::Step::Ready(()).is_ready());
    /// ```
    #[inline]
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    /// Applies `f` to a ready result.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Step;
    ///
    /// assert_eq!(Step::Ready(2).map(|n| n * 10), Step::Ready(20));
    /// ```
    #[inline]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Step<U> {
        match self {
            Self::Ready(value) => Step::Ready(f(value)),
            Self::NeedInput => Step::NeedInput,
        }
    }
}

/// A raw value being skipped across calls.
#[derive(Clone, Copy, Debug)]
struct RawProgress {
    /// The depth the value started at; it is complete when the reader is back.
    depth: usize,
    /// The value's first byte (absolute), once its first event was read.
    start: Option<usize>,
}

/// How much consumed text the buffer may keep before it is compacted.
const COMPACT_AT: usize = 4 * 1024;

/// A pull reader over input that arrives in pieces: bytes go in with
/// [`feed`](Self::feed) as they arrive, events come out as soon as they are
/// complete.
///
/// It is the same reader as [`Reader`](crate::Reader), over a buffer: the
/// same grammar, the same budgets, and the same errors at the same offsets
/// (counted from the first byte ever fed), whatever the input's chunking.
/// When a call needs bytes that have not arrived it returns
/// [`Step::NeedInput`] and changes nothing a later call would see
/// differently; after [`end_input`](Self::end_input), running out is the
/// [`ErrorKind::UnexpectedEnd`] that [`Reader`](crate::Reader) reports.
///
/// Events borrow from the reader's buffer, so each one must be dropped
/// before the next call. In particular, feed more input after the borrow
/// ends: in an `else` after `if let Step::Ready(..) = reader.next_event()?`
/// (Rust 2024 drops the `if let` temporaries before the `else`), or after
/// the loop that drains the ready events, as below; a `match` keeps the
/// event borrowed in every arm. Consumed input is dropped from the buffer as more
/// is fed, so a long document read as it arrives needs memory for one token
/// and one chunk, not for the document. A token cut by a chunk boundary is
/// continued where it stopped, never scanned again from its start: a string
/// fed one byte at a time costs linear time.
///
/// The input must be UTF-8, checked as it arrives (a character may be split
/// across chunks). The first invalid byte is reported as
/// [`ErrorKind::InvalidUtf8`] when the reader reaches it: a syntax error
/// earlier in the document is reported first. (A complete document given to
/// [`parse_bytes`](crate::parse_bytes) checks the encoding before the
/// syntax, so the two can name different errors for a document that has
/// both.)
///
/// [`Limits::max_input_bytes`] bounds the total fed; the other budgets apply
/// exactly as for a complete document. After an error the reader is
/// finished: every later call returns the same error.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{Event, Step, StreamReader};
///
/// let mut reader = StreamReader::new();
/// let mut keys = Vec::new();
/// for chunk in [&b"{\"jsonr"[..], b"pc\": \"2.0\", \"i", b"d\": 12", b"}"] {
///     reader.feed(chunk)?;
///     while let Step::Ready(Some(event)) = reader.next_event()? {
///         if let Event::Key(key) = event {
///             keys.push(key.into_owned());
///         }
///     }
/// }
/// reader.end_input();
/// assert_eq!(reader.next_event()?, Step::Ready(None)); // the document is complete
/// assert_eq!(keys, ["jsonrpc", "id"]);
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct StreamReader {
    /// The input not yet dropped: valid UTF-8, starting at absolute offset
    /// `base`.
    buf: String,
    base: usize,
    /// An incomplete UTF-8 sequence at the end of the input so far.
    tail: [u8; 4],
    tail_len: usize,
    /// The absolute offset of the first byte that is not UTF-8. Nothing
    /// from it on is kept.
    utf8_error: Option<usize>,
    /// Bytes fed in total.
    fed: usize,
    ended: bool,
    machine: Machine,
    failed: Option<Error>,
    /// The absolute offset where the most recent event's token began.
    last: usize,
    raw: Option<RawProgress>,
}

impl Default for StreamReader {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamReader {
    /// A reader with the default [`Limits`], refusing duplicate keys in what
    /// [`read_value`](Self::read_value) builds. For others, use
    /// [`Parser::stream_reader`].
    ///
    /// # Examples
    ///
    /// ```
    /// let r = jsonx_lib::StreamReader::new();
    /// assert_eq!(r.offset(), 0);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(Limits::new(), DuplicateKeys::Error)
    }

    /// A reader with the given budgets and duplicate-key policy.
    pub(crate) fn with_config(limits: Limits, duplicates: DuplicateKeys) -> Self {
        Self {
            buf: String::new(),
            base: 0,
            tail: [0; 4],
            tail_len: 0,
            utf8_error: None,
            fed: 0,
            ended: false,
            machine: {
                let mut machine = Machine::new(limits, duplicates, false);
                // Keys outlive the front of the buffer, which is dropped.
                machine.copy_keys = true;
                machine
            },
            failed: None,
            last: 0,
            raw: None,
        }
    }

    /// Reads a sequence of documents instead of one, as
    /// [`Reader::documents`](crate::Reader::documents) does: NDJSON, JSON
    /// Lines, or concatenated JSON, fed as it arrives. Whole-value reads
    /// return one document each, and `Ready(None)` once the input has ended
    /// after the last.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Step, StreamReader};
    ///
    /// let mut r = StreamReader::new().documents();
    /// let mut seen = Vec::new();
    /// for chunk in [&b"{\"id\": 1}\n{\"i"[..], b"d\": 2}\n"] {
    ///     r.feed(chunk)?;
    ///     while let Step::Ready(Some(doc)) = r.read_value()? {
    ///         seen.push(doc["id"].as_i64());
    ///     }
    /// }
    /// r.end_input();
    /// assert_eq!(r.read_value()?, Step::Ready(None));
    /// assert_eq!(seen, [Some(1), Some(2)]);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn documents(mut self) -> Self {
        self.machine.documents = true;
        self
    }

    /// Adds the next piece of input. A chunk may end anywhere, even inside a
    /// token or a UTF-8 character.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InputTooLarge`] when the input in total grows past
    /// [`Limits::max_input_bytes`] (the reader is then finished);
    /// [`ErrorKind::InputAfterEnd`] after [`end_input`](Self::end_input);
    /// any error the reader already stopped at.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Limits, Parser};
    ///
    /// let mut r = Parser::new().limits(Limits::new().with_max_input_bytes(4)).stream_reader();
    /// assert!(r.feed(b"[1,").is_ok());
    /// assert_eq!(r.feed(b"22]").unwrap_err().kind(), ErrorKind::InputTooLarge);
    /// ```
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        if self.ended {
            return Err(Error::new(ErrorKind::InputAfterEnd, self.fed));
        }
        let max = self.machine.limits.max_input_bytes;
        if bytes.len() > max - self.fed {
            let e = Error::new(ErrorKind::InputTooLarge, max);
            self.failed = Some(e);
            return Err(e);
        }
        self.fed += bytes.len();
        self.compact();
        if self.utf8_error.is_none() {
            self.append(bytes);
        }
        Ok(())
    }

    /// Says that the input has ended: nothing more will be fed. From now on,
    /// running out of input is an error, as for a complete document, and a
    /// number or whitespace at the very end is final.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Event, Step, StreamReader};
    ///
    /// let mut r = StreamReader::new();
    /// r.feed(b"12")?;
    /// assert_eq!(r.next_event()?, Step::NeedInput); // `123` may follow
    /// r.end_input();
    /// assert_eq!(r.next_event()?, Step::Ready(Some(Event::Number(12.into()))));
    /// assert_eq!(r.next_event()?, Step::Ready(None));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn end_input(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        if self.tail_len > 0 && self.utf8_error.is_none() {
            // A character cut off by the end of the input.
            self.utf8_error = Some(self.base + self.buf.len());
        }
        self.machine.complete = true;
    }

    /// Reads up to 8 KiB from `source` and feeds it, or calls
    /// [`end_input`](Self::end_input) when `source` is at its end. Returns
    /// the number of bytes read (`0` at the end). Interrupted reads are
    /// retried.
    ///
    /// A budget error from [`feed`](Self::feed) is not an I/O error: it is
    /// kept, and returned by the next reading call.
    ///
    /// # Errors
    ///
    /// Any error from `source` other than [`std::io::ErrorKind::Interrupted`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Step, StreamReader};
    ///
    /// let mut source: &[u8] = br#"{"big": [1, 2, 3]}"#;
    /// let mut r = StreamReader::new();
    /// let mut events = 0;
    /// loop {
    ///     if let Step::Ready(event) = r.next_event()? {
    ///         match event {
    ///             Some(_) => events += 1,
    ///             None => break,
    ///         }
    ///     } else {
    ///         r.read_from(&mut source)?;
    ///     }
    /// }
    /// assert_eq!(events, 8); // {, "big", [, 1, 2, 3, ], }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[cfg(feature = "std")]
    #[cfg_attr(docsrs, doc(cfg(feature = "std")))]
    pub fn read_from(&mut self, source: &mut dyn std::io::Read) -> std::io::Result<usize> {
        let mut chunk = [0u8; 8 * 1024];
        loop {
            match source.read(&mut chunk) {
                Ok(0) => {
                    self.end_input();
                    return Ok(0);
                }
                Ok(n) => {
                    // A refused feed is kept in `failed` and reported by the
                    // next reading call.
                    let _kept = self.feed(chunk.get(..n).unwrap_or_default());
                    return Ok(n);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Reads the next event: `Ready(Some(event))`, `Ready(None)` once the
    /// document is complete and the input has ended with nothing but
    /// whitespace after it, or `NeedInput`.
    ///
    /// # Errors
    ///
    /// The first syntax error, exceeded budget, or invalid UTF-8, at its
    /// byte offset. Once an error is returned, every later call returns it.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Event, Step, StreamReader};
    ///
    /// let mut r = StreamReader::new();
    /// r.feed(b"[1,")?;
    /// assert_eq!(r.next_event()?, Step::Ready(Some(Event::StartArray)));
    /// assert!(r.next_event()?.is_ready()); // 1
    /// r.feed(b"]")?;
    /// let err = r.next_event().unwrap_err();
    /// assert_eq!((err.kind(), err.offset()), (ErrorKind::TrailingComma, 3));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn next_event(&mut self) -> Result<Step<Option<Event<'_>>>, Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        let Self {
            buf,
            machine,
            base,
            last,
            failed,
            utf8_error,
            ..
        } = self;
        match machine.step(buf) {
            Ok(Some(event)) => {
                *last = *base + machine.last;
                Ok(Step::Ready(Some(event)))
            }
            Ok(None) => match utf8_error {
                // The valid text holds a whole document, then bad bytes.
                Some(at) => Err(*failed.insert(Error::new(ErrorKind::InvalidUtf8, *at))),
                None => Ok(Step::Ready(None)),
            },
            Err(e) => halt(failed, machine, buf.len(), *base, *utf8_error, e),
        }
    }

    /// Reads the next value whole and returns its exact source text, without
    /// decoding or allocating anything, as [`Reader::read_raw`](crate::Reader::read_raw)
    /// does, with the same positioning rules: `Ready(None)` when no value
    /// comes next.
    ///
    /// The value must be in the buffer whole before it can be returned, so
    /// a large value costs memory for all of it. `NeedInput` keeps the
    /// progress: the next call continues the same value.
    ///
    /// # Errors
    ///
    /// As [`next_event`](Self::next_event), for any error inside the value.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Event, Step, StreamReader};
    ///
    /// let mut r = StreamReader::new();
    /// r.feed(br#"{"id": 1, "params": {"line""#)?;
    /// assert!(r.next_event()?.is_ready()); // {
    /// assert!(r.next_event()?.is_ready()); // "id"
    /// assert_eq!(r.read_raw()?, Step::Ready(Some("1")));
    /// assert!(r.next_event()?.is_ready()); // "params"
    /// assert_eq!(r.read_raw()?, Step::NeedInput);
    /// r.feed(br#": 3}}"#)?;
    /// assert_eq!(r.read_raw()?, Step::Ready(Some(r#"{"line": 3}"#)));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn read_raw(&mut self) -> Result<Step<Option<&str>>, Error> {
        Ok(self.raw_span()?.map(|span| {
            span.and_then(|(start, end)| self.buf.get(start - self.base..end - self.base))
        }))
    }

    /// Reads the next value whole and builds it as a [`Value`], applying the
    /// reader's [`DuplicateKeys`] policy; positioning as for
    /// [`read_raw`](Self::read_raw). The value borrows from the buffer:
    /// call [`Value::into_owned`] to keep it.
    ///
    /// # Errors
    ///
    /// As [`read_raw`](Self::read_raw), plus [`ErrorKind::DuplicateKey`]
    /// under [`DuplicateKeys::Error`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Step, StreamReader};
    ///
    /// let mut r = StreamReader::new();
    /// r.feed(br#"{"a": [1, "#)?;
    /// assert_eq!(r.read_value()?, Step::NeedInput);
    /// r.feed(br#"2]}"#)?;
    /// let value = r.read_value()?.ready().flatten().unwrap_or_default();
    /// assert_eq!(value["a"][1], 2);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn read_value(&mut self) -> Result<Step<Option<Value<'_>>>, Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        let Step::Ready(span) = self.raw_span()? else {
            return Ok(Step::NeedInput);
        };
        let Some((start, end)) = span else {
            return Ok(Step::Ready(None));
        };
        let text = self
            .buf
            .get(start - self.base..end - self.base)
            .unwrap_or("");
        // The budgets, and duplicates under `DuplicateKeys::Error`, were
        // checked while the value was skipped; only `First` and `Last`
        // remain to apply.
        let parser = Parser::new()
            .limits(Limits::unlimited())
            .duplicate_keys(self.machine.duplicates);
        match parser.parse(text) {
            Ok(value) => Ok(Step::Ready(Some(value))),
            Err(e) => {
                let e = Error::new(e.kind(), e.offset() + start);
                self.failed = Some(e);
                Err(e)
            }
        }
    }

    /// The kind of the next value, without consuming anything: `Ready(None)`
    /// when no value comes next, or when the next byte cannot start one (the
    /// next [`next_event`](Self::next_event) reports why).
    ///
    /// # Errors
    ///
    /// Any error the reader already stopped at.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Kind, Step, StreamReader};
    ///
    /// let mut r = StreamReader::new();
    /// r.feed(b"[ ")?;
    /// assert!(r.next_event()?.is_ready());
    /// assert_eq!(r.peek_kind()?, Step::NeedInput);
    /// r.feed(b"\"x\"]")?;
    /// assert_eq!(r.peek_kind()?, Step::Ready(Some(Kind::String)));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn peek_kind(&mut self) -> Result<Step<Option<Kind>>, Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        match self.machine.at_value(&self.buf) {
            Ok(false) => Ok(Step::Ready(None)),
            Ok(true) => match self.machine.peek_kind(&self.buf) {
                Some(kind) => Ok(Step::Ready(Some(kind))),
                // The value's first byte has not arrived.
                None if self.machine.needed_from() == self.buf.len() && !self.machine.complete => {
                    self.halt(Error::new(ErrorKind::UnexpectedEnd, self.buf.len()))
                }
                None => Ok(Step::Ready(None)),
            },
            Err(e) => self.halt(e),
        }
    }

    /// The number of arrays and objects open.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::StreamReader::new();
    /// r.feed(b"[[")?;
    /// let _ = r.next_event()?;
    /// let _ = r.next_event()?;
    /// assert_eq!(r.depth(), 2);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn depth(&self) -> usize {
        self.machine.depth()
    }

    /// The byte offset, from the first byte fed, just past the input
    /// consumed so far.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::StreamReader::new();
    /// r.feed(b"[10, 20]")?;
    /// let _ = r.next_event()?;
    /// let _ = r.next_event()?;
    /// assert_eq!(r.offset(), 3);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn offset(&self) -> usize {
        self.base + self.machine.pos
    }

    /// The byte offset, from the first byte fed, where the most recent
    /// event's token began (see [`Reader::event_offset`](crate::Reader::event_offset)).
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::StreamReader::new();
    /// r.feed(b"[ 7]")?;
    /// let _ = r.next_event()?;
    /// let _ = r.next_event()?;
    /// assert_eq!(r.event_offset(), 2);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn event_offset(&self) -> usize {
        self.last
    }

    /// The bytes of input the reader holds: what has been fed and not yet
    /// dropped. Consumed input is dropped as more is fed.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut r = jsonx_lib::StreamReader::new();
    /// r.feed(b"[1, 2")?;
    /// assert_eq!(r.buffered(), 5);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.buf.len() + self.tail_len
    }

    // ---- internals -----------------------------------------------------------

    /// [`halt`] for `self`.
    fn halt<T>(&mut self, e: Error) -> Result<Step<T>, Error> {
        halt(
            &mut self.failed,
            &self.machine,
            self.buf.len(),
            self.base,
            self.utf8_error,
            e,
        )
    }

    /// Skips the next value (continuing one already started) and returns
    /// its absolute span, or `None` when no value comes next.
    fn raw_span(&mut self) -> Result<Step<Option<(usize, usize)>>, Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        let mut progress = match self.raw {
            Some(progress) => progress,
            None => match self.machine.at_value(&self.buf) {
                Ok(false) => return Ok(Step::Ready(None)),
                Ok(true) => RawProgress {
                    depth: self.machine.depth(),
                    start: None,
                },
                Err(e) => return self.halt(e),
            },
        };
        self.machine.skipping = true;
        let outcome = loop {
            match self.machine.step(&self.buf) {
                Ok(Some(_)) => {
                    self.last = self.base + self.machine.last;
                    let start = *progress.start.get_or_insert(self.last);
                    if self.machine.depth() == progress.depth {
                        break Ok(Some((start, self.base + self.machine.pos)));
                    }
                }
                // `at_value` saw a value, so the document cannot end here.
                Ok(None) => break Err(Error::new(ErrorKind::UnexpectedEnd, self.buf.len())),
                Err(e) => break Err(e),
            }
        };
        self.machine.skipping = false;
        match outcome {
            Ok(span) => {
                self.raw = None;
                Ok(Step::Ready(span))
            }
            Err(e) => {
                self.raw = Some(progress);
                self.halt(e)
            }
        }
    }

    /// Drops consumed input from the front of the buffer once there is
    /// enough of it to be worth the copy (amortized constant per byte).
    fn compact(&mut self) {
        let mut keep_from = self.machine.needed_from();
        if let Some(RawProgress {
            start: Some(start), ..
        }) = self.raw
        {
            keep_from = keep_from.min(start - self.base);
        }
        if keep_from < COMPACT_AT
            || keep_from * 2 < self.buf.len()
            || !self.buf.is_char_boundary(keep_from)
        {
            return;
        }
        let _dropped = self.buf.drain(..keep_from);
        self.base += keep_from;
        self.machine.rebase(keep_from);
    }

    /// Records that the input stops being UTF-8 at the end of the buffer.
    /// No valid byte can follow, so the text so far is all there will be.
    fn bad_byte(&mut self) {
        self.utf8_error = Some(self.base + self.buf.len());
        self.machine.complete = true;
    }

    /// Appends `bytes` to the buffer, keeping only valid UTF-8 and holding
    /// back a character cut off at the end.
    fn append(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        if self.tail_len > 0 {
            // Complete the character the last chunk cut off.
            let take = (4 - self.tail_len).min(rest.len());
            let (head, after) = rest.split_at(take);
            if let Some(slot) = self.tail.get_mut(self.tail_len..self.tail_len + take) {
                slot.copy_from_slice(head);
            }
            let have = self.tail_len + take;
            let pending = self.tail.get(..have).unwrap_or_default();
            match core::str::from_utf8(pending) {
                Ok(text) => {
                    self.buf.push_str(text);
                    self.tail_len = 0;
                    rest = after;
                }
                Err(e) => {
                    let used = e.valid_up_to();
                    if used > 0 {
                        // The cut character completed; the bytes after it
                        // in `pending` are new input to read normally.
                        let done = pending.get(..used).unwrap_or_default();
                        self.buf.push_str(core::str::from_utf8(done).unwrap_or(""));
                        let extra = used - self.tail_len;
                        self.tail_len = 0;
                        rest = bytes.get(extra..).unwrap_or_default();
                    } else if e.error_len().is_none() && after.is_empty() {
                        // Still incomplete, and this chunk is used up.
                        self.tail_len = have;
                        return;
                    } else {
                        self.bad_byte();
                        return;
                    }
                }
            }
        }
        match core::str::from_utf8(rest) {
            Ok(text) => self.buf.push_str(text),
            Err(e) => {
                let (valid, invalid) = rest.split_at(e.valid_up_to());
                self.buf.push_str(core::str::from_utf8(valid).unwrap_or(""));
                if e.error_len().is_none() {
                    // A character cut off by the end of this chunk.
                    if let Some(slot) = self.tail.get_mut(..invalid.len()) {
                        slot.copy_from_slice(invalid);
                    }
                    self.tail_len = invalid.len();
                } else {
                    self.bad_byte();
                }
            }
        }
    }
}

/// Turns a machine error into a call's answer: `NeedInput` when the buffer
/// (of `len` bytes, starting at absolute offset `base`) merely ran out,
/// otherwise the error, made absolute and kept in `failed`.
fn halt<T>(
    failed: &mut Option<Error>,
    machine: &Machine,
    len: usize,
    base: usize,
    utf8_error: Option<usize>,
    e: Error,
) -> Result<Step<T>, Error> {
    let ran_out = e.kind() == ErrorKind::UnexpectedEnd && e.offset() == len;
    let e = match (ran_out, utf8_error) {
        (true, _) if !machine.complete => return Ok(Step::NeedInput),
        // The valid text ends where the input stops being UTF-8.
        (true, Some(at)) => Error::new(ErrorKind::InvalidUtf8, at),
        _ => Error::new(e.kind(), e.offset() + base),
    };
    *failed = Some(e);
    Err(e)
}

impl Parser {
    /// A [`StreamReader`] with this parser's budgets and duplicate-key
    /// policy (which [`StreamReader::read_value`] applies).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Limits, Parser};
    ///
    /// let mut r = Parser::new().limits(Limits::new().with_max_depth(1)).stream_reader();
    /// r.feed(b"[[]]")?;
    /// assert!(r.next_event().is_ok());
    /// assert_eq!(r.next_event().unwrap_err().kind(), ErrorKind::DepthLimitExceeded);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn stream_reader(&self) -> StreamReader {
        StreamReader::with_config(self.get_limits(), self.get_duplicate_keys())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_split_at_every_byte_of_a_character() {
        let text = "[\"a\u{e9}\u{20ac}\u{1f600}\"]";
        for cut in 0..=text.len() {
            let (a, b) = text.as_bytes().split_at(cut);
            let mut r = StreamReader::new();
            assert!(r.feed(a).is_ok() && r.feed(b).is_ok());
            r.end_input();
            let _start = r.next_event();
            let got = r.next_event().ok().and_then(Step::ready).flatten();
            assert_eq!(
                got,
                Some(Event::String("a\u{e9}\u{20ac}\u{1f600}".into())),
                "cut {cut}"
            );
        }
        // One byte at a time.
        let mut r = StreamReader::new();
        for &byte in text.as_bytes() {
            assert!(r.feed(&[byte]).is_ok());
        }
        assert_eq!(r.buffered(), text.len());
    }

    #[test]
    fn test_invalid_utf8_is_reported_where_the_reader_reaches_it() {
        let mut r = StreamReader::new();
        assert!(r.feed(b"[1, \"a\xC3").is_ok());
        assert!(r.feed(b"(\"]").is_ok()); // 0xC3 0x28 is not UTF-8
        let _start = r.next_event();
        let _one = r.next_event();
        let err = r.next_event().err();
        assert_eq!(
            err.map(|e| (e.kind(), e.offset())),
            Some((ErrorKind::InvalidUtf8, 6))
        );
        // A character cut off by the end of the input.
        let mut r = StreamReader::new();
        assert!(r.feed(b"\"\xE2\x82").is_ok());
        assert_eq!(r.next_event().ok(), Some(Step::NeedInput));
        r.end_input();
        let err = r.next_event().err();
        assert_eq!(
            err.map(|e| (e.kind(), e.offset())),
            Some((ErrorKind::InvalidUtf8, 1))
        );
        // A syntax error before the bad byte comes first.
        let mut r = StreamReader::new();
        assert!(r.feed(b"[1 2, \xFF]").is_ok());
        let _start = r.next_event();
        let _one = r.next_event();
        let err = r.next_event().err();
        assert_eq!(
            err.map(|e| (e.kind(), e.offset())),
            Some((ErrorKind::ExpectedCommaOrBracket, 3))
        );
        // An invalid continuation completing a cut character.
        let mut r = StreamReader::new();
        assert!(r.feed(b"\"\xE2").is_ok());
        assert!(r.feed(b"\x41\"").is_ok());
        let err = r.next_event().err();
        assert_eq!(
            err.map(|e| (e.kind(), e.offset())),
            Some((ErrorKind::InvalidUtf8, 1))
        );
        // A cut character completed, with more input after it in the chunk.
        let mut r = StreamReader::new();
        assert!(r.feed(b"\"\xE2\x82").is_ok());
        assert!(r.feed(b"\xACabc\"").is_ok());
        assert_eq!(
            r.next_event().ok().and_then(Step::ready).flatten(),
            Some(Event::String("\u{20ac}abc".into()))
        );
    }

    #[test]
    fn test_feed_after_end_and_errors_latch() {
        let mut r = StreamReader::new();
        assert!(r.feed(b"[").is_ok());
        r.end_input();
        r.end_input();
        assert_eq!(
            r.feed(b"]").err().map(|e| e.kind()),
            Some(ErrorKind::InputAfterEnd)
        );
        let _start = r.next_event();
        let first = r.next_event().err();
        assert_eq!(first.map(|e| e.kind()), Some(ErrorKind::UnexpectedEnd));
        assert_eq!(r.next_event().err(), first);
        assert_eq!(r.read_raw().err(), first);
        assert_eq!(r.read_value().err(), first);
        assert_eq!(r.peek_kind().err(), first);
        assert_eq!(r.feed(b"x").err(), first);
    }

    #[test]
    fn test_compaction_keeps_memory_to_a_chunk_and_offsets_absolute() {
        let mut r = StreamReader::new();
        let item = b"12345, ";
        let mut total = 1;
        assert!(r.feed(b"[").is_ok());
        let mut max_buffered = 0;
        for _ in 0..20_000 {
            assert!(r.feed(item).is_ok());
            total += item.len();
            while let Ok(Step::Ready(Some(_))) = r.next_event() {}
            max_buffered = max_buffered.max(r.buffered());
        }
        assert!(max_buffered < 3 * COMPACT_AT, "{max_buffered}");
        assert!(r.feed(b"]").is_ok());
        let err = r.next_event().err();
        assert_eq!(
            err.map(|e| (e.kind(), e.offset())),
            Some((ErrorKind::TrailingComma, total))
        );
    }

    #[test]
    fn test_raw_value_spanning_many_chunks_survives_compaction() {
        let mut r = StreamReader::new();
        assert!(r.feed(b"[0, ").is_ok());
        let _start = r.next_event();
        assert_eq!(
            r.read_raw().ok().map(|s| s.map(|o| o.map(str::len))),
            Some(Step::Ready(Some(1)))
        );
        assert!(r.feed(b"{\"k\": [").is_ok());
        assert_eq!(r.read_raw().ok(), Some(Step::NeedInput));
        let mut expected = String::from("{\"k\": [");
        for i in 0..5_000 {
            let piece = alloc::format!("{i}, ");
            expected.push_str(&piece);
            assert!(r.feed(piece.as_bytes()).is_ok());
            assert_eq!(r.read_raw().ok(), Some(Step::NeedInput));
        }
        assert!(r.feed(b"0]}]").is_ok());
        expected.push_str("0]}");
        let raw = r
            .read_raw()
            .ok()
            .and_then(Step::ready)
            .flatten()
            .map(String::from);
        assert_eq!(raw.as_deref(), Some(expected.as_str()));
        assert_eq!(r.event_offset(), 4 + expected.len() - 1);
    }
}
