//! Output options: layout and escaping.

/// How the writers lay out and escape their output.
///
/// The default is compact JSON (no whitespace) with the escaping described
/// on [`write_string`](crate::write_string): quotes, backslashes, control
/// characters, and U+2028/U+2029, and nothing else. [`pretty`](Self::pretty)
/// is the [`to_string_pretty`](crate::to_string_pretty) layout. Two stricter
/// escapings are available for output that is embedded somewhere JSON's own
/// rules are not enough:
///
/// - [`html_safe`](Self::html_safe) also escapes `<`, `>`, and `&`, so the
///   output can sit inside an HTML `<script>` element or attribute without
///   ending it (`</script>` cannot appear in it);
/// - [`ascii`](Self::ascii) escapes every non-ASCII character as `\uXXXX`
///   (a surrogate pair above U+FFFF), for channels that are not 8-bit clean.
///
/// Every format writes valid JSON that reads back as the same value; they
/// differ only in bytes.
///
/// The struct is `#[non_exhaustive]`: build it from [`Format::compact`] or
/// [`Format::pretty`] and the `with_*` methods.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{json, to_string_with, Format};
///
/// let v = json!({ "html": "</script>", "word": "caf\u{e9}" });
/// let safe = to_string_with(&v, Format::compact().html_safe(true).ascii(true));
/// assert!(!safe.contains('<') && safe.is_ascii());
/// assert_eq!(jsonx_lib::parse(&safe)?, v);
///
/// let wide = to_string_with(&json!([1]), Format::pretty().with_indent(4));
/// assert_eq!(wide, "[\n    1\n]");
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Format {
    pub(crate) pretty: bool,
    pub(crate) indent: u8,
    pub(crate) ascii: bool,
    pub(crate) html: bool,
}

impl Format {
    /// Compact JSON: no whitespace at all. The default.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{json, to_string_with, Format};
    ///
    /// assert_eq!(to_string_with(&json!({ "a": [1, 2] }), Format::compact()), r#"{"a":[1,2]}"#);
    /// ```
    #[must_use]
    pub const fn compact() -> Self {
        Self {
            pretty: false,
            indent: 2,
            ascii: false,
            html: false,
        }
    }

    /// The indented layout of [`to_string_pretty`](crate::to_string_pretty):
    /// one member or item per line, two spaces per level, `": "` after keys,
    /// empty containers as `[]` and `{}`, no trailing newline.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{json, to_string_with, Format};
    ///
    /// assert_eq!(to_string_with(&json!([1]), Format::pretty()), "[\n  1\n]");
    /// ```
    #[must_use]
    pub const fn pretty() -> Self {
        Self {
            pretty: true,
            ..Self::compact()
        }
    }

    /// The indented layout with `spaces` spaces per level (`0` puts each
    /// member on its own line without indenting it).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{json, to_string_with, Format};
    ///
    /// assert_eq!(to_string_with(&json!({ "a": 1 }), Format::compact().with_indent(1)), "{\n \"a\": 1\n}");
    /// ```
    #[must_use]
    pub const fn with_indent(mut self, spaces: u8) -> Self {
        self.pretty = true;
        self.indent = spaces;
        self
    }

    /// Whether to also escape `<`, `>`, and `&` (as `\u003c`, `\u003e`,
    /// `\u0026`), so the output is safe inside HTML.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{to_string_with, Format};
    ///
    /// let text = to_string_with("</script>", Format::compact().html_safe(true));
    /// assert_eq!(text, "\"\\u003c/script\\u003e\"");
    /// ```
    #[must_use]
    pub const fn html_safe(mut self, on: bool) -> Self {
        self.html = on;
        self
    }

    /// Whether to escape every non-ASCII character, so the output is pure
    /// ASCII.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{to_string_with, Format};
    ///
    /// assert_eq!(to_string_with("\u{e9}\u{1f600}", Format::compact().ascii(true)), "\"\\u00e9\\ud83d\\ude00\"");
    /// ```
    #[must_use]
    pub const fn ascii(mut self, on: bool) -> Self {
        self.ascii = on;
        self
    }

    /// Whether this is an indented layout.
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(jsonx_lib::Format::pretty().is_pretty());
    /// assert!(!jsonx_lib::Format::default().is_pretty());
    /// ```
    #[must_use]
    pub const fn is_pretty(&self) -> bool {
        self.pretty
    }

    /// Whether strings need more than the minimal escaping.
    #[inline]
    pub(crate) const fn strict(&self) -> bool {
        self.ascii || self.html
    }
}

impl Default for Format {
    /// [`Format::compact`].
    #[inline]
    fn default() -> Self {
        Self::compact()
    }
}
