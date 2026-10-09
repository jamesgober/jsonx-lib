//! The strict tokenizer: one string, number, or literal token at a byte
//! offset, with the exact offset of the first byte that breaks it.
//!
//! Every function here works on the raw bytes of a `&str` and reports the
//! first byte at which the input stops being the beginning of a valid token,
//! so an error points at the mistake rather than at the token that holds it.
//! Running off the end of the input is always [`ErrorKind::UnexpectedEnd`] at
//! the input's length: the text so far could still have been valid.

use alloc::string::String;

use crate::{
    error::{Error, ErrorKind},
    scan,
};

/// A scanned string token.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StrToken {
    /// The byte just past the closing quote.
    pub(crate) end: usize,
    /// Whether the body holds an escape sequence (and so must be decoded).
    pub(crate) escaped: bool,
}

#[inline]
const fn err<T>(kind: ErrorKind, offset: usize) -> Result<T, Error> {
    Err(Error::new(kind, offset))
}

/// Scans the string whose opening quote is at `quote`, validating every escape
/// and refusing a body longer than `max` source bytes.
///
/// Nothing is decoded or allocated here: a string with escapes is validated in
/// this pass and decoded by [`decode`] in a second, so the decoder can size its
/// buffer exactly, and only after the length budget has passed.
pub(crate) fn scan_string(bytes: &[u8], quote: usize, max: usize) -> Result<StrToken, Error> {
    let len = bytes.len();
    let body = quote + 1;
    // The closing quote may sit at `body + max` at the latest; nothing past
    // that needs looking at before the budget refuses the string.
    let cap = body.saturating_add(max).saturating_add(1).min(len);
    let mut i = body;
    let mut escaped = false;
    loop {
        i = scan::find_read_special(bytes, i, cap);
        let byte = if i < cap { bytes.get(i).copied() } else { None };
        let Some(b) = byte else {
            return if i - body > max {
                err(ErrorKind::StringTooLong, quote)
            } else {
                err(ErrorKind::UnexpectedEnd, len)
            };
        };
        match b {
            b'"' => {
                return Ok(StrToken {
                    end: i + 1,
                    escaped,
                });
            }
            b'\\' => {
                escaped = true;
                i = scan_escape(bytes, i)?;
                if i - body > max {
                    return err(ErrorKind::StringTooLong, quote);
                }
            }
            // `find_read_special` stops only at a quote, a backslash, or a
            // control character.
            _ => return err(ErrorKind::ControlCharacter, i),
        }
    }
}

/// Whether `b` may follow a backslash as a one-character escape.
#[inline]
const fn is_short_escape(b: u8) -> bool {
    matches!(b, b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't')
}

/// Validates the escape whose backslash is at `at` and returns the offset just
/// past it. A high surrogate must be followed at once by a `\u` escape naming
/// a low one; the pair is one escape here.
fn scan_escape(bytes: &[u8], at: usize) -> Result<usize, Error> {
    let len = bytes.len();
    match bytes.get(at + 1) {
        None => err(ErrorKind::UnexpectedEnd, len),
        Some(&b) if is_short_escape(b) => Ok(at + 2),
        Some(b'u') => {
            let unit = hex4(bytes, at + 2)?;
            match unit {
                0xD800..=0xDBFF => {
                    let next = at + 6;
                    match bytes.get(next) {
                        None => return err(ErrorKind::UnexpectedEnd, len),
                        Some(b'\\') => {}
                        Some(_) => return err(ErrorKind::LoneSurrogate, at),
                    }
                    match bytes.get(next + 1) {
                        None => err(ErrorKind::UnexpectedEnd, len),
                        Some(b'u') => {
                            let low = hex4(bytes, next + 2)?;
                            if (0xDC00..=0xDFFF).contains(&low) {
                                Ok(next + 6)
                            } else {
                                err(ErrorKind::LoneSurrogate, at)
                            }
                        }
                        // A well-formed escape that is not the low half.
                        Some(&b) if is_short_escape(b) => err(ErrorKind::LoneSurrogate, at),
                        Some(_) => err(ErrorKind::InvalidEscape, next + 1),
                    }
                }
                0xDC00..=0xDFFF => err(ErrorKind::LoneSurrogate, at),
                _ => Ok(at + 6),
            }
        }
        Some(_) => err(ErrorKind::InvalidEscape, at + 1),
    }
}

