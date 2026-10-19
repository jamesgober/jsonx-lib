//! Typed output: [`ToJson`], and the writers it writes through.
//!
//! A `ToJson` value writes itself straight into the output buffer, member by
//! member, without building a [`Value`] first. The writers are typed so that
//! the output is well-formed by construction: an [`ObjectWriter`] only takes
//! members, an [`ArrayWriter`] only takes items, and a [`ValueWriter`] is
//! consumed by the one value it writes, returning the [`Written`] proof that
//! `to_json` must return.

#[cfg(target_has_atomic = "ptr")]
use alloc::sync::Arc;
use alloc::{
    borrow::Cow,
    boxed::Box,
    collections::{BTreeMap, BTreeSet, VecDeque},
    rc::Rc,
    string::String,
    vec::Vec,
};
use core::convert::Infallible;

use crate::{Array, Format, Number, Object, RawValue, Value, number, write};

/// Takes the output buffer when it holds enough to pass on.
pub(crate) trait Flush {
    /// Passes `buf` on and empties it. A failure is kept by the
    /// implementation and reported when the write completes.
    fn flush(&mut self, buf: &mut String);
}

/// Output kept in a `String`: nothing to pass on.
pub(crate) struct Keep;

impl Flush for Keep {
    #[inline]
    fn flush(&mut self, _buf: &mut String) {}
}

/// Output to an [`io::Write`](std::io::Write), in chunks. The first error is
/// kept; after it, chunks are dropped.
#[cfg(feature = "std")]
pub(crate) struct IoSink<'a> {
    pub(crate) writer: &'a mut dyn std::io::Write,
    pub(crate) error: Option<std::io::Error>,
}

#[cfg(feature = "std")]
impl Flush for IoSink<'_> {
    fn flush(&mut self, buf: &mut String) {
        if self.error.is_none() {
            if let Err(e) = self.writer.write_all(buf.as_bytes()) {
                self.error = Some(e);
            }
        }
        buf.clear();
    }
}

/// Where a writer writes: the buffer, what to do when it fills, the layout,
/// and the indentation level of what is written next.
pub(crate) struct Out<'w> {
    pub(crate) buf: &'w mut String,
    pub(crate) sink: &'w mut dyn Flush,
    /// Hand the buffer to `sink` once it holds this many bytes.
    pub(crate) flush_at: usize,
    pub(crate) format: Format,
    pub(crate) depth: usize,
}

impl Out<'_> {
    /// The same output, at indentation level `depth`.
    #[inline]
    pub(crate) fn at(&mut self, depth: usize) -> Out<'_> {
        Out {
            buf: &mut *self.buf,
            sink: &mut *self.sink,
            flush_at: self.flush_at,
            format: self.format,
            depth,
        }
    }

    /// Ends a value: passes the buffer on if it is full enough.
    #[inline]
    fn done(&mut self) -> Written {
        if self.buf.len() >= self.flush_at {
            self.sink.flush(self.buf);
        }
        Written(())
    }
}

/// Proof that a [`ValueWriter`] wrote its value: what [`ToJson::to_json`]
/// returns.
///
/// Only a `ValueWriter` method makes one, and each consumes the writer, so
/// a `to_json` that compiles writes exactly one value: never none, never
/// two. There is nothing to do with a `Written` but return it.
#[must_use = "return it from `to_json`"]
#[derive(Debug)]
pub struct Written(pub(crate) ());

/// Writes one JSON value; see [`ToJson`].
///
/// Each method writes a value and consumes the writer. Strings are escaped
/// as [`write_string`](crate::write_string) describes; numbers are written
/// in their shortest exact form; containers are built in a closure that
/// gets an [`ObjectWriter`] or [`ArrayWriter`]. The layout (compact or
/// pretty) is the caller's choice and is applied for you.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{ToJson, ValueWriter, Written};
///
/// struct Celsius(f64);
///
/// impl ToJson for Celsius {
///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
///         out.f64(self.0)
///     }
/// }
///
/// assert_eq!(jsonx_lib::to_string(&Celsius(21.5)), "21.5");
/// assert_eq!(jsonx_lib::to_string(&Celsius(f64::NAN)), "null");
/// ```
pub struct ValueWriter<'w> {
    out: Out<'w>,
}

impl<'w> ValueWriter<'w> {
    #[inline]
    pub(crate) const fn new(out: Out<'w>) -> Self {
        Self { out }
    }

