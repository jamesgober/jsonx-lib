//! The writer: deterministic compact and pretty output with complete escaping.

use alloc::{borrow::Cow, string::String};
use core::{fmt, slice};

use crate::{
    Format, ToJson, Value,
    encode::{Keep, Out, ValueWriter, Written},
    scan,
    tree::Member,
};

/// Spaces for indentation, copied in slices.
const SPACES: &str = "                                                                ";

/// Appends `text` to `out` as a JSON string literal, quotes included.
///
/// The escaping is complete and the same every time:
///
/// - `"` and `\` are escaped as `\"` and `\\`;
/// - the control characters U+0000 to U+001F are escaped, with the short forms
///   `\b \f \n \r \t` where JSON has one and `\u00XX` (lowercase hex)
///   otherwise;
/// - U+2028 LINE SEPARATOR and U+2029 PARAGRAPH SEPARATOR are escaped as
///   `\u2028` and `\u2029`: JSON allows them raw, but JavaScript before ES2019
///   ends a line at them, so raw ones break JSON embedded in a script;
/// - everything else, including `/`, DEL, and all other non-ASCII text, is
///   copied unchanged as UTF-8.
///
/// This escaping is not HTML-safe: `<`, `>`, and `&` are copied, so
/// `</script>` inside a string stays `</script>` in the output. For JSON
/// placed inside HTML, write with [`Format::html_safe`] (and
/// [`Format::ascii`] for a channel that is not 8-bit clean).
///
/// So no string can end the literal early or put a raw line break in it,
/// whatever it contains: the output is always exactly one JSON string token
/// that decodes back to `text`. Unescaped stretches are copied in bulk, found
/// eight bytes at a time.
///
/// # Examples
///
/// ```
/// let mut out = String::from("{\"msg\":");
/// jsonx_lib::write_string(&mut out, "say \"hi\"\n\u{2028}");
/// out.push('}');
/// assert_eq!(out, r#"{"msg":"say \"hi\"\n\u2028"}"#);
/// ```
pub fn write_string(out: &mut String, text: &str) {
    out.reserve(text.len() + 2);
    out.push('"');
    let bytes = text.as_bytes();
    let mut from = 0;
    let mut at = 0;
    loop {
        at = scan::find_write_special(bytes, at);
        let Some(&b) = bytes.get(at) else { break };
        let escape: &str = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0x08 => "\\b",
            0x0C => "\\f",
            0xE2 => match bytes.get(at + 1..at + 3) {
                Some([0x80, 0xA8]) => "\\u2028",
                Some([0x80, 0xA9]) => "\\u2029",
                // Another character starting with 0xE2: copy it as is.
                _ => {
                    at += 1;
                    continue;
                }
            },
            control => {
                out.push_str(text.get(from..at).unwrap_or(""));
                out.push_str("\\u00");
                out.push(char::from(scan::hex_digit(control >> 4)));
                out.push(char::from(scan::hex_digit(control)));
                at += 1;
                from = at;
                continue;
            }
        };
        out.push_str(text.get(from..at).unwrap_or(""));
        out.push_str(escape);
        // The line separators are three bytes; every other escaped byte, one.
        at += if b == 0xE2 { 3 } else { 1 };
        from = at;
    }
    out.push_str(text.get(from..).unwrap_or(""));
    out.push('"');
}

/// Appends `text` as a JSON string literal escaped as `format` asks:
/// [`write_string`] unless the format is [`html_safe`](Format::html_safe)
/// or [`ascii`](Format::ascii).
#[inline]
pub(crate) fn write_str(out: &mut String, text: &str, format: Format) {
    if format.strict() {
        write_string_strict(out, text, format);
    } else {
        write_string(out, text);
    }
}

