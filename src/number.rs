//! JSON numbers, kept as their exact text.

use alloc::{borrow::Cow, string::String};
use core::fmt::{self, Write as _};

use crate::lexer;

/// A JSON number, kept as the exact text of the document.
///
/// Nothing is converted while parsing: the number borrows its digits from the
/// input, so a 30-digit integer or a decimal with more precision than `f64`
/// survives a read and a write unchanged. Conversion to a Rust number happens
/// only when it is asked for, with [`as_i64`](Self::as_i64),
/// [`as_u64`](Self::as_u64), or [`as_f64`](Self::as_f64), each of which says
/// exactly when it succeeds.
///
/// Two numbers are equal when their text is: `1`, `1.0`, and `1e0` are three
/// different numbers here, as they are three different documents. Compare
/// [`as_f64`](Self::as_f64) results to compare values.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{parse, Number};
///
/// let doc = parse("[12345678901234567890123, 2.50, -7]")?;
/// let big = doc[0].as_number().unwrap();
/// assert_eq!(big.as_str(), "12345678901234567890123"); // exact
/// assert_eq!(big.as_u64(), None); // does not fit
/// assert_eq!(doc[1].as_number().unwrap().as_str(), "2.50"); // trailing zero kept
/// assert_eq!(doc[2].as_i64(), Some(-7));
///
/// assert_eq!(Number::from(42u8).as_str(), "42");
/// assert_eq!(Number::from_f64(0.1).unwrap().as_str(), "0.1");
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Number<'a> {
    /// Always one complete RFC 8259 number.
    text: Cow<'a, str>,
}

impl<'a> Number<'a> {
    /// Wraps text the tokenizer has already checked.
    #[inline]
    pub(crate) const fn from_valid(text: &'a str) -> Self {
        Self {
            text: Cow::Borrowed(text),
        }
    }

    /// Makes a number from its JSON text, which must be exactly one number in
    /// RFC 8259 syntax with nothing around it. The text is borrowed, not
    /// copied, so arbitrary-precision values pass through unchanged.
    ///
    /// Returns `None` for anything else: `+1`, `01`, `.5`, `1.`, `NaN`,
    /// whitespace, or an empty string.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{to_string, Number, Value};
    ///
    /// let price = Number::from_text("19.990").unwrap();
    /// assert_eq!(to_string(&Value::Number(price)), "19.990");
    /// assert!(Number::from_text("1.").is_none());
    /// assert!(Number::from_text("0x10").is_none());
    /// ```
    #[must_use]
    pub fn from_text(text: &'a str) -> Option<Self> {
        lexer::is_number(text).then_some(Self::from_valid(text))
    }