    /// Writes `null`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use jsonx_lib::{ToJson, ValueWriter, Written};
    /// struct Nothing;
    /// impl ToJson for Nothing {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written { out.null() }
    /// }
    /// assert_eq!(jsonx_lib::to_string(&Nothing), "null");
    /// ```
    #[inline]
    pub fn null(mut self) -> Written {
        self.out.buf.push_str("null");
        self.out.done()
    }

    /// Writes `true` or `false`.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::to_string(&true), "true");
    /// ```
    #[inline]
    pub fn bool(mut self, value: bool) -> Written {
        self.out.buf.push_str(if value { "true" } else { "false" });
        self.out.done()
    }

    /// Writes an integer in decimal.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::to_string(&-42i64), "-42");
    /// ```
    #[inline]
    pub fn i64(mut self, value: i64) -> Written {
        number::write_i64(self.out.buf, value);
        self.out.done()
    }

    /// Writes an integer in decimal.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::to_string(&u64::MAX), "18446744073709551615");
    /// ```
    #[inline]
    pub fn u64(mut self, value: u64) -> Written {
        number::write_u64(self.out.buf, value);
        self.out.done()
    }

    /// Writes a 128-bit integer in decimal, exactly (JSON numbers have no
    /// size limit; a reader that stores them as `f64` will round it).
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::to_string(&i128::MIN), "-170141183460469231731687303715884105728");
    /// ```
    #[inline]
    pub fn i128(mut self, value: i128) -> Written {
        number::write_i128(self.out.buf, value);
        self.out.done()
    }

    /// Writes a 128-bit unsigned integer in decimal, exactly.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::to_string(&u128::MAX), "340282366920938463463374607431768211455");
    /// ```
    #[inline]
    pub fn u128(mut self, value: u128) -> Written {
        number::write_u128(self.out.buf, value);
        self.out.done()
    }

    /// Writes the shortest text that reads back as the same `f64`, laid out
    /// as [`Number::from_f64`] describes (`1.0`, `0.1`, `1e100`). NaN and the
    /// infinities, which JSON cannot represent, are written as `null`, as
    /// [`Value::from`] makes them.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::to_string(&1.0f64), "1.0");
    /// assert_eq!(jsonx_lib::to_string(&f64::INFINITY), "null");
    /// ```
    #[inline]
    pub fn f64(mut self, value: f64) -> Written {
        if !number::write_f64(self.out.buf, value) {
            self.out.buf.push_str("null");
        }
        self.out.done()
    }

    /// Writes the shortest text that reads back as the same `f32` (so
    /// `0.1f32` is `0.1`); NaN and the infinities as `null`.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::to_string(&0.1f32), "0.1");
    /// ```
    #[inline]
    pub fn f32(mut self, value: f32) -> Written {
        if !number::write_f32(self.out.buf, value) {
            self.out.buf.push_str("null");
        }
        self.out.done()
    }

    /// Writes a [`Number`]'s exact text.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Number;
    ///
    /// let price = Number::from_text("19.990").unwrap();
    /// assert_eq!(jsonx_lib::to_string(&price), "19.990");
    /// ```
    #[inline]
    pub fn number(mut self, value: &Number<'_>) -> Written {
        self.out.buf.push_str(value.as_str());
        self.out.done()
    }

    /// Writes a string, escaped as [`write_string`](crate::write_string)
    /// describes.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::to_string("say \"hi\""), r#""say \"hi\"""#);
    /// ```
    #[inline]
    pub fn str(mut self, value: &str) -> Written {
        write::write_str(self.out.buf, value, self.out.format);
        self.out.done()
    }

    /// Writes a [`RawValue`]'s text: already valid JSON, so it is not
    /// checked again. In the plain formats it is copied unchanged (in the
    /// pretty layout it keeps its own spacing). Under
    /// [`Format::html_safe`](crate::Format::html_safe) or
    /// [`Format::ascii`](crate::Format::ascii), the characters that format
    /// escapes are escaped inside the raw text's strings too (the only place
    /// valid JSON can hold them), so `</script>` in a forwarded value is
    /// written `\u003c/script\u003e` and the format's promise holds; the
    /// value it stands for is unchanged.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{RawValue, ToJson, ValueWriter, Written};
    ///
    /// struct Forward<'a> { id: u64, result: RawValue<'a> }
    ///
    /// impl ToJson for Forward<'_> {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
    ///         out.object(|o| {
    ///             o.member("id", &self.id);
    ///             o.member_with("result", |w| w.raw(&self.result));
    ///         })
    ///     }
    /// }
    ///
    /// let result = RawValue::new(r#"{"items": [1, 2]}"#)?;
    /// let reply = Forward { id: 3, result };
    /// assert_eq!(jsonx_lib::to_string(&reply), r#"{"id":3,"result":{"items": [1, 2]}}"#);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    pub fn raw(mut self, value: &RawValue<'_>) -> Written {
        crate::write::write_raw(self.out.buf, value.as_str(), self.out.format);
        self.out.done()
    }

    /// Writes a [`Value`] (iterative at any depth), in the writer's layout.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{json, ToJson, Value, ValueWriter, Written};
    ///
    /// struct Extra<'a>(Value<'a>);
    /// impl ToJson for Extra<'_> {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written { out.value(&self.0) }
    /// }
    /// assert_eq!(jsonx_lib::to_string(&Extra(json!([1, {}]))), "[1,{}]");
    /// ```
    pub fn value(mut self, value: &Value<'_>) -> Written {
        let Out {
            buf,
            sink,
            flush_at,
            format,
            depth,
        } = &mut self.out;
        let Ok(()) = write::emit::<Infallible>(buf, value, *format, *depth, *flush_at, &mut |b| {
            sink.flush(b);
            Ok(())
        });
        self.out.done()
    }

    /// Writes `value` through its own [`ToJson`]: `out.json(&x)` is
    /// `x.to_json(out)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ToJson, ValueWriter, Written};
    ///
    /// struct Id(u32);
    /// impl ToJson for Id {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written { out.json(&self.0) }
    /// }
    /// assert_eq!(jsonx_lib::to_string(&Id(7)), "7");
    /// ```
    #[inline]
    pub fn json<T: ToJson + ?Sized>(self, value: &T) -> Written {
        value.to_json(self)
    }

    /// Writes an object whose members `members` adds, in the order it adds
    /// them.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ToJson, ValueWriter, Written};
    ///
    /// struct Position { line: u32, character: u32 }
    ///
    /// impl ToJson for Position {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
    ///         out.object(|o| {
    ///             o.member("line", &self.line);
    ///             o.member("character", &self.character);
    ///         })
    ///     }
    /// }
    ///
    /// let p = Position { line: 3, character: 14 };
    /// assert_eq!(jsonx_lib::to_string(&p), r#"{"line":3,"character":14}"#);
    /// assert_eq!(jsonx_lib::to_string_pretty(&p), "{\n  \"line\": 3,\n  \"character\": 14\n}");
    /// ```
    pub fn object(mut self, members: impl FnOnce(&mut ObjectWriter<'_>)) -> Written {
        self.out.buf.push('{');
        let depth = self.out.depth;
        let mut object = ObjectWriter {
            out: self.out.at(depth + 1),
            first: true,
        };
        members(&mut object);
        let empty = object.first;
        self.close(empty, '}')
    }

    /// Writes an array whose items `items` adds.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ToJson, ValueWriter, Written};
    ///
    /// struct Range { start: u32, end: u32 }
    ///
    /// impl ToJson for Range {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
    ///         out.array(|a| {
    ///             a.item(&self.start);
    ///             a.item(&self.end);
    ///         })
    ///     }
    /// }
    ///
    /// assert_eq!(jsonx_lib::to_string(&Range { start: 1, end: 4 }), "[1,4]");
    /// ```
    pub fn array(mut self, items: impl FnOnce(&mut ArrayWriter<'_>)) -> Written {
        self.out.buf.push('[');
        let depth = self.out.depth;
        let mut array = ArrayWriter {
            out: self.out.at(depth + 1),
            first: true,
        };
        items(&mut array);
        let empty = array.first;
        self.close(empty, ']')
    }

    /// Closes a container: on its own line in the pretty layout, unless it
    /// is empty (`[]`, `{}`).
    fn close(mut self, empty: bool, bracket: char) -> Written {
        if !empty && self.out.format.pretty {
            write::newline(self.out.buf, self.out.depth, self.out.format);
        }
        self.out.buf.push(bracket);
        self.out.done()
    }
}

/// Adds the members of an object being written; see
/// [`ValueWriter::object`].
///
/// Members are written in the order they are added. Nothing stops a key
/// from being added twice; the reader of the output decides what that means
/// (this crate's parser refuses it by default), so add each key once.
pub struct ObjectWriter<'w> {
    out: Out<'w>,
    first: bool,
}