/// [`write_string`] with the stricter escaping of `format`: `<`, `>`, and
/// `&` when HTML-safe, every non-ASCII character when ASCII-only.
fn write_string_strict(out: &mut String, text: &str, format: Format) {
    out.reserve(text.len() + 2);
    out.push('"');
    let mut from = 0;
    for (at, ch) in text.char_indices() {
        let escape = match ch {
            '"' | '\\' | '\u{0}'..='\u{1f}' | '\u{2028}' | '\u{2029}' => true,
            '<' | '>' | '&' => format.html,
            _ => format.ascii && !ch.is_ascii(),
        };
        if !escape {
            continue;
        }
        out.push_str(text.get(from..at).unwrap_or(""));
        from = at + ch.len_utf8();
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            _ => {
                let mut units = [0u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    push_unicode_escape(out, *unit);
                }
            }
        }
    }
    out.push_str(text.get(from..).unwrap_or(""));
    out.push('"');
}

/// Appends `raw`, the text of one valid JSON value, in `format`: unchanged
/// in the plain formats; under a strict one, with every character that
/// format escapes in strings (`<`, `>`, `&` when HTML-safe, anything
/// non-ASCII when ASCII-only, U+2028 and U+2029 in both) written as its
/// `\uXXXX` escape. In valid JSON those characters can only occur inside
/// strings, where the escape stands for the same character, so the value is
/// unchanged and the format's byte guarantees hold for raw text too.
pub(crate) fn write_raw(out: &mut String, raw: &str, format: Format) {
    if !format.strict() {
        out.push_str(raw);
        return;
    }
    out.reserve(raw.len());
    let mut from = 0;
    for (at, ch) in raw.char_indices() {
        let escape = match ch {
            '\u{2028}' | '\u{2029}' => true,
            '<' | '>' | '&' => format.html,
            _ => format.ascii && !ch.is_ascii(),
        };
        if !escape {
            continue;
        }
        out.push_str(raw.get(from..at).unwrap_or(""));
        from = at + ch.len_utf8();
        let mut units = [0u16; 2];
        for unit in ch.encode_utf16(&mut units) {
            push_unicode_escape(out, *unit);
        }
    }
    out.push_str(raw.get(from..).unwrap_or(""));
}

/// Appends the six-character escape for one UTF-16 code unit, lowercase.
fn push_unicode_escape(out: &mut String, unit: u16) {
    out.push('\\');
    out.push('u');
    for shift in [12, 8, 4, 0] {
        // The nibble is below 16.
        out.push(char::from(scan::hex_digit((unit >> shift) as u8)));
    }
}

/// Serializes `value` as compact JSON: no whitespace at all.
///
/// `value` is anything that implements [`ToJson`]: a [`Value`], or your own
/// type, written straight to text without building a `Value`. The output is
/// deterministic: members in their order, numbers as their exact text,
/// strings escaped as [`write_string`] describes. For a `Value`, parsing the
/// output gives back an equal value.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{json, to_string};
///
/// let v = json!({ "id": 1, "tags": ["a", "b"], "none": null });
/// assert_eq!(to_string(&v), r#"{"id":1,"tags":["a","b"],"none":null}"#);
/// assert_eq!(to_string(&[Some(1), None]), "[1,null]");
/// ```
#[must_use]
pub fn to_string<T: ToJson + ?Sized>(value: &T) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

/// Serializes `value` as indented JSON: two spaces per level, one member or
/// item per line, `": "` after keys, empty containers as `[]` and `{}`, and
/// no trailing newline.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{json, to_string_pretty};
///
/// let v = json!({ "a": [1, 2], "b": {} });
/// assert_eq!(to_string_pretty(&v), "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": {}\n}");
/// ```
#[must_use]
pub fn to_string_pretty<T: ToJson + ?Sized>(value: &T) -> String {
    let mut out = String::new();
    write_value_pretty(&mut out, value);
    out
}

/// Appends `value` to `out` as compact JSON (see [`to_string`]). Reusing one
/// buffer across documents avoids an allocation per document.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{json, write_value};
///
/// let mut out = String::from("Content: ");
/// write_value(&mut out, &json!([true]));
/// assert_eq!(out, "Content: [true]");
/// ```
pub fn write_value<T: ToJson + ?Sized>(out: &mut String, value: &T) {
    write_value_with(out, value, Format::compact());
}

