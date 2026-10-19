//! The push writer: a document written one event at a time.

use alloc::{string::String, vec::Vec};
use core::fmt;

use crate::{
    Format, ToJson,
    encode::{Flush, Keep, Out, ValueWriter, Written},
    reader::Event,
    write,
};

/// Why a [`Writer`] call was refused.
///
/// The sequence errors (every variant but `Io` and `Fmt`) mean the calling
/// code asked for something that is not JSON; the call wrote nothing and the
/// writer is unchanged, so the caller may go on with a correct call. An
/// output error finishes the writer: every later call fails too.
///
/// The enum is `#[non_exhaustive]`.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{WriteError, Writer};
///
/// let mut out = String::new();
/// let mut w = Writer::new(&mut out);
/// w.begin_object()?;
/// assert!(matches!(w.value(&1), Err(WriteError::ExpectedKey)));
/// w.key("n")?;
/// w.value(&1)?;
/// w.end_object()?;
/// w.finish()?;
/// assert_eq!(out, r#"{"n":1}"#);
/// # Ok::<(), WriteError>(())
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum WriteError {
    /// The [`io::Write`](std::io::Write) failed. Its first error is returned
    /// once; later calls return an error of the same kind.
    #[cfg(feature = "std")]
    #[cfg_attr(docsrs, doc(cfg(feature = "std")))]
    Io(std::io::Error),
    /// The [`fmt::Write`](core::fmt::Write) failed.
    Fmt,
    /// A value, or the start of an array or object, where an object member's
    /// key belongs.
    ExpectedKey,
    /// A key outside an object, or a second key before the first one's
    /// value.
    UnexpectedKey,
    /// An `end_array` or `end_object` that does not close the innermost open
    /// container (or with none open, or with a key still waiting for its
    /// value).
    MismatchedEnd,
    /// A second top-level value: a document is one value.
    DocumentComplete,
    /// [`finish`](Writer::finish) before the document's value was complete.
    Incomplete,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            #[cfg(feature = "std")]
            Self::Io(e) => return write!(f, "writing JSON failed: {e}"),
            Self::Fmt => "writing JSON failed: the formatter returned an error",
            Self::ExpectedKey => "expected an object key before this value",
            Self::UnexpectedKey => "a key is only allowed where an object member starts",
            Self::MismatchedEnd => "this end does not close the innermost open container",
            Self::DocumentComplete => "the document already has its value",
            Self::Incomplete => "the document is not complete",
        })
    }
}

impl core::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            #[cfg(feature = "std")]
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// An open container.
#[derive(Clone, Copy, Debug)]
struct Frame {
    object: bool,
    /// No member or item written yet.
    first: bool,
    /// A key was written; its value comes next.
    key: bool,
}

/// Where the output goes.
enum Target<'o> {
    /// Into the caller's `String`, never handed on.
    Keep(Keep),
    #[cfg(feature = "std")]
    Io {
        writer: &'o mut dyn std::io::Write,
        error: Option<std::io::Error>,
        /// The kind of an error already returned.
        failed: Option<std::io::ErrorKind>,
    },
    Fmt {
        writer: &'o mut dyn fmt::Write,
        failed: bool,
    },
}

impl Flush for Target<'_> {
    fn flush(&mut self, buf: &mut String) {
        match self {
            Self::Keep(keep) => keep.flush(buf),
            #[cfg(feature = "std")]
            Self::Io {
                writer,
                error,
                failed,
            } => {
                if error.is_none() && failed.is_none() {
                    if let Err(e) = writer.write_all(buf.as_bytes()) {
                        *error = Some(e);
                    }
                }
                buf.clear();
            }
            Self::Fmt { writer, failed } => {
                if !*failed && writer.write_str(buf).is_err() {
                    *failed = true;
                }
                buf.clear();
            }
        }
    }
}