/// Reads the four hex digits of a `\u` escape starting at `at`.
fn hex4(bytes: &[u8], at: usize) -> Result<u32, Error> {
    let mut value = 0;
    for k in 0..4 {
        let Some(&b) = bytes.get(at + k) else {
            return err(ErrorKind::UnexpectedEnd, bytes.len());
        };
        let Some(digit) = scan::hex_value(b) else {
            return err(ErrorKind::InvalidUnicodeEscape, at + k);
        };
        value = (value << 4) | digit;
    }
    Ok(value)
}

/// Reads four hex digits already validated by [`scan_string`].
fn hex4_valid(bytes: &[u8], at: usize) -> u32 {
    let mut value = 0;
    for k in 0..4 {
        let digit = bytes
            .get(at + k)
            .and_then(|&b| scan::hex_value(b))
            .unwrap_or(0);
        value = (value << 4) | digit;
    }
    value
}

/// Decodes the body of a string that [`scan_string`] accepted.
///
/// The output never needs more room than the body: every escape is at least as
/// long as the UTF-8 it stands for (`\n` is two bytes for one, `\uXXXX` six
/// for at most three, a surrogate pair twelve for four), so the one allocation
/// is sized exactly up front and never grows.
pub(crate) fn decode(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut from = 0;
    loop {
        // In a validated body the only byte `find_read_special` can stop at is
        // a backslash: quotes and control characters cannot appear unescaped.
        let at = scan::find_read_special(bytes, from, bytes.len());
        out.push_str(body.get(from..at).unwrap_or(""));
        let Some(&kind) = bytes.get(at + 1) else {
            return out;
        };
        let (ch, width) = match kind {
            b'b' => ('\u{8}', 2),
            b'f' => ('\u{c}', 2),
            b'n' => ('\n', 2),
            b'r' => ('\r', 2),
            b't' => ('\t', 2),
            b'u' => {
                let unit = hex4_valid(bytes, at + 2);
                if (0xD800..=0xDBFF).contains(&unit) {
                    let low = hex4_valid(bytes, at + 8);
                    let code =
                        0x10000 + ((unit - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                    (
                        char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER),
                        12,
                    )
                } else {
                    (
                        char::from_u32(unit).unwrap_or(char::REPLACEMENT_CHARACTER),
                        6,
                    )
                }
            }
            // `"`, `\`, and `/` stand for themselves.
            other => (char::from(other), 2),
        };
        out.push(ch);
        from = at + width;
    }
}

/// Scans the number starting at `start` (a `-` or a digit) and returns the
/// offset just past it. A number longer than `max` bytes is refused as
/// [`ErrorKind::NumberTooLong`] at `start`, whatever follows it.
pub(crate) fn scan_number(bytes: &[u8], start: usize, max: usize) -> Result<usize, Error> {
    match scan_number_text(bytes, start) {
        Ok(end) if end - start > max => err(ErrorKind::NumberTooLong, start),
        Err(e) if e.offset() - start > max => err(ErrorKind::NumberTooLong, start),
        other => other,
    }
}

/// The RFC 8259 number grammar:
/// `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`.
fn scan_number_text(bytes: &[u8], start: usize) -> Result<usize, Error> {
    let len = bytes.len();
    let digits_from = |mut i: usize| {
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        i
    };
    // One digit is required here; returns the offset past the run.
    let required_digits = |i: usize| match bytes.get(i) {
        None => err(ErrorKind::UnexpectedEnd, len),
        Some(b) if b.is_ascii_digit() => Ok(digits_from(i + 1)),
        Some(_) => err(ErrorKind::InvalidNumber, i),
    };

    let mut i = start;
    if bytes.get(i) == Some(&b'-') {
        i += 1;
    }
    i = match bytes.get(i) {
        None => return err(ErrorKind::UnexpectedEnd, len),
        Some(b'0') => {
            // A leading zero stands alone: `01` and `-00` are not numbers.
            if bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
                return err(ErrorKind::InvalidNumber, i + 1);
            }
            i + 1
        }
        Some(b'1'..=b'9') => digits_from(i + 1),
        Some(_) => return err(ErrorKind::InvalidNumber, i),
    };
    if bytes.get(i) == Some(&b'.') {
        i = required_digits(i + 1)?;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        i = required_digits(i)?;
    }
    Ok(i)
}