impl ObjectWriter<'_> {
    /// Writes the separator, the indentation, and the key with its colon.
    fn key(&mut self, key: &str) {
        if !self.first {
            self.out.buf.push(',');
        }
        self.first = false;
        if self.out.format.pretty {
            write::newline(self.out.buf, self.out.depth, self.out.format);
        }
        write::write_str(self.out.buf, key, self.out.format);
        self.out
            .buf
            .push_str(if self.out.format.pretty { ": " } else { ":" });
    }

    /// Adds the member `key: value`. An `Option` that is `None` is written as
    /// `null`; to leave the member out instead, use
    /// [`optional`](Self::optional).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ToJson, ValueWriter, Written};
    ///
    /// struct Reply<'a> { id: u64, result: Option<&'a str> }
    ///
    /// impl ToJson for Reply<'_> {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
    ///         out.object(|o| {
    ///             o.member("jsonrpc", "2.0");
    ///             o.member("id", &self.id);
    ///             o.member("result", &self.result); // `null` when None
    ///         })
    ///     }
    /// }
    ///
    /// let reply = Reply { id: 1, result: None };
    /// assert_eq!(jsonx_lib::to_string(&reply), r#"{"jsonrpc":"2.0","id":1,"result":null}"#);
    /// ```
    pub fn member<T: ToJson + ?Sized>(&mut self, key: &str, value: &T) {
        self.key(key);
        let depth = self.out.depth;
        let Written(()) = value.to_json(ValueWriter::new(self.out.at(depth)));
    }

    /// Adds the member `key: value`, unless `value` is absent
    /// ([`ToJson::is_absent`]: an `Option` that is `None`). This is
    /// TypeScript's optional `key?: T`, as LSP and MCP use it.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ToJson, ValueWriter, Written};
    ///
    /// struct Item { label: String, detail: Option<String> }
    ///
    /// impl ToJson for Item {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
    ///         out.object(|o| {
    ///             o.member("label", &self.label);
    ///             o.optional("detail", &self.detail);
    ///         })
    ///     }
    /// }
    ///
    /// let bare = Item { label: "x".into(), detail: None };
    /// assert_eq!(jsonx_lib::to_string(&bare), r#"{"label":"x"}"#);
    /// let full = Item { label: "x".into(), detail: Some("fn x()".into()) };
    /// assert_eq!(jsonx_lib::to_string(&full), r#"{"label":"x","detail":"fn x()"}"#);
    /// ```
    pub fn optional<T: ToJson + ?Sized>(&mut self, key: &str, value: &T) {
        if !value.is_absent() {
            self.member(key, value);
        }
    }

    /// Adds the member `key`, its value written by `value`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ToJson, ValueWriter, Written};
    ///
    /// struct Hover { markdown: String }
    ///
    /// impl ToJson for Hover {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
    ///         out.object(|o| {
    ///             o.member_with("contents", |w| {
    ///                 w.object(|c| {
    ///                     c.member("kind", "markdown");
    ///                     c.member("value", &self.markdown);
    ///                 })
    ///             });
    ///         })
    ///     }
    /// }
    ///
    /// let h = Hover { markdown: "**x**".into() };
    /// assert_eq!(jsonx_lib::to_string(&h), r#"{"contents":{"kind":"markdown","value":"**x**"}}"#);
    /// ```
    pub fn member_with(&mut self, key: &str, value: impl FnOnce(ValueWriter<'_>) -> Written) {
        self.key(key);
        let depth = self.out.depth;
        let Written(()) = value(ValueWriter::new(self.out.at(depth)));
    }
}

