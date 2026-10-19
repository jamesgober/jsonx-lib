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

/// Where a string scan stopped, so it can continue when more input arrives:
/// every byte before `next` has been checked, and `next` is not inside an
/// escape sequence.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StrScan {
    /// The first byte not yet checked.
    pub(crate) next: usize,
    /// Whether an escape sequence was seen before `next`.
    pub(crate) escaped: bool,
}

impl StrScan {
    /// A scan of the string whose opening quote is at `quote`, from its
    /// first body byte.
    #[inline]
    pub(crate) const fn start(quote: usize) -> Self {
        Self {
            next: quote + 1,
            escaped: false,
        }
    }
}

/// Scans the string whose opening quote is at `quote`, validating every escape
/// and refusing a body longer than `max` source bytes.
///
/// Nothing is decoded or allocated here: a string with escapes is validated in
/// this pass and decoded by [`decode`] in a second, so the decoder can size its
/// buffer exactly, and only after the length budget has passed.
#[cfg(test)]
pub(crate) fn scan_string(bytes: &[u8], quote: usize, max: usize) -> Result<StrToken, Error> {
    resume_string(bytes, quote, &mut StrScan::start(quote), max)
}

/// [`scan_string`] from where `scan` stopped. When the input runs out
/// ([`ErrorKind::UnexpectedEnd`]), `scan` records how far the string was
/// checked, so a later call over a longer input picks up there: a string fed
/// in pieces is scanned once, plus at most the eleven bytes of an escape cut
/// in half.
pub(crate) fn resume_string(
    bytes: &[u8],
    quote: usize,
    scan: &mut StrScan,
    max: usize,
) -> Result<StrToken, Error> {
    let len = bytes.len();
    let body = quote + 1;
    // The closing quote may sit at `body + max` at the latest; nothing past
    // that needs looking at before the budget refuses the string.
    let cap = body.saturating_add(max).saturating_add(1).min(len);
    let mut i = scan.next;
    loop {
        i = scan::find_read_special(bytes, i, cap);
        scan.next = i;
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
                    escaped: scan.escaped,
                });
            }
            b'\\' => {
                i = scan_escape(bytes, i)?;
                scan.next = i;
                scan.escaped = true;
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

/// Where a number scan stopped: the first byte not yet looked at, and the
/// grammar state reached before it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NumScan {
    /// The first byte not yet checked.
    next: usize,
    state: NumState,
}

impl NumScan {
    /// A scan of the number whose first byte is at `start`.
    #[inline]
    pub(crate) const fn start(start: usize) -> Self {
        Self {
            next: start,
            state: NumState::Start,
        }
    }
}

/// The states of the RFC 8259 number grammar,
/// `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumState {
    /// Nothing read yet.
    Start,
    /// After `-`: a digit must follow.
    Minus,
    /// A leading `0`: a whole number, but no digit may follow it.
    Zero,
    /// In the integer digits.
    Int,
    /// After `.`: a digit must follow.
    Dot,
    /// In the fraction digits.
    Frac,
    /// After `e` or `E`: a sign or a digit must follow.
    Exp,
    /// After the exponent's sign: a digit must follow.
    ExpSign,
    /// In the exponent digits.
    ExpDigits,
}

impl NumState {
    /// Whether the text so far is a whole number.
    #[inline]
    const fn is_complete(self) -> bool {
        matches!(self, Self::Zero | Self::Int | Self::Frac | Self::ExpDigits)
    }
}

/// Scans the number starting at `start` (a `-` or a digit) and returns the
/// offset just past it. A number longer than `max` bytes is refused as
/// [`ErrorKind::NumberTooLong`] at `start`, whatever follows it.
#[cfg(test)]
pub(crate) fn scan_number(bytes: &[u8], start: usize, max: usize) -> Result<usize, Error> {
    resume_number(bytes, start, &mut NumScan::start(start), true, max)
}

/// [`scan_number`] from where `scan` stopped. With `complete`, the end of
/// `bytes` is the end of the input, so a number that reaches it ends there.
/// Without it, a number that reaches the end may still go on: the scan stops
/// with [`ErrorKind::UnexpectedEnd`] and `scan` set to continue, so a number
/// fed in pieces is scanned once.
pub(crate) fn resume_number(
    bytes: &[u8],
    start: usize,
    scan: &mut NumScan,
    complete: bool,
    max: usize,
) -> Result<usize, Error> {
    match number_text(bytes, scan, complete) {
        Ok(end) if end - start > max => err(ErrorKind::NumberTooLong, start),
        Err(e) if e.offset() - start > max => err(ErrorKind::NumberTooLong, start),
        other => other,
    }
}