/// Matches the rest of the literal `word` (`true`, `false`, or `null`), whose
/// first byte is at `at` and already known to match.
pub(crate) fn scan_literal(bytes: &[u8], at: usize, word: &[u8]) -> Result<usize, Error> {
    for (k, &expected) in word.iter().enumerate().skip(1) {
        match bytes.get(at + k) {
            None => return err(ErrorKind::UnexpectedEnd, bytes.len()),
            Some(&b) if b == expected => {}
            Some(_) => return err(ErrorKind::InvalidLiteral, at + k),
        }
    }
    Ok(at + word.len())
}

/// Whether `text` is exactly one JSON number, with nothing before or after.
pub(crate) fn is_number(text: &str) -> bool {
    let bytes = text.as_bytes();
    matches!(bytes.first(), Some(b'-' | b'0'..=b'9'))
        && scan_number_text(bytes, 0).is_ok_and(|end| end == bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(src: &str) -> Result<StrToken, Error> {
        scan_string(src.as_bytes(), 0, usize::MAX)
    }

    fn kind_at(r: Result<impl core::fmt::Debug, Error>) -> (ErrorKind, usize) {
        let e = r.err().map(|e| (e.kind(), e.offset()));
        e.unwrap_or((ErrorKind::UnexpectedEnd, usize::MAX))
    }

    #[test]
    fn test_scan_string_plain_and_escaped() {
        let t = string(r#""abc" tail"#).ok();
        assert_eq!(t.map(|t| (t.end, t.escaped)), Some((5, false)));
        let t = string(r#""a\nb""#).ok();
        assert_eq!(t.map(|t| (t.end, t.escaped)), Some((6, true)));
        let t = string(r#""\ud83d\ude00""#).ok();
        assert_eq!(t.map(|t| t.end), Some(14));
    }

    #[test]
    fn test_scan_string_errors_point_at_the_offending_byte() {
        assert_eq!(kind_at(string("\"abc")), (ErrorKind::UnexpectedEnd, 4));
        assert_eq!(
            kind_at(string("\"a\u{1}\"")),
            (ErrorKind::ControlCharacter, 2)
        );
        assert_eq!(kind_at(string(r#""\x""#)), (ErrorKind::InvalidEscape, 2));
        assert_eq!(
            kind_at(string(r#""\u12g4""#)),
            (ErrorKind::InvalidUnicodeEscape, 5)
        );
        assert_eq!(kind_at(string(r#""\u12"#)), (ErrorKind::UnexpectedEnd, 5));
        assert_eq!(
            kind_at(string(r#""a\udc00""#)),
            (ErrorKind::LoneSurrogate, 2)
        );
        assert_eq!(
            kind_at(string(r#""\ud800""#)),
            (ErrorKind::LoneSurrogate, 1)
        );
        assert_eq!(
            kind_at(string(r#""\ud800\n""#)),
            (ErrorKind::LoneSurrogate, 1)
        );
        assert_eq!(
            kind_at(string(r#""\ud800\u0041""#)),
            (ErrorKind::LoneSurrogate, 1)
        );
        assert_eq!(
            kind_at(string(r#""\ud800\q""#)),
            (ErrorKind::InvalidEscape, 8)
        );
        assert_eq!(
            kind_at(string(r#""\ud800\"#)),
            (ErrorKind::UnexpectedEnd, 8)
        );
        assert_eq!(kind_at(string(r#""\ud800"#)), (ErrorKind::UnexpectedEnd, 7));
    }

    #[test]
    fn test_scan_string_budget_boundaries() {
        let b = br#""abcd""#;
        assert!(scan_string(b, 0, 4).is_ok());
        assert_eq!(kind_at(scan_string(b, 0, 3)), (ErrorKind::StringTooLong, 0));
        assert_eq!(
            kind_at(scan_string(br#""ab\n""#, 0, 3)),
            (ErrorKind::StringTooLong, 0)
        );
        assert!(scan_string(br#""ab\n""#, 0, 4).is_ok());
        assert!(scan_string(br#""""#, 0, 0).is_ok());
        // Cut short inside the budget: the input simply ended.
        assert_eq!(
            kind_at(scan_string(b"\"ab", 0, 4)),
            (ErrorKind::UnexpectedEnd, 3)
        );
    }

    #[test]
    fn test_decode_every_escape() {
        assert_eq!(
            decode(r#"a\"b\\c\/d\be\ff\ng\rh\ti"#),
            "a\"b\\c/d\u{8}e\u{c}f\ng\rh\ti"
        );
        assert_eq!(decode(r"\u0041\u00e9\u20ac"), "A\u{e9}\u{20ac}");
        assert_eq!(decode(r"x\ud83d\ude00y"), "x\u{1f600}y");
        assert_eq!(decode("plain"), "plain");
    }

    #[test]
    fn test_scan_number_grammar() {
        let num = |s: &str| scan_number(s.as_bytes(), 0, usize::MAX);
        for ok in [
            "0", "-0", "1", "-12", "1.5", "0.0", "1e5", "1E+5", "1e-05", "-0.5e10",
        ] {
            assert_eq!(num(ok).ok(), Some(ok.len()), "{ok}");
        }
        assert_eq!(kind_at(num("01")), (ErrorKind::InvalidNumber, 1));
        assert_eq!(kind_at(num("-01")), (ErrorKind::InvalidNumber, 2));
        assert_eq!(kind_at(num("-")), (ErrorKind::UnexpectedEnd, 1));
        assert_eq!(kind_at(num("-a")), (ErrorKind::InvalidNumber, 1));
        assert_eq!(kind_at(num("1.")), (ErrorKind::UnexpectedEnd, 2));
        assert_eq!(kind_at(num("1.e")), (ErrorKind::InvalidNumber, 2));
        assert_eq!(kind_at(num("1e")), (ErrorKind::UnexpectedEnd, 2));
        assert_eq!(kind_at(num("1e+")), (ErrorKind::UnexpectedEnd, 3));
        assert_eq!(kind_at(num("1e+x")), (ErrorKind::InvalidNumber, 3));
        // The number stops where the grammar does; the caller judges what follows.
        assert_eq!(num("1.5.2").ok(), Some(3));
        assert_eq!(num("12abc").ok(), Some(2));
    }

    #[test]
    fn test_scan_number_budget() {
        assert_eq!(scan_number(b"-12", 0, 3).ok(), Some(3));
        assert_eq!(
            kind_at(scan_number(b"-123", 0, 3)),
            (ErrorKind::NumberTooLong, 0)
        );
        // Too long wins over a later syntax error.
        assert_eq!(
            kind_at(scan_number(b"1234.x", 0, 3)),
            (ErrorKind::NumberTooLong, 0)
        );
        assert_eq!(
            kind_at(scan_number(b"12.x", 0, 3)),
            (ErrorKind::InvalidNumber, 3)
        );
    }

    #[test]
    fn test_scan_literal() {
        assert_eq!(scan_literal(b"true", 0, b"true").ok(), Some(4));
        assert_eq!(
            kind_at(scan_literal(b"trux", 0, b"true")),
            (ErrorKind::InvalidLiteral, 3)
        );
        assert_eq!(
            kind_at(scan_literal(b"nul", 0, b"null")),
            (ErrorKind::UnexpectedEnd, 3)
        );
    }

    #[test]
    fn test_is_number() {
        assert!(is_number("-1.5e3"));
        assert!(!is_number(""));
        assert!(!is_number("1 "));
        assert!(!is_number("+1"));
        assert!(!is_number("01"));
    }
}