    /// Makes a number from an `f64`, written as the shortest text that reads
    /// back as the same `f64`. Returns `None` for NaN and the infinities,
    /// which JSON cannot represent.
    ///
    /// The text has a fraction or an exponent, so it reads back as a float:
    /// `1.0` is written `1.0`, not `1`. Values from 1e-5 up to below 1e16 are
    /// written as decimals (`0.00001`, `123.456`, `1000000000000000.0`); the
    /// rest in exponent form (`1e16`, `1.5e-7`, `-2.2250738585072014e-308`).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Number;
    ///
    /// assert_eq!(Number::from_f64(1.0).unwrap().as_str(), "1.0");
    /// assert_eq!(Number::from_f64(-0.0).unwrap().as_str(), "-0.0");
    /// assert_eq!(Number::from_f64(1e100).unwrap().as_str(), "1e100");
    /// assert_eq!(Number::from_f64(0.1 + 0.2).unwrap().as_str(), "0.30000000000000004");
    /// assert!(Number::from_f64(f64::NAN).is_none());
    /// ```
    #[must_use]
    pub fn from_f64(value: f64) -> Option<Number<'static>> {
        format_f64(value).map(|text| Number {
            text: Cow::Owned(text),
        })
    }

    /// The number's text, exactly as written in the document (or as this crate
    /// wrote it, for a number made from a Rust value).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::parse;
    ///
    /// assert_eq!(parse("1E+02")?.as_number().unwrap().as_str(), "1E+02");
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether the text is an integer: no fraction and no exponent. `2.0` and
    /// `2e3` are not integers by this test, though their values are whole.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Number;
    ///
    /// assert!(Number::from(-3).is_integer());
    /// assert!(!Number::from_text("2.0").unwrap().is_integer());
    /// ```
    #[must_use]
    pub fn is_integer(&self) -> bool {
        !self.text.bytes().any(|b| matches!(b, b'.' | b'e' | b'E'))
    }

    /// The number as an `i64`, if its text is an integer (see
    /// [`is_integer`](Self::is_integer)) in `i64`'s range. `None` otherwise:
    /// nothing is rounded or truncated.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Number;
    ///
    /// let n = |s| Number::from_text(s).unwrap();
    /// assert_eq!(n("-9223372036854775808").as_i64(), Some(i64::MIN));
    /// assert_eq!(n("9223372036854775808").as_i64(), None); // out of range
    /// assert_eq!(n("3.0").as_i64(), None); // not an integer's text
    /// ```
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        if self.is_integer() {
            self.text.parse().ok()
        } else {
            None
        }
    }

    /// The number as a `u64`, if its text is an integer (see
    /// [`is_integer`](Self::is_integer)) in `u64`'s range. `-0` reads as `0`.
    /// `None` otherwise: nothing is rounded or truncated.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Number;
    ///
    /// let n = |s| Number::from_text(s).unwrap();
    /// assert_eq!(n("18446744073709551615").as_u64(), Some(u64::MAX));
    /// assert_eq!(n("-1").as_u64(), None);
    /// assert_eq!(n("-0").as_u64(), Some(0));
    /// ```
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        if !self.is_integer() {
            return None;
        }
        match self.text.strip_prefix('-') {
            // The grammar forbids leading zeros, so `-0` is the only negative
            // integer text with a value `u64` can hold.
            Some("0") => Some(0),
            Some(_) => None,
            None => self.text.parse().ok(),
        }
    }

    /// The number as the nearest `f64` (correctly rounded), or `None` if it is
    /// too large in magnitude to be finite (`1e400`). A value too small to
    /// represent rounds to zero, as IEEE 754 does. Integers beyond 2^53 lose
    /// precision; use [`as_i64`](Self::as_i64), [`as_u64`](Self::as_u64), or
    /// the exact [`as_str`](Self::as_str) when that matters.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Number;
    ///
    /// let n = |s| Number::from_text(s).unwrap();
    /// assert_eq!(n("2.5e-3").as_f64(), Some(0.0025));
    /// assert_eq!(n("7").as_f64(), Some(7.0));
    /// assert_eq!(n("1e400").as_f64(), None);
    /// ```
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        self.text.parse::<f64>().ok().filter(|x| x.is_finite())
    }

    /// Copies borrowed text so the number no longer borrows from the input.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Number;
    ///
    /// let owned: Number<'static> = {
    ///     let text = String::from("12.5");
    ///     Number::from_text(&text).unwrap().into_owned()
    /// };
    /// assert_eq!(owned.as_str(), "12.5");
    /// ```
    #[must_use]
    pub fn into_owned(self) -> Number<'static> {
        Number {
            text: Cow::Owned(self.text.into_owned()),
        }
    }
}

macro_rules! from_integer {
    ($($t:ty),*) => {$(
        impl From<$t> for Number<'static> {
            /// Writes the integer in decimal.
            #[inline]
            fn from(value: $t) -> Self {
                let mut text = String::new();
                // Writing to a `String` cannot fail.
                let _infallible = write!(text, "{value}");
                Self { text: Cow::Owned(text) }
            }
        }
    )*};
}

from_integer!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

impl fmt::Display for Number<'_> {
    /// Writes the number's exact text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Debug for Number<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// A fixed buffer for `core::fmt` output, so formatting an `f64` needs no
/// allocation before the final text is built.
struct StackText {
    buf: [u8; 40],
    len: usize,
}

