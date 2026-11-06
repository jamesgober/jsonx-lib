//! Output options: layout, escaping, and float text.

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
/// Both apply to everything written, [`RawValue`](crate::RawValue) text
/// included (its strings are escaped the same way).
///
/// Two options change the text of floats written from Rust values
/// ([`ValueWriter::f64`](crate::ValueWriter::f64) and
/// [`f32`](crate::ValueWriter::f32)), for output that must match another
/// writer's byte for byte: [`exponent_plus`](Self::exponent_plus) writes
/// `1e+16` for `1e16`, and [`even_ties`](Self::even_ties) changes which of
/// two equally close shortest texts is written. A [`Number`](crate::Number)
/// keeps its own text in every format. Every format writes
/// valid JSON that reads back as the same value (for objects without
/// repeated keys: the writers do not check for them; see
/// [`ObjectWriter`](crate::ObjectWriter)); the formats differ only in
/// bytes.
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
    pub(crate) exponent_plus: bool,
    pub(crate) even_ties: bool,
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
            exponent_plus: false,
            even_ties: false,
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

    /// Whether to write the `+` of a positive exponent in float text:
    /// `1e+16` rather than `1e16`, as `serde_json` (1.0.147 and later),
    /// JavaScript and Python write it. Negative exponents always have their
    /// `-`; floats from 1e-5 up to below 1e16 are written as decimals and
    /// have no exponent.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{to_string_with, Format};
    ///
    /// let plus = Format::compact().exponent_plus(true);
    /// assert_eq!(to_string_with(&[1e16, 2.5e-7, 0.5], plus), "[1e+16,2.5e-7,0.5]");
    /// assert_eq!(to_string_with(&1e16, Format::compact()), "1e16");
    /// ```
    #[must_use]
    pub const fn exponent_plus(mut self, on: bool) -> Self {
        self.exponent_plus = on;
        self
    }

    /// Whether an exact tie in float text goes to the even digit.
    ///
    /// A float is written as the shortest decimal that reads back as it, and
    /// of those the closest. Rarely two are equally close (the float lies
    /// exactly halfway between them; it takes 16 or 17 significant digits
    /// for an `f64`). Both read back as the same float. By default the higher
    /// one is written, as Rust's own formatting (`{}`, `{:e}`) picks it; with
    /// this option, the one whose last digit is even, as `serde_json`, Java,
    /// JavaScript and Python pick it.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{to_string_with, Format};
    ///
    /// // 164696460098373.625 lies exactly between ...373.62 and ...373.63.
    /// let x = 164_696_460_098_373.625_f64;
    /// assert_eq!(to_string_with(&x, Format::compact()), "164696460098373.63");
    /// assert_eq!(to_string_with(&x, Format::compact().even_ties(true)), "164696460098373.62");
    /// assert_eq!(format!("{x}"), "164696460098373.63");
    /// ```
    #[must_use]
    pub const fn even_ties(mut self, on: bool) -> Self {
        self.even_ties = on;
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

    /// How an exact tie in float text is broken.
    #[inline]
    pub(crate) const fn ties(&self) -> crate::float::Ties {
        if self.even_ties {
            crate::float::Ties::Even
        } else {
            crate::float::Ties::Up
        }
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