/// Adds the items of an array being written; see [`ValueWriter::array`].
pub struct ArrayWriter<'w> {
    out: Out<'w>,
    first: bool,
}

impl ArrayWriter<'_> {
    /// Writes the separator and the indentation, and returns the writer for
    /// the item.
    fn next(&mut self) -> ValueWriter<'_> {
        if !self.first {
            self.out.buf.push(',');
        }
        self.first = false;
        let depth = self.out.depth;
        if self.out.format.pretty {
            write::newline(self.out.buf, depth, self.out.format);
        }
        ValueWriter::new(self.out.at(depth))
    }

    /// Adds `value` as the next item.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ToJson, ValueWriter, Written};
    ///
    /// struct Pair(&'static str, i32);
    /// impl ToJson for Pair {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
    ///         out.array(|a| {
    ///             a.item(self.0);
    ///             a.item(&self.1);
    ///         })
    ///     }
    /// }
    /// assert_eq!(jsonx_lib::to_string(&Pair("x", -1)), r#"["x",-1]"#);
    /// ```
    pub fn item<T: ToJson + ?Sized>(&mut self, value: &T) {
        let Written(()) = value.to_json(self.next());
    }

    /// Adds the next item, written by `value`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ToJson, ValueWriter, Written};
    ///
    /// struct Squares(u32);
    /// impl ToJson for Squares {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
    ///         out.array(|a| {
    ///             for n in 1..=self.0 {
    ///                 a.item_with(|w| w.u64(u64::from(n * n)));
    ///             }
    ///         })
    ///     }
    /// }
    /// assert_eq!(jsonx_lib::to_string(&Squares(3)), "[1,4,9]");
    /// ```
    pub fn item_with(&mut self, value: impl FnOnce(ValueWriter<'_>) -> Written) {
        let Written(()) = value(self.next());
    }

    /// Adds every item of `items`, in order.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ToJson, ValueWriter, Written};
    ///
    /// struct Evens(u32);
    /// impl ToJson for Evens {
    ///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
    ///         out.array(|a| a.items((0..self.0).map(|n| n * 2)))
    ///     }
    /// }
    /// assert_eq!(jsonx_lib::to_string(&Evens(3)), "[0,2,4]");
    /// ```
    pub fn items<I>(&mut self, items: I)
    where
        I: IntoIterator,
        I::Item: ToJson,
    {
        for item in items {
            self.item(&item);
        }
    }
}