/// Appends `value` to `out` as indented JSON (see [`to_string_pretty`]).
///
/// # Examples
///
/// ```
/// use jsonx_lib::{json, write_value_pretty};
///
/// let mut out = String::new();
/// write_value_pretty(&mut out, &json!([1]));
/// assert_eq!(out, "[\n  1\n]");
/// ```
pub fn write_value_pretty<T: ToJson + ?Sized>(out: &mut String, value: &T) {
    write_value_with(out, value, Format::pretty());
}

/// Serializes `value` in the given [`Format`].
///
/// # Examples
///
/// ```
/// use jsonx_lib::{json, to_string_with, Format};
///
/// let v = json!({ "tag": "<b>" });
/// assert_eq!(to_string_with(&v, Format::compact().html_safe(true)), r#"{"tag":"\u003cb\u003e"}"#);
/// ```
#[must_use]
pub fn to_string_with<T: ToJson + ?Sized>(value: &T, format: Format) -> String {
    let mut out = String::new();
    write_value_with(&mut out, value, format);
    out
}

/// Appends `value` to `out` in the given [`Format`].
///
/// # Examples
///
/// ```
/// use jsonx_lib::{write_value_with, Format};
///
/// let mut out = String::from("data: ");
/// write_value_with(&mut out, &[1, 2], Format::pretty().with_indent(0));
/// assert_eq!(out, "data: [\n1,\n2\n]");
/// ```
pub fn write_value_with<T: ToJson + ?Sized>(out: &mut String, value: &T, format: Format) {
    let mut keep = Keep;
    let writer = ValueWriter::new(Out {
        buf: out,
        sink: &mut keep,
        flush_at: usize::MAX,
        format,
        depth: 0,
    });
    let Written(()) = value.to_json(writer);
}

/// Writes `value` as compact JSON to an [`io::Write`](std::io::Write),
/// through a buffer of a few kilobytes, so a large document is never held as
/// one string.
///
/// # Errors
///
/// Any error from `writer`.
///
/// # Examples
///
/// ```
/// let mut out = Vec::new();
/// jsonx_lib::to_writer(&mut out, &jsonx_lib::json!({ "ok": true }))?;
/// assert_eq!(out, br#"{"ok":true}"#);
/// # Ok::<(), std::io::Error>(())
/// ```
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub fn to_writer<W: std::io::Write, T: ToJson + ?Sized>(
    writer: W,
    value: &T,
) -> std::io::Result<()> {
    to_writer_with(writer, value, Format::compact())
}

/// Writes `value` as indented JSON (see [`to_string_pretty`]) to an
/// [`io::Write`](std::io::Write), through a buffer of a few kilobytes.
///
/// # Errors
///
/// Any error from `writer`.
///
/// # Examples
///
/// ```
/// let mut out = Vec::new();
/// jsonx_lib::to_writer_pretty(&mut out, &jsonx_lib::json!([null]))?;
/// assert_eq!(out, b"[\n  null\n]");
/// # Ok::<(), std::io::Error>(())
/// ```
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub fn to_writer_pretty<W: std::io::Write, T: ToJson + ?Sized>(
    writer: W,
    value: &T,
) -> std::io::Result<()> {
    to_writer_with(writer, value, Format::pretty())
}

/// How much output the streaming writers collect before passing it on.
pub(crate) const CHUNK: usize = 8 * 1024;

/// Writes `value` in the given [`Format`] to an [`io::Write`](std::io::Write),
/// through a buffer of a few kilobytes.
///
/// # Errors
///
/// Any error from `writer`.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{json, to_writer_with, Format};
///
/// let mut out = Vec::new();
/// to_writer_with(&mut out, &json!(["caf\u{e9}"]), Format::compact().ascii(true))?;
/// assert_eq!(out, b"[\"caf\\u00e9\"]");
/// # Ok::<(), std::io::Error>(())
/// ```
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub fn to_writer_with<W: std::io::Write, T: ToJson + ?Sized>(
    mut writer: W,
    value: &T,
    format: Format,
) -> std::io::Result<()> {
    let mut buf = String::with_capacity(CHUNK + 256);
    let mut sink = crate::encode::IoSink {
        writer: &mut writer,
        error: None,
    };
    let out = Out {
        buf: &mut buf,
        sink: &mut sink,
        flush_at: CHUNK,
        format,
        depth: 0,
    };
    let Written(()) = value.to_json(ValueWriter::new(out));
    crate::encode::Flush::flush(&mut sink, &mut buf);
    sink.error.map_or(Ok(()), Err)
}