/// The number grammar as a resumable state machine; see [`NumState`].
fn number_text(bytes: &[u8], scan: &mut NumScan, complete: bool) -> Result<usize, Error> {
    let mut i = scan.next;
    let mut state = scan.state;
    loop {
        let Some(&b) = bytes.get(i) else {
            scan.next = i;
            scan.state = state;
            return if complete && state.is_complete() {
                Ok(i)
            } else {
                err(ErrorKind::UnexpectedEnd, bytes.len())
            };
        };
        state = match (state, b) {
            (NumState::Start, b'-') => NumState::Minus,
            (NumState::Start | NumState::Minus, b'0') => NumState::Zero,
            (NumState::Start | NumState::Minus, b'1'..=b'9') => NumState::Int,
            // A leading zero stands alone: `01` and `-00` are not numbers.
            (NumState::Zero, b'0'..=b'9') => return err(ErrorKind::InvalidNumber, i),
            (NumState::Int | NumState::Frac | NumState::ExpDigits, b'0'..=b'9') => {
                // The common case: a run of digits.
                i += 1;
                while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
                continue;
            }
            (NumState::Zero | NumState::Int, b'.') => NumState::Dot,
            (NumState::Dot, b'0'..=b'9') => NumState::Frac,
            (NumState::Zero | NumState::Int | NumState::Frac, b'e' | b'E') => NumState::Exp,
            (NumState::Exp, b'+' | b'-') => NumState::ExpSign,
            (NumState::Exp | NumState::ExpSign, b'0'..=b'9') => NumState::ExpDigits,
            // The number stops where the grammar does; the caller judges
            // what follows.
            (complete_so_far, _) if complete_so_far.is_complete() => return Ok(i),
            _ => return err(ErrorKind::InvalidNumber, i),
        };
        i += 1;
    }
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
        && number_text(bytes, &mut NumScan::start(0), true).is_ok_and(|end| end == bytes.len())
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
    fn test_resume_string_at_every_split_matches_one_scan() {
        let src = br#""ab\n\u00e9\ud83d\ude00cd" tail"#;
        let whole = scan_string(src, 0, usize::MAX)
            .ok()
            .map(|t| (t.end, t.escaped));
        assert_eq!(whole, Some((26, true)));
        for cut in 1..src.len() {
            let mut scan = StrScan::start(0);
            let head = src.get(..cut).unwrap_or_default();
            match resume_string(head, 0, &mut scan, usize::MAX) {
                Ok(t) => assert_eq!(Some((t.end, t.escaped)), whole),
                Err(e) => {
                    assert_eq!((e.kind(), e.offset()), (ErrorKind::UnexpectedEnd, cut));
                    // The scan never stops inside an escape, and never past the cut.
                    assert!(scan.next <= cut);
                    assert_ne!(src.get(scan.next.saturating_sub(1)), Some(&b'\\'));
                    let rest = resume_string(src, 0, &mut scan, usize::MAX);
                    assert_eq!(rest.ok().map(|t| (t.end, t.escaped)), whole, "cut {cut}");
                }
            }
        }
    }

    #[test]
    fn test_resume_string_keeps_budget_and_errors() {
        let mut scan = StrScan::start(0);
        assert!(resume_string(b"\"abc", 0, &mut scan, 3).is_err());
        assert_eq!(
            kind_at(resume_string(b"\"abcd\"", 0, &mut scan, 3)),
            (ErrorKind::StringTooLong, 0)
        );
        let mut scan = StrScan::start(0);
        assert!(resume_string(b"\"a\\", 0, &mut scan, 9).is_err());
        assert_eq!(
            kind_at(resume_string(b"\"a\\q\"", 0, &mut scan, 9)),
            (ErrorKind::InvalidEscape, 3)
        );
    }

    #[test]
    fn test_resume_number_at_every_split_matches_one_scan() {
        for text in ["-12.5e+30,", "0,", "-0.0e0]", "7 ", "123456789}", "1E-2,"] {
            let src = text.as_bytes();
            let whole = scan_number(src, 0, usize::MAX).ok();
            for cut in 1..src.len() {
                let mut scan = NumScan::start(0);
                let head = src.get(..cut).unwrap_or_default();
                match resume_number(head, 0, &mut scan, false, usize::MAX) {
                    Ok(end) => assert_eq!(Some(end), whole, "{text} cut {cut}"),
                    Err(e) => {
                        assert_eq!((e.kind(), e.offset()), (ErrorKind::UnexpectedEnd, cut));
                        let rest = resume_number(src, 0, &mut scan, false, usize::MAX);
                        assert_eq!(rest.ok(), whole, "{text} cut {cut}");
                    }
                }
            }
            // Complete input: a number at the very end ends there.
            let bare = src.get(..src.len() - 1).unwrap_or_default();
            assert_eq!(scan_number(bare, 0, usize::MAX).ok(), whole);
        }
        // Partial input: an invalid byte is still refused at once.
        let mut scan = NumScan::start(0);
        assert_eq!(
            kind_at(resume_number(b"01", 0, &mut scan, false, 9)),
            (ErrorKind::InvalidNumber, 1)
        );
        // And the budget is enforced on the part seen so far.
        let mut scan = NumScan::start(0);
        assert_eq!(
            kind_at(resume_number(b"1234", 0, &mut scan, false, 3)),
            (ErrorKind::NumberTooLong, 0)
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