/// A type that writes itself as JSON, straight into the output.
///
/// This is the explicit, hand-written half of jsonx-lib's typed API (the
/// other half is [`FromJson`](crate::FromJson)). There is no derive and no
/// reflection: an implementation says, in code, which members a type has and
/// what their keys are, and the compiler checks that it writes exactly one
/// well-formed value. Nothing is built in between: no [`Value`], no
/// intermediate strings.
///
/// # Implementing it
///
/// Write the value with the [`ValueWriter`] you are given and return what
/// its method returns. A struct is an object, a member per field; a newtype
/// usually writes its inner value; an enum writes whichever shape each
/// variant has.
///
/// ```
/// use jsonx_lib::{ToJson, ValueWriter, Written};
///
/// struct Position { line: u32, character: u32 }
/// struct Range { start: Position, end: Position }
/// enum Id { Number(i64), Text(String) }
/// struct Diagnostic { range: Range, message: String, code: Option<Id>, tags: Vec<u8> }
///
/// impl ToJson for Position {
///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
///         out.object(|o| {
///             o.member("line", &self.line);
///             o.member("character", &self.character);
///         })
///     }
/// }
///
/// impl ToJson for Range {
///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
///         out.object(|o| {
///             o.member("start", &self.start);
///             o.member("end", &self.end);
///         })
///     }
/// }
///
/// impl ToJson for Id {
///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
///         match self {
///             Id::Number(n) => out.i64(*n),
///             Id::Text(s) => out.str(s),
///         }
///     }
/// }
///
/// impl ToJson for Diagnostic {
///     fn to_json(&self, out: ValueWriter<'_>) -> Written {
///         out.object(|o| {
///             o.member("range", &self.range);
///             o.member("message", &self.message);
///             o.optional("code", &self.code); // left out when None
///             if !self.tags.is_empty() {
///                 o.member("tags", &self.tags);
///             }
///         })
///     }
/// }
///
/// let d = Diagnostic {
///     range: Range { start: Position { line: 1, character: 0 }, end: Position { line: 1, character: 5 } },
///     message: "unused \"x\"".into(),
///     code: Some(Id::Text("W01".into())),
///     tags: vec![1],
/// };
/// assert_eq!(
///     jsonx_lib::to_string(&d),
///     r#"{"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":5}},"message":"unused \"x\"","code":"W01","tags":[1]}"#
/// );
/// ```
///
/// The rules the writers enforce:
///
/// - **Exactly one value.** Each [`ValueWriter`] method consumes the writer
///   and returns the [`Written`] that `to_json` must return.
/// - **Members only in objects, items only in arrays.** The closure of
///   [`object`](ValueWriter::object) gets an [`ObjectWriter`], which only
///   adds members; [`array`](ValueWriter::array)'s gets an [`ArrayWriter`].
/// - **Escaping is not your job.** Keys and strings are escaped by the
///   writer; no string can break out of its quotes.
/// - **Layout is not your job.** The same impl writes compact
///   ([`to_string`](crate::to_string)) and pretty
///   ([`to_string_pretty`](crate::to_string_pretty)) output.
///
/// Writing never fails from the implementation's point of view: an I/O
/// error of the underlying writer is kept and returned by the call that
/// started the write ([`to_writer`](crate::to_writer),
/// [`Writer::value`](crate::Writer::value)). A type whose values nest as
/// deep as their data (a tree of its own nodes) recurses once per level;
/// [`Value`] does not, and is the type to use for untrusted depth.
///
/// `ToJson` is object-safe: `&dyn ToJson` and `Box<dyn ToJson>` work, for
/// a list of differently typed values.
///
/// # Provided implementations
///
/// `bool`; every integer type (exact decimal); `f32` and `f64` (shortest
/// round-trip text, NaN and the infinities as `null`); `str`, `String`,
/// `Cow<str>`, `Box<str>`, `char`; `()` (`null`); `Option<T>` (`None` is
/// `null`, or left out by [`ObjectWriter::optional`]); `&T`, `&mut T`,
/// `Box<T>`, `Rc<T>`, `Arc<T>`; `[T]`, `[T; N]`, `Vec<T>`, `VecDeque<T>`,
/// `BTreeSet<T>`, and with `std` `HashSet<T, S>` (arrays); tuples of up to
/// eight (arrays); `BTreeMap<K, V>` and with `std` `HashMap<K, V, S>`
/// (objects, keys through [`ToJsonKey`]; a `HashMap` writes in its own
/// order, so use a `BTreeMap` for deterministic output); and this crate's
/// [`Value`], [`Number`], [`Array`], [`Object`], [`RawValue`], and
/// [`Nullable`](crate::Nullable).
pub trait ToJson {
    /// Writes `self` as one JSON value.
    fn to_json(&self, out: ValueWriter<'_>) -> Written;

    /// Whether [`ObjectWriter::optional`] leaves this member out. `false`,
    /// except for `Option<T>`, where it is `is_none()`, and the pointer types,
    /// which ask what they point to.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::ToJson;
    ///
    /// assert!(None::<u8>.is_absent());
    /// assert!(!Some(0u8).is_absent());
    /// assert!(!0u8.is_absent());
    /// ```
    #[inline]
    fn is_absent(&self) -> bool {
        false
    }
}

/// A type usable as a JSON object key, for [`ToJson`] on maps.
///
/// # Examples
///
/// ```
/// use std::borrow::Cow;
/// use std::collections::BTreeMap;
/// use jsonx_lib::ToJsonKey;
///
/// #[derive(PartialEq, Eq, PartialOrd, Ord)]
/// struct Uri(String);
///
/// impl ToJsonKey for Uri {
///     fn to_json_key(&self) -> Cow<'_, str> {
///         Cow::Borrowed(&self.0)
///     }
/// }
///
/// let mut changes = BTreeMap::new();
/// changes.insert(Uri("file:///a.rs".into()), vec![1, 2]);
/// assert_eq!(jsonx_lib::to_string(&changes), r#"{"file:///a.rs":[1,2]}"#);
/// ```
pub trait ToJsonKey {
    /// The key's text (escaped by the writer).
    fn to_json_key(&self) -> Cow<'_, str>;
}

// ---- Implementations ---------------------------------------------------------

impl ToJson for bool {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.bool(*self)
    }
}