impl Target<'_> {
    /// The output error, if the output has failed.
    fn error(&mut self) -> Option<WriteError> {
        match self {
            Self::Keep(_) => None,
            #[cfg(feature = "std")]
            Self::Io { error, failed, .. } => {
                if let Some(e) = error.take() {
                    *failed = Some(e.kind());
                    return Some(WriteError::Io(e));
                }
                failed.map(|kind| WriteError::Io(std::io::Error::from(kind)))
            }
            Self::Fmt { failed, .. } => failed.then_some(WriteError::Fmt),
        }
    }
}

/// The caller's buffer, or the writer's own.
enum Buffer<'o> {
    Borrowed(&'o mut String),
    Owned(String),
}

impl Buffer<'_> {
    #[inline]
    fn get(&mut self) -> &mut String {
        match self {
            Self::Borrowed(s) => s,
            Self::Owned(s) => s,
        }
    }
}

/// A push writer: a document written one event at a time, checked as it
/// goes.
///
/// The counterpart of [`Reader`](crate::Reader): where the reader pulls
/// [`Event`]s out of text, the writer takes them in and writes text. Use it
/// to write a document whose shape is only known at run time, to transcode
/// a [`Reader`](crate::Reader)'s events (any depth: the writer keeps an
/// explicit stack, never recursion), or to stream a large document to an
/// [`io::Write`](std::io::Write) without holding it. For a type known at
/// compile time, [`ToJson`] is simpler and checked by the compiler; the two
/// mix, since [`value`](Self::value) takes any `ToJson`.
///
/// Every call is checked against the grammar: a key outside an object, a
/// value where a key belongs, an end that does not match, or a second
/// top-level value is refused with a [`WriteError`] and writes nothing.
/// [`finish`](Self::finish) checks that the document is complete and passes
/// on the last of the output. Escaping and layout are done for you, as for
/// [`to_string_with`](crate::to_string_with).
///
/// Keys are not checked for repeats (that would mean keeping every key of
/// every open object): writing the same key twice in one object is
/// grammatical JSON that this crate's default parser refuses, so a caller
/// whose keys come from input should make them unique.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{WriteError, Writer};
///
/// let mut out = String::new();
/// let mut w = Writer::new(&mut out);
/// w.begin_object()?;
/// w.key("jsonrpc")?;
/// w.value("2.0")?;
/// w.key("result")?;
/// w.begin_array()?;
/// for n in 1..=3 {
///     w.value(&n)?;
/// }
/// w.end_array()?;
/// w.end_object()?;
/// w.finish()?;
/// assert_eq!(out, r#"{"jsonrpc":"2.0","result":[1,2,3]}"#);
/// # Ok::<(), WriteError>(())
/// ```
///
/// Re-indent a document of any depth, event by event:
///
/// ```
/// use jsonx_lib::{Reader, Writer};
///
/// let text = r#"{"a":[1,{"b":null}]}"#;
/// let mut out = String::new();
/// let mut w = Writer::new(&mut out).pretty();
/// for event in Reader::new(text) {
///     w.event(&event?)?;
/// }
/// w.finish()?;
/// assert_eq!(out, jsonx_lib::to_string_pretty(&jsonx_lib::parse(text)?));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Writer<'o> {
    buf: Buffer<'o>,
    target: Target<'o>,
    format: Format,
    flush_at: usize,
    stack: Vec<Frame>,
    /// The top-level value is complete.
    done: bool,
}

impl fmt::Debug for Writer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("format", &self.format)
            .field("depth", &self.stack.len())
            .field("complete", &self.is_complete())
            .finish_non_exhaustive()
    }
}

impl<'o> Writer<'o> {
    /// A writer that appends compact JSON to `out`.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut out = String::from("> ");
    /// let mut w = jsonx_lib::Writer::new(&mut out);
    /// w.value(&true)?;
    /// w.finish()?;
    /// assert_eq!(out, "> true");
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    #[must_use]
    pub fn new(out: &'o mut String) -> Self {
        Self::with_target(Buffer::Borrowed(out), Target::Keep(Keep), usize::MAX)
    }