impl fmt::Write for StackText {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        let slot = self.buf.get_mut(self.len..end).ok_or(fmt::Error)?;
        slot.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// Pushes `count` ASCII zeros.
fn zeros(out: &mut String, count: usize) {
    out.extend(core::iter::repeat_n('0', count));
}

/// The shortest round-trip text for a finite `f64`, laid out as described on
/// [`Number::from_f64`].
///
/// `core`'s `{:e}` formatting already yields the shortest digit string that
/// reads back as the same `f64` (it is the same algorithm `Display` uses); this
/// only moves the decimal point.
fn format_f64(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    let mut sci = StackText {
        buf: [0; 40],
        len: 0,
    };
    write!(sci, "{value:e}").ok()?;
    let sci = core::str::from_utf8(sci.buf.get(..sci.len)?).ok()?;
    let (negative, sci) = match sci.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, sci),
    };
    let (mantissa, exponent) = sci.split_once('e')?;
    let exponent: i32 = exponent.parse().ok()?;
    // `{:e}` writes one digit before the point.
    let (lead, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));

    let mut out = String::with_capacity(26);
    if negative {
        out.push('-');
    }
    if (-5..16).contains(&exponent) {
        if let Ok(point) = usize::try_from(exponent) {
            // `point` digits of `frac` go before the decimal point.
            out.push_str(lead);
            if frac.len() <= point {
                out.push_str(frac);
                zeros(&mut out, point - frac.len());
                out.push_str(".0");
            } else {
                out.push_str(frac.get(..point)?);
                out.push('.');
                out.push_str(frac.get(point..)?);
            }
        } else {
            out.push_str("0.");
            zeros(&mut out, exponent.unsigned_abs() as usize - 1);
            out.push_str(lead);
            out.push_str(frac);
        }
    } else {
        out.push_str(lead);
        if !frac.is_empty() {
            out.push('.');
            out.push_str(frac);
        }
        write!(out, "e{exponent}").ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use proptest::prelude::*;

    use super::*;

    fn f(x: f64) -> String {
        format_f64(x).unwrap_or_default()
    }

    #[test]
    fn test_format_f64_layouts() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (1.5, "1.5"),
            (100.0, "100.0"),
            (123.456, "123.456"),
            (0.1, "0.1"),
            (1e-5, "0.00001"),
            (1.5e-5, "0.000015"),
            (1e-6, "1e-6"),
            (1.5e-7, "1.5e-7"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e16"),
            (1.7976931348623157e308, "1.7976931348623157e308"),
            (5e-324, "5e-324"),
            (-2.2250738585072014e-308, "-2.2250738585072014e-308"),
            (123456789012345.67, "123456789012345.67"),
        ];
        for &(x, want) in cases {
            assert_eq!(f(x), want, "{x:e}");
        }
        assert!(format_f64(f64::NAN).is_none());
        assert!(format_f64(f64::INFINITY).is_none());
        assert!(format_f64(f64::NEG_INFINITY).is_none());
    }

    #[test]
    fn test_integer_conversions_exact() {
        assert_eq!(Number::from(i64::MIN).as_i64(), Some(i64::MIN));
        assert_eq!(Number::from(u64::MAX).as_u64(), Some(u64::MAX));
        assert_eq!(Number::from(u64::MAX).as_i64(), None);
        assert_eq!(Number::from(-1i8).as_u64(), None);
        assert_eq!(
            Number::from(u128::MAX).as_str(),
            "340282366920938463463374607431768211455"
        );
    }

    proptest! {
        #[test]
        fn prop_f64_text_is_valid_and_round_trips(bits in any::<u64>()) {
            let x = f64::from_bits(bits);
            prop_assume!(x.is_finite());
            let n = Number::from_f64(x).unwrap_or_else(|| Number::from(0));
            prop_assert!(lexer::is_number(n.as_str()), "{}", n.as_str());
            prop_assert!(!n.is_integer());
            let back = n.as_f64().unwrap_or(f64::NAN);
            prop_assert_eq!(back.to_bits(), x.to_bits());
            // Shortest: no representation with fewer significant digits exists,
            // which `{:e}` guarantees; check the text carries exactly its digits.
            let digits: Vec<u8> = alloc::format!("{x:e}").bytes().take_while(|&b| b != b'e').filter(u8::is_ascii_digit).collect();
            let ours: Vec<u8> = n.as_str().bytes().take_while(|&b| b != b'e' && b != b'E').filter(u8::is_ascii_digit).collect();
            let trim = |d: &[u8]| -> Vec<u8> {
                let s: &[u8] = d;
                let start = s.iter().position(|&b| b != b'0').unwrap_or(s.len());
                let end = s.iter().rposition(|&b| b != b'0').map_or(start, |e| e + 1);
                s.get(start..end.max(start)).unwrap_or(&[]).to_vec()
            };
            prop_assert_eq!(trim(&digits), trim(&ours));
        }

        #[test]
        fn prop_i64_round_trips(x in any::<i64>()) {
            prop_assert_eq!(Number::from(x).as_i64(), Some(x));
        }

        #[test]
        fn prop_u64_round_trips(x in any::<u64>()) {
            prop_assert_eq!(Number::from(x).as_u64(), Some(x));
        }
    }
}
