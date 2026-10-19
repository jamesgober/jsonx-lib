//! The one error type: what went wrong, and the byte offset where it did.

use core::fmt;

/// Why a document was rejected, and where.
///
/// Every error carries the byte offset of the first byte that made the input
/// invalid. For a syntax error that is the first byte at which the input stops
/// being the beginning of any valid JSON text, so the offset points at the
/// mistake itself, not at the start of the value that contains it. When the
/// input ends too early the offset is the input's length.
///
/// A few errors point at a byte that starts something larger: a lone
/// surrogate at the backslash of its `\u` escape, a duplicate key at the
/// opening quote of its second occurrence, and the budget errors at the start
/// of the value that exceeded the budget. [`ErrorKind`] says which applies.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{parse, ErrorKind};
///
/// let err = parse(r#"{"a": 1,}"#).unwrap_err();
/// assert_eq!(err.kind(), ErrorKind::TrailingComma);
/// assert_eq!(err.offset(), 8);
/// assert_eq!(err.to_string(), "trailing comma before `}` or `]` at byte 8");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct Error {
    kind: ErrorKind,
    offset: usize,
}

impl Error {
    /// Creates an error of `kind` at byte `offset`.
    #[inline]
    pub(crate) const fn new(kind: ErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }

    /// What went wrong.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{parse, ErrorKind};
    ///
    /// assert_eq!(parse("[1 2]").unwrap_err().kind(), ErrorKind::ExpectedCommaOrBracket);
    /// ```
    #[inline]
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The byte offset into the input where the error was found.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::parse;
    ///
    /// // The `x` is the first byte that cannot continue `tru`.
    /// assert_eq!(parse("[tru x]").unwrap_err().offset(), 4);
    /// ```
    #[inline]
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.offset
    }
}

impl Error {
    /// The error's line and column in `input` (the text that was parsed),
    /// both counted from 1, the column in characters: for a message a person
    /// reads. A line ends at `\n`, `\r\n`, or `\r`.
    ///
    /// # Examples
    ///
    /// ```
    /// let text = "{\n  \"a\": tru\n}";
    /// let err = jsonx_lib::parse(text).unwrap_err();
    /// assert_eq!(err.line_col(text), (2, 11));
    /// ```
    #[must_use]
    pub fn line_col(&self, input: impl AsRef<[u8]>) -> (usize, usize) {
        let at = locate(input.as_ref(), self.offset);
        (at.line + 1, at.chars + 1)
    }

    /// The error's position in `input` as the Language Server Protocol
    /// counts it: line and character from 0, the character in UTF-16 code
    /// units (a character above U+FFFF counts two). A line ends at `\n`,
    /// `\r\n`, or `\r`.
    ///
    /// # Examples
    ///
    /// ```
    /// let text = "[\"\u{1f600}\", tru]";
    /// let err = jsonx_lib::parse(text).unwrap_err();
    /// assert_eq!(err.offset(), 12);
    /// assert_eq!(err.utf16_position(text), (0, 10)); // the emoji is two units
    /// ```
    #[must_use]
    pub fn utf16_position(&self, input: impl AsRef<[u8]>) -> (usize, usize) {
        let at = locate(input.as_ref(), self.offset);
        (at.line, at.utf16)
    }
}

/// Where a byte offset falls in a text.
pub(crate) struct Location {
    /// The line, from 0.
    pub(crate) line: usize,
    /// Characters from the start of the line.
    pub(crate) chars: usize,
    /// UTF-16 code units from the start of the line.
    pub(crate) utf16: usize,
}

/// Finds byte `offset` (clamped to the input) in `input`. A byte that is
/// not UTF-8 counts as one character.
pub(crate) fn locate(input: &[u8], offset: usize) -> Location {
    let head = input.get(..offset.min(input.len())).unwrap_or(input);
    let mut line = 0;
    let mut start = 0;
    for (i, &b) in head.iter().enumerate() {
        let ends = match b {
            b'\n' => true,
            // `\r\n` is one line end, counted at the `\n`.
            b'\r' => input.get(i + 1) != Some(&b'\n'),
            _ => false,
        };
        if ends {
            line += 1;
            start = i + 1;
        }
    }
    let text = head.get(start..).unwrap_or(&[]);
    // Every byte but a UTF-8 continuation byte starts a character; a lead
    // byte of a four-byte sequence starts one that is two UTF-16 units.
    let chars = text.iter().filter(|&&b| b & 0xC0 != 0x80).count();
    let wide = text.iter().filter(|&&b| b >= 0xF0).count();
    Location {
        line,
        chars,
        utf16: chars + wide,
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.kind, self.offset)
    }
}