macro_rules! to_json_integer {
    ($($t:ty => $method:ident as $via:ty),* $(,)?) => {$(
        impl ToJson for $t {
            #[inline]
            fn to_json(&self, out: ValueWriter<'_>) -> Written {
                out.$method(<$via>::from(*self))
            }
        }
    )*};
}

to_json_integer!(
    i8 => i64 as i64, i16 => i64 as i64, i32 => i64 as i64, i64 => i64 as i64,
    u8 => u64 as u64, u16 => u64 as u64, u32 => u64 as u64, u64 => u64 as u64,
    i128 => i128 as i128, u128 => u128 as u128,
);

impl ToJson for isize {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        // `isize` fits `i128` on every target.
        out.i128(*self as i128)
    }
}

impl ToJson for usize {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.u128(*self as u128)
    }
}

impl ToJson for f64 {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.f64(*self)
    }
}

impl ToJson for f32 {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.f32(*self)
    }
}

impl ToJson for str {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.str(self)
    }
}

impl ToJson for String {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.str(self)
    }
}

impl ToJson for Cow<'_, str> {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.str(self)
    }
}

impl ToJson for char {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        let mut utf8 = [0u8; 4];
        out.str(self.encode_utf8(&mut utf8))
    }
}

impl ToJson for () {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.null()
    }
}