/// Appends a newline and the indentation for `depth` levels.
pub(crate) fn newline(out: &mut String, depth: usize, format: Format) {
    out.push('\n');
    let mut width = depth.saturating_mul(usize::from(format.indent));
    while width > 0 {
        let chunk = width.min(SPACES.len());
        out.push_str(SPACES.get(..chunk).unwrap_or(""));
        width -= chunk;
    }
}

/// The children of an open container still to be written.
enum Open<'r, 'a> {
    Array(slice::Iter<'r, Value<'a>>),
    Object(slice::Iter<'r, Member<'a>>),
}

/// Writes `value` into `out`, handing `out` to `flush` whenever it holds at
/// least `flush_at` bytes (the flush is expected to empty it). In the pretty
/// layout, the value starts at indentation level `base`.
///
/// Iterative: an explicit stack of open containers replaces recursion, so any
/// depth is safe.
pub(crate) fn emit<E>(
    out: &mut String,
    value: &Value<'_>,
    format: Format,
    base: usize,
    flush_at: usize,
    flush: &mut dyn FnMut(&mut String) -> Result<(), E>,
) -> Result<(), E> {
    // One copy of the loop per layout, so the common one (compact, plainly
    // escaped) tests neither per value.
    match (format.pretty, format.strict()) {
        (false, false) => emit_as::<E, false, false>(out, value, format, base, flush_at, flush),
        (false, true) => emit_as::<E, false, true>(out, value, format, base, flush_at, flush),
        (true, false) => emit_as::<E, true, false>(out, value, format, base, flush_at, flush),
        (true, true) => emit_as::<E, true, true>(out, value, format, base, flush_at, flush),
    }
}

/// [`emit`] for one layout: `PRETTY` is `format.pretty`, `STRICT` is
/// `format.strict()`.
fn emit_as<E, const PRETTY: bool, const STRICT: bool>(
    out: &mut String,
    value: &Value<'_>,
    format: Format,
    base: usize,
    flush_at: usize,
    flush: &mut dyn FnMut(&mut String) -> Result<(), E>,
) -> Result<(), E> {
    let write_text = |out: &mut String, text: &str| {
        if STRICT {
            write_string_strict(out, text, format);
        } else {
            write_string(out, text);
        }
    };
    let mut stack: alloc::vec::Vec<(Open<'_, '_>, bool)> = alloc::vec::Vec::new();
    let mut next = Some(value);
    loop {
        if let Some(value) = next.take() {
            match value {
                Value::Null => out.push_str("null"),
                Value::Bool(true) => out.push_str("true"),
                Value::Bool(false) => out.push_str("false"),
                Value::Number(n) => out.push_str(n.as_str()),
                Value::String(s) => write_text(out, s),
                Value::Array(a) if a.is_empty() => out.push_str("[]"),
                Value::Object(o) if o.is_empty() => out.push_str("{}"),
                Value::Array(a) => {
                    out.push('[');
                    stack.push((Open::Array(a.iter()), true));
                }
                Value::Object(o) => {
                    out.push('{');
                    stack.push((Open::Object(o.as_slice().iter()), true));
                }
            }
            if out.len() >= flush_at {
                flush(out)?;
            }
        }
        let depth = stack.len();
        let Some((open, first)) = stack.last_mut() else {
            return Ok(());
        };
        let (child, key): (Option<&Value<'_>>, Option<&Cow<'_, str>>) = match open {
            Open::Array(it) => (it.next(), None),
            Open::Object(it) => match it.next() {
                Some((k, v)) => (Some(v), Some(k)),
                None => (None, None),
            },
        };
        let Some(child) = child else {
            let close = if matches!(open, Open::Array(_)) {
                ']'
            } else {
                '}'
            };
            let _closed = stack.pop();
            if PRETTY {
                newline(out, base + depth - 1, format);
            }
            out.push(close);
            continue;
        };
        if !*first {
            out.push(',');
        }
        *first = false;
        if PRETTY {
            newline(out, base + depth, format);
        }
        if let Some(key) = key {
            write_text(out, key);
            out.push_str(if PRETTY { ": " } else { ":" });
        }
        next = Some(child);
    }
}