    /// A writer to an [`io::Write`](std::io::Write), through a buffer of a
    /// few kilobytes: a document of any size is written in pieces as it is
    /// produced. Call [`finish`](Self::finish) to write the last piece.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut file = Vec::new(); // any `io::Write`
    /// let mut w = jsonx_lib::Writer::io(&mut file);
    /// w.begin_array()?;
    /// for i in 0..10_000 {
    ///     w.value(&i)?;
    /// }
    /// w.end_array()?;
    /// w.finish()?;
    /// assert!(file.starts_with(b"[0,1,2,") && file.ends_with(b",9999]"));
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    #[cfg(feature = "std")]
    #[cfg_attr(docsrs, doc(cfg(feature = "std")))]
    #[must_use]
    pub fn io(out: &'o mut dyn std::io::Write) -> Self {
        let target = Target::Io {
            writer: out,
            error: None,
            failed: None,
        };
        let buf = Buffer::Owned(String::with_capacity(write::CHUNK + 256));
        Self::with_target(buf, target, write::CHUNK)
    }

    /// A writer to a [`fmt::Write`](core::fmt::Write) (a `Formatter`, or
    /// anything else that takes text), in pieces of a few kilobytes. Works
    /// without `std`.
    ///
    /// # Examples
    ///
    /// ```
    /// use core::fmt;
    ///
    /// struct Point(i32, i32);
    ///
    /// impl fmt::Display for Point {
    ///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    ///         let mut w = jsonx_lib::Writer::fmt(f);
    ///         w.value(&[self.0, self.1]).map_err(|_| fmt::Error)?;
    ///         w.finish().map_err(|_| fmt::Error)
    ///     }
    /// }
    ///
    /// assert_eq!(Point(3, -4).to_string(), "[3,-4]");
    /// ```
    #[must_use]
    pub fn fmt(out: &'o mut dyn fmt::Write) -> Self {
        let target = Target::Fmt {
            writer: out,
            failed: false,
        };
        let buf = Buffer::Owned(String::with_capacity(write::CHUNK + 256));
        Self::with_target(buf, target, write::CHUNK)
    }

    fn with_target(buf: Buffer<'o>, target: Target<'o>, flush_at: usize) -> Self {
        Self {
            buf,
            target,
            format: Format::compact(),
            flush_at,
            stack: Vec::new(),
            done: false,
        }
    }

    /// Switches to the indented layout of
    /// [`to_string_pretty`](crate::to_string_pretty). Call before writing.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut out = String::new();
    /// let mut w = jsonx_lib::Writer::new(&mut out).pretty();
    /// w.value(&[1, 2])?;
    /// w.finish()?;
    /// assert_eq!(out, "[\n  1,\n  2\n]");
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    #[must_use]
    pub fn pretty(self) -> Self {
        self.with_format(Format::pretty())
    }