impl<T: ToJson> ToJson for Option<T> {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        match self {
            Some(value) => value.to_json(out),
            None => out.null(),
        }
    }

    #[inline]
    fn is_absent(&self) -> bool {
        self.is_none()
    }
}

macro_rules! to_json_pointer {
    ($($t:ty),*) => {$(
        impl<T: ToJson + ?Sized> ToJson for $t {
            #[inline]
            fn to_json(&self, out: ValueWriter<'_>) -> Written {
                (**self).to_json(out)
            }

            #[inline]
            fn is_absent(&self) -> bool {
                (**self).is_absent()
            }
        }
    )*};
}

to_json_pointer!(&T, &mut T, Box<T>, Rc<T>);
#[cfg(target_has_atomic = "ptr")]
to_json_pointer!(Arc<T>);

impl<T: ToJson> ToJson for [T] {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.array(|a| a.items(self))
    }
}

impl<T: ToJson, const N: usize> ToJson for [T; N] {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        self.as_slice().to_json(out)
    }
}

impl<T: ToJson> ToJson for Vec<T> {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        self.as_slice().to_json(out)
    }
}

impl<T: ToJson> ToJson for VecDeque<T> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.array(|a| a.items(self))
    }
}

impl<T: ToJson> ToJson for BTreeSet<T> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.array(|a| a.items(self))
    }
}

#[cfg(feature = "std")]
impl<T: ToJson, S> ToJson for std::collections::HashSet<T, S> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.array(|a| a.items(self))
    }
}

macro_rules! to_json_tuple {
    ($(($($name:ident),+))*) => {$(
        impl<$($name: ToJson),+> ToJson for ($($name,)+) {
            #[allow(non_snake_case)]
            fn to_json(&self, out: ValueWriter<'_>) -> Written {
                let ($($name,)+) = self;
                out.array(|a| {
                    $(a.item($name);)+
                })
            }
        }
    )*};
}

to_json_tuple! {
    (A)
    (A, B)
    (A, B, C)
    (A, B, C, D)
    (A, B, C, D, E)
    (A, B, C, D, E, F)
    (A, B, C, D, E, F, G)
    (A, B, C, D, E, F, G, H)
}

impl<K: ToJsonKey, V: ToJson> ToJson for BTreeMap<K, V> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.object(|o| {
            for (key, value) in self {
                o.member(&key.to_json_key(), value);
            }
        })
    }
}

#[cfg(feature = "std")]
impl<K: ToJsonKey, V: ToJson, S> ToJson for std::collections::HashMap<K, V, S> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.object(|o| {
            for (key, value) in self {
                o.member(&key.to_json_key(), value);
            }
        })
    }
}

impl ToJson for Value<'_> {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.value(self)
    }
}

impl ToJson for Number<'_> {
    #[inline]
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.number(self)
    }
}

impl ToJson for Array<'_> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.array(|a| a.items(self.iter()))
    }
}

impl ToJson for Object<'_> {
    fn to_json(&self, out: ValueWriter<'_>) -> Written {
        out.object(|o| {
            for (key, value) in self {
                o.member(key, value);
            }
        })
    }
}

impl ToJsonKey for str {
    #[inline]
    fn to_json_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(self)
    }
}

impl ToJsonKey for String {
    #[inline]
    fn to_json_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(self)
    }
}

impl ToJsonKey for Cow<'_, str> {
    #[inline]
    fn to_json_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(self)
    }
}

impl ToJsonKey for Box<str> {
    #[inline]
    fn to_json_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(self)
    }
}

impl ToJsonKey for char {
    #[inline]
    fn to_json_key(&self) -> Cow<'_, str> {
        let mut utf8 = [0u8; 4];
        Cow::Owned(String::from(&*self.encode_utf8(&mut utf8)))
    }
}

impl<T: ToJsonKey + ?Sized> ToJsonKey for &T {
    #[inline]
    fn to_json_key(&self) -> Cow<'_, str> {
        (**self).to_json_key()
    }
}

#[cfg(test)]
mod tests {
    use alloc::{string::ToString, vec};

    use super::*;
    use crate::{json, to_string, to_string_pretty};