impl core::error::Error for Error {}

/// The kinds of [`Error`].
///
/// The enum is `#[non_exhaustive]`: later versions may report a case more
/// precisely with a new variant, so a `match` needs a wildcard arm.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{parse, ErrorKind};
///
/// match parse("[1, 2").unwrap_err().kind() {
///     ErrorKind::UnexpectedEnd => { /* the input was cut short: wait for more */ }
///     _ => { /* the input is malformed */ }
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The input ended inside a value, or held no value at all. The input is
    /// a truncated document; with a stream, wait for more bytes.
    UnexpectedEnd,
    /// A value was expected and this byte cannot start one (values start with
    /// `{`, `[`, `"`, `-`, a digit, `t`, `f`, or `n`).
    ExpectedValue,
    /// An object key must be followed by `:`.
    ExpectedColon,
    /// Inside an array, a value must be followed by `,` or `]`.
    ExpectedCommaOrBracket,
    /// Inside an object, a member must be followed by `,` or `}`.
    ExpectedCommaOrBrace,
    /// An object key was expected: a member must start with a string.
    ExpectedKey,
    /// A `,` came directly before the `]` or `}` closing its array or
    /// object. JSON does not allow trailing commas.
    TrailingComma,
    /// The document's value ended, but non-whitespace input followed it.
    TrailingCharacters,
    /// The input started `true`, `false`, or `null` but did not finish it.
    InvalidLiteral,
    /// A number broke the RFC 8259 number grammar: a leading zero, a missing
    /// digit after `-`, `.`, or the exponent marker, and so on.
    InvalidNumber,
    /// A backslash in a string was followed by a character that is not one of
    /// `" \ / b f n r t u`.
    InvalidEscape,
    /// A `\u` escape was not followed by four hexadecimal digits.
    InvalidUnicodeEscape,
    /// A `\u` escape named half of a UTF-16 surrogate pair without the other
    /// half. Such a string has no Unicode text to decode to; the offset is the
    /// backslash of the unpaired escape.
    LoneSurrogate,
    /// A string contained an unescaped control character (U+0000 to U+001F).
    /// Control characters must be written as escapes.
    ControlCharacter,
    /// The input was not valid UTF-8 ([`parse_bytes`](crate::parse_bytes)
    /// only). The offset is the first byte of the invalid sequence.
    InvalidUtf8,
    /// An object repeated a key, under [`DuplicateKeys::Error`](crate::DuplicateKeys::Error)
    /// (the default). The offset is the second occurrence's opening quote.
    /// Duplicates are found when the object closes, so a syntax error later in
    /// the same object is reported first.
    DuplicateKey,
    /// Containers nested deeper than [`Limits::max_depth`](crate::Limits::max_depth).
    /// The offset is the `[` or `{` that went one level too deep.
    DepthLimitExceeded,
    /// The input is longer than [`Limits::max_input_bytes`](crate::Limits::max_input_bytes).
    /// The offset is the limit: the first byte past the budget.
    InputTooLarge,
    /// A string (or key) is longer than [`Limits::max_string_bytes`](crate::Limits::max_string_bytes),
    /// counted in source bytes between the quotes. The offset is its opening quote.
    StringTooLong,
    /// A number's text is longer than [`Limits::max_number_bytes`](crate::Limits::max_number_bytes).
    /// The offset is the number's first byte.
    NumberTooLong,
    /// The document holds more values than [`Limits::max_elements`](crate::Limits::max_elements).
    /// The offset is the first byte of the value that went over.
    TooManyElements,
    /// Building the document's tree would hold more memory than
    /// [`Limits::max_memory`](crate::Limits::max_memory). The offset is the
    /// first byte of the value whose storage went over.
    MemoryLimitExceeded,
    /// Bytes were fed to a [`StreamReader`](crate::StreamReader) after
    /// [`end_input`](crate::StreamReader::end_input) said the input had
    /// ended. A mistake in the calling code, not in the document; the
    /// offset is the number of bytes accepted before the end.
    InputAfterEnd,
}

impl ErrorKind {
    /// Whether this error comes from a budget in [`Limits`](crate::Limits)
    /// rather than from the input being invalid JSON. The same input may parse
    /// with larger limits.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::ErrorKind;
    ///
    /// assert!(ErrorKind::DepthLimitExceeded.is_budget());
    /// assert!(!ErrorKind::TrailingComma.is_budget());
    /// ```
    #[must_use]
    pub const fn is_budget(self) -> bool {
        matches!(
            self,
            Self::DepthLimitExceeded
                | Self::InputTooLarge
                | Self::StringTooLong
                | Self::NumberTooLong
                | Self::TooManyElements
                | Self::MemoryLimitExceeded
        )
    }