    /// Switches to `format`. Call before writing.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Format, Writer};
    ///
    /// let mut out = String::new();
    /// let mut w = Writer::new(&mut out).with_format(Format::compact().html_safe(true));
    /// w.value("<b>")?;
    /// w.finish()?;
    /// assert_eq!(out, r#""\u003cb\u003e""#);
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    #[must_use]
    pub fn with_format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }

    /// Starts an object (as a value: at the top, as an array item, or after
    /// a key).
    ///
    /// # Errors
    ///
    /// [`WriteError::ExpectedKey`] inside an object before a key,
    /// [`WriteError::DocumentComplete`] after the top-level value, or an
    /// output error.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut out = String::new();
    /// let mut w = jsonx_lib::Writer::new(&mut out);
    /// w.begin_object()?;
    /// w.end_object()?;
    /// w.finish()?;
    /// assert_eq!(out, "{}");
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    pub fn begin_object(&mut self) -> Result<(), WriteError> {
        self.open(true)
    }

    /// Starts an array; placed as [`begin_object`](Self::begin_object).
    ///
    /// # Errors
    ///
    /// As [`begin_object`](Self::begin_object).
    ///
    /// # Examples
    ///
    /// ```
    /// let mut out = String::new();
    /// let mut w = jsonx_lib::Writer::new(&mut out);
    /// w.begin_array()?;
    /// w.begin_array()?;
    /// w.end_array()?;
    /// w.end_array()?;
    /// w.finish()?;
    /// assert_eq!(out, "[[]]");
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    pub fn begin_array(&mut self) -> Result<(), WriteError> {
        self.open(false)
    }

    /// Ends the innermost open object.
    ///
    /// # Errors
    ///
    /// [`WriteError::MismatchedEnd`] if the innermost open container is not
    /// an object, none is open, or a key is waiting for its value; or an
    /// output error.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{WriteError, Writer};
    ///
    /// let mut out = String::new();
    /// let mut w = Writer::new(&mut out);
    /// w.begin_array()?;
    /// assert!(matches!(w.end_object(), Err(WriteError::MismatchedEnd)));
    /// w.end_array()?;
    /// # Ok::<(), WriteError>(())
    /// ```
    pub fn end_object(&mut self) -> Result<(), WriteError> {
        self.close(true)
    }

    /// Ends the innermost open array.
    ///
    /// # Errors
    ///
    /// As [`end_object`](Self::end_object), for an array.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut out = String::new();
    /// let mut w = jsonx_lib::Writer::new(&mut out);
    /// w.begin_array()?;
    /// w.value(&"x")?;
    /// w.end_array()?;
    /// assert!(w.is_complete());
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    pub fn end_array(&mut self) -> Result<(), WriteError> {
        self.close(false)
    }

    /// Writes an object member's key; its value comes next. A key already
    /// written in the same object is not refused (see [`Writer`]).
    ///
    /// # Errors
    ///
    /// [`WriteError::UnexpectedKey`] outside an object or right after
    /// another key, or an output error.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{WriteError, Writer};
    ///
    /// let mut out = String::new();
    /// let mut w = Writer::new(&mut out);
    /// assert!(matches!(w.key("k"), Err(WriteError::UnexpectedKey)));
    /// w.begin_object()?;
    /// w.key("say \"hi\"")?;
    /// w.value(&1)?;
    /// w.end_object()?;
    /// w.finish()?;
    /// assert_eq!(out, r#"{"say \"hi\"":1}"#);
    /// # Ok::<(), WriteError>(())
    /// ```
    pub fn key(&mut self, key: &str) -> Result<(), WriteError> {
        self.check()?;
        let format = self.format;
        let depth = self.stack.len();
        let Some(frame) = self.stack.last_mut() else {
            return Err(WriteError::UnexpectedKey);
        };
        if !frame.object || frame.key {
            return Err(WriteError::UnexpectedKey);
        }
        let buf = self.buf.get();
        if !frame.first {
            buf.push(',');
        }
        frame.first = false;
        frame.key = true;
        if format.pretty {
            write::newline(buf, depth, format);
        }
        write::write_str(buf, key, format);
        buf.push_str(if format.pretty { ": " } else { ":" });
        self.flushed()
    }

    /// Writes a whole value, of any type that implements [`ToJson`]: a
    /// scalar, a [`Value`](crate::Value), or your own type (placed as
    /// [`begin_object`](Self::begin_object)).
    ///
    /// # Errors
    ///
    /// As [`begin_object`](Self::begin_object).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{json, Writer};
    ///
    /// let mut out = String::new();
    /// let mut w = Writer::new(&mut out);
    /// w.begin_array()?;
    /// w.value(&json!({ "a": [1] }))?;
    /// w.value(&Some(2.5))?;
    /// w.value("three")?;
    /// w.end_array()?;
    /// w.finish()?;
    /// assert_eq!(out, r#"[{"a":[1]},2.5,"three"]"#);
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    pub fn value<T: ToJson + ?Sized>(&mut self, value: &T) -> Result<(), WriteError> {
        self.before_value()?;
        let out = Out {
            buf: self.buf.get(),
            sink: &mut self.target,
            flush_at: self.flush_at,
            format: self.format,
            depth: self.stack.len(),
        };
        let Written(()) = value.to_json(ValueWriter::new(out));
        self.after_value();
        self.flushed()
    }

    /// Writes one event, as from a [`Reader`](crate::Reader): the start or
    /// end of a container, a key, or a scalar.
    ///
    /// # Errors
    ///
    /// The error of the call the event stands for.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Event, Writer};
    ///
    /// let mut out = String::new();
    /// let mut w = Writer::new(&mut out);
    /// for e in [Event::StartObject, Event::Key("k".into()), Event::Null, Event::EndObject] {
    ///     w.event(&e)?;
    /// }
    /// w.finish()?;
    /// assert_eq!(out, r#"{"k":null}"#);
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    pub fn event(&mut self, event: &Event<'_>) -> Result<(), WriteError> {
        match event {
            Event::StartObject => self.begin_object(),
            Event::EndObject => self.end_object(),
            Event::StartArray => self.begin_array(),
            Event::EndArray => self.end_array(),
            Event::Key(key) => self.key(key),
            Event::String(text) => self.value(&**text),
            Event::Number(number) => self.value(number),
            Event::Bool(b) => self.value(b),
            Event::Null => self.value(&()),
        }
    }

    /// The number of arrays and objects open.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut out = String::new();
    /// let mut w = jsonx_lib::Writer::new(&mut out);
    /// w.begin_array()?;
    /// assert_eq!(w.depth(), 1);
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Whether the document's value is complete.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut out = String::new();
    /// let mut w = jsonx_lib::Writer::new(&mut out);
    /// assert!(!w.is_complete());
    /// w.value(&0)?;
    /// assert!(w.is_complete());
    /// # Ok::<(), jsonx_lib::WriteError>(())
    /// ```
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.done
    }

    /// Checks that the document is complete and passes on the rest of the
    /// output (for [`io`](Self::io) and [`fmt`](Self::fmt) writers).
    ///
    /// # Errors
    ///
    /// [`WriteError::Incomplete`] with containers open or no value written,
    /// or an output error.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{WriteError, Writer};
    ///
    /// let mut out = String::new();
    /// let mut w = Writer::new(&mut out);
    /// w.begin_array()?;
    /// assert!(matches!(w.finish(), Err(WriteError::Incomplete)));
    /// # Ok::<(), WriteError>(())
    /// ```
    pub fn finish(mut self) -> Result<(), WriteError> {
        self.check()?;
        if !self.done {
            return Err(WriteError::Incomplete);
        }
        self.target.flush(self.buf.get());
        self.check()
    }

    // ---- internals -----------------------------------------------------------

    /// Fails if the output has failed.
    fn check(&mut self) -> Result<(), WriteError> {
        self.target.error().map_or(Ok(()), Err)
    }

    /// Passes the buffer on if it is full, then reports any output error.
    fn flushed(&mut self) -> Result<(), WriteError> {
        let buf = self.buf.get();
        if buf.len() >= self.flush_at {
            self.target.flush(buf);
        }
        self.check()
    }

    /// Checks that a value may come here and writes what goes before it.
    fn before_value(&mut self) -> Result<(), WriteError> {
        self.check()?;
        let format = self.format;
        let depth = self.stack.len();
        match self.stack.last_mut() {
            None if self.done => Err(WriteError::DocumentComplete),
            None => Ok(()),
            Some(frame) if frame.object => {
                if frame.key {
                    frame.key = false;
                    Ok(())
                } else {
                    Err(WriteError::ExpectedKey)
                }
            }
            Some(frame) => {
                let buf = self.buf.get();
                if !frame.first {
                    buf.push(',');
                }
                frame.first = false;
                if format.pretty {
                    write::newline(buf, depth, format);
                }
                Ok(())
            }
        }
    }

    /// Marks a value complete.
    #[inline]
    fn after_value(&mut self) {
        if self.stack.is_empty() {
            self.done = true;
        }
    }

    fn open(&mut self, object: bool) -> Result<(), WriteError> {
        self.before_value()?;
        self.buf.get().push(if object { '{' } else { '[' });
        self.stack.push(Frame {
            object,
            first: true,
            key: false,
        });
        self.flushed()
    }

    fn close(&mut self, object: bool) -> Result<(), WriteError> {
        self.check()?;
        match self.stack.last() {
            Some(frame) if frame.object == object && !frame.key => {}
            _ => return Err(WriteError::MismatchedEnd),
        }
        let Some(frame) = self.stack.pop() else {
            return Err(WriteError::MismatchedEnd);
        };
        let format = self.format;
        let depth = self.stack.len();
        let buf = self.buf.get();
        if !frame.first && format.pretty {
            write::newline(buf, depth, format);
        }
        buf.push(if object { '}' } else { ']' });
        self.after_value();
        self.flushed()
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn test_sequence_errors_write_nothing_and_leave_the_writer_usable() {
        let mut out = String::new();
        let mut w = Writer::new(&mut out);
        assert!(matches!(w.end_array(), Err(WriteError::MismatchedEnd)));
        assert!(matches!(w.key("k"), Err(WriteError::UnexpectedKey)));
        assert!(w.begin_object().is_ok());
        assert!(matches!(w.begin_array(), Err(WriteError::ExpectedKey)));
        assert!(w.key("a").is_ok());
        assert!(matches!(w.key("b"), Err(WriteError::UnexpectedKey)));
        assert!(matches!(w.end_object(), Err(WriteError::MismatchedEnd)));
        assert!(w.value(&1).is_ok());
        assert!(matches!(w.end_array(), Err(WriteError::MismatchedEnd)));
        assert!(w.end_object().is_ok());
        assert!(matches!(w.value(&2), Err(WriteError::DocumentComplete)));
        assert!(matches!(w.begin_array(), Err(WriteError::DocumentComplete)));
        assert_eq!(w.depth(), 0);
        assert!(w.finish().is_ok());
        assert_eq!(out, r#"{"a":1}"#);

        let mut out = String::new();
        assert!(matches!(
            Writer::new(&mut out).finish(),
            Err(WriteError::Incomplete)
        ));
        for e in [
            WriteError::Fmt,
            WriteError::ExpectedKey,
            WriteError::UnexpectedKey,
            WriteError::MismatchedEnd,
            WriteError::DocumentComplete,
            WriteError::Incomplete,
        ] {
            assert!(!e.to_string().is_empty());
            assert!(core::error::Error::source(&e).is_none());
        }
    }

    #[test]
    fn test_pretty_events_match_pretty_values() {
        let v = crate::json!({ "a": [], "b": {}, "c": [{ "d": [1, 2] }], "e": "x" });
        let mut out = String::new();
        let mut w = Writer::new(&mut out).pretty();
        for event in crate::Reader::new(&v.to_string()) {
            assert!(event.is_ok_and(|e| w.event(&e).is_ok()));
        }
        assert!(w.finish().is_ok());
        assert_eq!(out, crate::to_string_pretty(&v));
    }

    #[test]
    fn test_fmt_target_reports_failure() {
        struct Full;
        impl fmt::Write for Full {
            fn write_str(&mut self, _: &str) -> fmt::Result {
                Err(fmt::Error)
            }
        }
        let mut full = Full;
        let mut w = Writer::fmt(&mut full);
        assert!(w.value(&"x").is_ok()); // still buffered
        assert!(matches!(w.finish(), Err(WriteError::Fmt)));
        let mut full = Full;
        let mut w = Writer::fmt(&mut full);
        assert!(w.begin_array().is_ok());
        let long = "y".repeat(write::CHUNK);
        assert!(matches!(w.value(&long), Err(WriteError::Fmt)));
        assert!(matches!(w.value(&1), Err(WriteError::Fmt)));
        assert!(alloc::format!("{w:?}").contains("depth: 1"));
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_io_target_returns_the_error_then_its_kind() {
        struct Broken;
        impl std::io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut broken = Broken;
        let mut w = Writer::io(&mut broken);
        assert!(w.begin_array().is_ok());
        let long = "z".repeat(write::CHUNK);
        let first = w.value(&long).err();
        assert_eq!(
            first.map(|e| e.to_string()),
            Some("writing JSON failed: disk full".to_string())
        );
        let later = w.end_array().err();
        assert!(
            matches!(later, Some(WriteError::Io(ref e)) if e.kind() == std::io::ErrorKind::Other)
        );
        assert!(later.is_some_and(|e| core::error::Error::source(&e).is_some()));
    }
}