impl fmt::Display for Value<'_> {
    /// Writes the value as compact JSON, or as indented JSON with the
    /// alternate flag (`{:#}`).
    ///
    /// # Examples
    ///
    /// ```
    /// let v = jsonx_lib::json!({ "a": [1] });
    /// assert_eq!(format!("{v}"), r#"{"a":[1]}"#);
    /// assert_eq!(format!("{v:#}"), "{\n  \"a\": [\n    1\n  ]\n}");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const CHUNK: usize = 4 * 1024;
        let mut buf = String::new();
        emit(
            &mut buf,
            self,
            if f.alternate() {
                Format::pretty()
            } else {
                Format::compact()
            },
            0,
            CHUNK,
            &mut |chunk: &mut String| {
                let result = f.write_str(chunk);
                chunk.clear();
                result
            },
        )?;
        f.write_str(&buf)
    }
}

impl fmt::Debug for Value<'_> {
    /// Writes the value as compact JSON.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    fn quoted(text: &str) -> String {
        let mut out = String::new();
        write_string(&mut out, text);
        out
    }

    #[test]
    fn test_write_string_escapes_exactly() {
        assert_eq!(quoted(""), r#""""#);
        assert_eq!(quoted("plain/text"), r#""plain/text""#);
        assert_eq!(quoted("\"\\"), r#""\"\\""#);
        assert_eq!(quoted("\u{8}\u{c}\n\r\t"), r#""\b\f\n\r\t""#);
        assert_eq!(
            quoted("\u{0}\u{1}\u{1f}\u{7f}"),
            "\"\\u0000\\u0001\\u001f\u{7f}\""
        );
        assert_eq!(quoted("a\u{2028}b\u{2029}c"), r#""a\u2028b\u2029c""#);
        // Other characters starting with 0xE2 pass through.
        assert_eq!(
            quoted("\u{20ac}\u{2027}\u{202a}"),
            "\"\u{20ac}\u{2027}\u{202a}\""
        );
        assert_eq!(
            quoted("long run of text then \" quote"),
            r#""long run of text then \" quote""#
        );
    }

    #[test]
    fn test_pretty_layout_nested_and_empty() {
        let v = crate::parse(r#"{"a":[],"b":{},"c":[{"d":null}]}"#).unwrap_or_default();
        assert_eq!(
            to_string_pretty(&v),
            "{\n  \"a\": [],\n  \"b\": {},\n  \"c\": [\n    {\n      \"d\": null\n    }\n  ]\n}"
        );
        assert_eq!(to_string_pretty(&Value::from(1)), "1");
    }

    #[test]
    fn test_display_flushes_in_chunks_identically() {
        let mut big = String::from("[");
        for i in 0..5000 {
            big.push_str(&alloc::format!("{i},"));
        }
        big.push_str("0]");
        let v = crate::parse(&big).unwrap_or_default();
        assert_eq!(v.to_string(), big);
    }

    #[test]
    fn test_deep_indent_beyond_spaces_buffer() {
        let depth = 40;
        let text = "[".repeat(depth) + &"]".repeat(depth);
        let v = crate::parse(&text).unwrap_or_default();
        let pretty = to_string_pretty(&v);
        assert!(pretty.contains(&alloc::format!("\n{}[]", " ".repeat((depth - 1) * 2))));
    }
}