    const fn message(self) -> &'static str {
        match self {
            Self::UnexpectedEnd => "unexpected end of input",
            Self::ExpectedValue => "expected a value",
            Self::ExpectedColon => "expected `:` after an object key",
            Self::ExpectedCommaOrBracket => "expected `,` or `]` after an array element",
            Self::ExpectedCommaOrBrace => "expected `,` or `}` after an object member",
            Self::ExpectedKey => "expected a string key",
            Self::TrailingComma => "trailing comma before `}` or `]`",
            Self::TrailingCharacters => "unexpected input after the document's value",
            Self::InvalidLiteral => "invalid literal (expected `true`, `false`, or `null`)",
            Self::InvalidNumber => "invalid number",
            Self::InvalidEscape => "invalid escape sequence",
            Self::InvalidUnicodeEscape => "`\\u` must be followed by four hex digits",
            Self::LoneSurrogate => "unpaired UTF-16 surrogate escape",
            Self::ControlCharacter => "unescaped control character in a string",
            Self::InvalidUtf8 => "invalid UTF-8",
            Self::DuplicateKey => "duplicate object key",
            Self::DepthLimitExceeded => "nesting depth limit exceeded",
            Self::InputTooLarge => "input size limit exceeded",
            Self::StringTooLong => "string length limit exceeded",
            Self::NumberTooLong => "number length limit exceeded",
            Self::TooManyElements => "element count limit exceeded",
            Self::MemoryLimitExceeded => "memory limit exceeded",
            Self::InputAfterEnd => "input fed after the end of input",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn test_error_display_names_kind_and_offset() {
        let e = Error::new(ErrorKind::ExpectedColon, 7);
        assert_eq!(e.to_string(), "expected `:` after an object key at byte 7");
        assert_eq!(e.kind(), ErrorKind::ExpectedColon);
        assert_eq!(e.offset(), 7);
    }

    #[test]
    fn test_locate_counts_lines_and_columns() {
        let at = |text: &str, offset| {
            let l = locate(text.as_bytes(), offset);
            (l.line, l.chars, l.utf16)
        };
        assert_eq!(at("abc", 2), (0, 2, 2));
        assert_eq!(at("a\nbc", 3), (1, 1, 1));
        assert_eq!(at("a\r\nbc", 4), (1, 1, 1));
        assert_eq!(at("a\rbc", 3), (1, 1, 1));
        assert_eq!(at("a\r\n", 2), (0, 2, 2)); // inside the `\r\n`
        assert_eq!(at("\u{e9}\u{1f600}x", 6), (0, 2, 3));
        assert_eq!(at("ab", 99), (0, 2, 2));
        let e = Error::new(ErrorKind::ExpectedValue, 5);
        assert_eq!(e.line_col("[\n  ]x"), (2, 4));
        assert_eq!(e.utf16_position(b"[\n  ]x".as_slice()), (1, 3));
    }

    #[test]
    fn test_error_kind_budget_classification_is_exact() {
        let all = [
            ErrorKind::UnexpectedEnd,
            ErrorKind::ExpectedValue,
            ErrorKind::ExpectedColon,
            ErrorKind::ExpectedCommaOrBracket,
            ErrorKind::ExpectedCommaOrBrace,
            ErrorKind::ExpectedKey,
            ErrorKind::TrailingComma,
            ErrorKind::TrailingCharacters,
            ErrorKind::InvalidLiteral,
            ErrorKind::InvalidNumber,
            ErrorKind::InvalidEscape,
            ErrorKind::InvalidUnicodeEscape,
            ErrorKind::LoneSurrogate,
            ErrorKind::ControlCharacter,
            ErrorKind::InvalidUtf8,
            ErrorKind::DuplicateKey,
            ErrorKind::DepthLimitExceeded,
            ErrorKind::InputTooLarge,
            ErrorKind::StringTooLong,
            ErrorKind::NumberTooLong,
            ErrorKind::TooManyElements,
            ErrorKind::MemoryLimitExceeded,
            ErrorKind::InputAfterEnd,
        ];
        let budgets = all.iter().filter(|k| k.is_budget()).count();
        assert_eq!(budgets, 6);
        for kind in all {
            assert!(!kind.to_string().is_empty());
        }
    }
}