    #[test]
    fn test_provided_impls_write_their_shapes() {
        assert_eq!(
            to_string(&(1u8, "two", 3.5f64, None::<u8>, ())),
            r#"[1,"two",3.5,null,null]"#
        );
        assert_eq!(to_string(&[[1, 2], [3, 4]]), "[[1,2],[3,4]]");
        assert_eq!(to_string(&Vec::<u8>::new()), "[]");
        assert_eq!(to_string(&'q'), r#""q""#);
        assert_eq!(to_string(&Box::new(Some(-7i32))), "-7");
        assert_eq!(to_string(&Rc::new("rc")), r#""rc""#);
        assert_eq!(to_string(&VecDeque::from(vec![true])), "[true]");
        assert_eq!(to_string(&BTreeSet::from([2, 1])), "[1,2]");
        let map: BTreeMap<&str, Vec<u8>> = [("b", vec![]), ("a", vec![1])].into_iter().collect();
        assert_eq!(to_string(&map), r#"{"a":[1],"b":[]}"#);
        let chars: BTreeMap<char, u8> = [('x', 1)].into_iter().collect();
        assert_eq!(to_string(&chars), r#"{"x":1}"#);
        let cow: BTreeMap<Cow<'_, str>, Box<str>> =
            [(Cow::Borrowed("k"), Box::from("v"))].into_iter().collect();
        assert_eq!(to_string(&cow), r#"{"k":"v"}"#);
        let boxed: BTreeMap<Box<str>, String> =
            [(Box::from("k"), "v".to_string())].into_iter().collect();
        assert_eq!(to_string(&boxed), r#"{"k":"v"}"#);
        assert_eq!(
            to_string(&(isize::MIN, usize::MAX)),
            alloc::format!("[{},{}]", isize::MIN, usize::MAX)
        );
        assert_eq!(to_string(&(1, 2, 3, 4, 5, 6, 7, 8)), "[1,2,3,4,5,6,7,8]");
        assert_eq!(to_string(&f32::NAN), "null");
        #[cfg(target_has_atomic = "ptr")]
        assert_eq!(to_string(&Arc::new(1u8)), "1");
        let mut n = 5u8;
        assert_eq!(to_string(&&mut n), "5");
    }

    #[test]
    fn test_crate_types_write_as_themselves() {
        let v = json!({ "a": [1, "x", null, {}], "b": { "c": 2.50 } });
        assert_eq!(to_string(&v), v.to_string());
        let arr = v["a"].as_array().cloned().unwrap_or_default();
        assert_eq!(to_string(&arr), r#"[1,"x",null,{}]"#);
        let obj = v.as_object().cloned().unwrap_or_default();
        assert_eq!(to_string(&obj), v.to_string());
        assert_eq!(to_string_pretty(&obj), to_string_pretty(&v));
        assert_eq!(to_string(&Number::from(3)), "3");
    }

    #[test]
    fn test_values_nested_in_typed_output_keep_the_pretty_layout() {
        struct Wrap<'a>(Value<'a>, Vec<u8>);
        impl ToJson for Wrap<'_> {
            fn to_json(&self, out: ValueWriter<'_>) -> Written {
                out.object(|o| {
                    o.member("v", &self.0);
                    o.member("list", &self.1);
                    o.member_with("empty", |w| w.object(|_| {}));
                    o.member_with("none", |w| w.array(|_| {}));
                })
            }
        }
        let w = Wrap(json!({ "x": [1, { "y": [] }] }), vec![7]);
        let expected = json!({
            "v": { "x": [1, { "y": [] }] },
            "list": [7],
            "empty": {},
            "none": [],
        });
        assert_eq!(to_string_pretty(&w), to_string_pretty(&expected));
        assert_eq!(to_string(&w), to_string(&expected));
    }

    #[test]
    fn test_dyn_to_json_and_absence() {
        let parts: [&dyn ToJson; 3] = [&1u8, &"s", &None::<u8>];
        assert_eq!(
            to_string(&parts.as_slice().iter().collect::<Vec<_>>()),
            r#"[1,"s",null]"#
        );
        let boxed: Box<Option<u8>> = Box::new(None);
        assert!(boxed.is_absent());
        assert!(!().is_absent());
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_std_collections() {
        let map: std::collections::HashMap<String, u8> =
            [("k".to_string(), 1)].into_iter().collect();
        assert_eq!(to_string(&map), r#"{"k":1}"#);
        let set: std::collections::HashSet<u8> = [3].into_iter().collect();
        assert_eq!(to_string(&set), "[3]");
    }
}
