//! Byte-level scanning shared by the reader and the writer.
//!
//! String bodies are scanned eight bytes at a time with SWAR ("SIMD within a
//! register") arithmetic on a `u64`: each test below sets the high bit of
//! every byte lane that matches, and the lowest set bit marks the first match.
//! The classic zero-byte trick can also flag a lane *above* a true match (the
//! subtraction borrows into it), but never one below, so the lowest flagged
//! lane is always a real match. OR-ing several such masks keeps that property:
//! a false positive in one mask needs a true match below it, which would be
//! the lower bit. The scalar tail then handles the last few bytes.

/// `0x01` in every byte lane.
const ONES: u64 = 0x0101_0101_0101_0101;
/// `0x80` in every byte lane.
const HIGHS: u64 = 0x8080_8080_8080_8080;

/// High bit set in each lane of `x` that is zero (plus possible false
/// positives above a true one).
#[inline(always)]
const fn zero_lanes(x: u64) -> u64 {
    x.wrapping_sub(ONES) & !x & HIGHS
}

/// High bit set in each lane of `x` equal to `byte`.
#[inline(always)]
const fn lanes_equal(x: u64, byte: u8) -> u64 {
    zero_lanes(x ^ (ONES * byte as u64))
}

/// High bit set in each lane of `x` below `0x20` (a control character).
#[inline(always)]
const fn lanes_control(x: u64) -> u64 {
    x.wrapping_sub(ONES * 0x20) & !x & HIGHS
}

/// Whether a string body byte ends the fast path when reading: a quote, a
/// backslash, or a control character.
#[inline(always)]
pub(crate) const fn is_read_special(b: u8) -> bool {
    b == b'"' || b == b'\\' || b < 0x20
}

/// Whether a byte might need escaping when writing: a quote, a backslash, a
/// control character, or `0xE2`, the lead byte of U+2028 and U+2029.
#[inline(always)]
pub(crate) const fn is_write_special(b: u8) -> bool {
    is_read_special(b) || b == 0xE2
}

/// The index of the first byte at or after `from` in `bytes[..end]` that is
/// [`is_read_special`], or `end` if there is none.
#[inline]
pub(crate) fn find_read_special(bytes: &[u8], from: usize, end: usize) -> usize {
    let end = end.min(bytes.len());
    let mut i = from;
    while i + 8 <= end {
        let Some(chunk) = bytes.get(i..).and_then(<[u8]>::first_chunk::<8>) else {
            break;
        };
        let x = u64::from_le_bytes(*chunk);
        let hits = lanes_equal(x, b'"') | lanes_equal(x, b'\\') | lanes_control(x);
        if hits != 0 {
            // `trailing_zeros / 8` is below 8.
            return i + (hits.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    while i < end {
        match bytes.get(i) {
            Some(&b) if is_read_special(b) => return i,
            Some(_) => i += 1,
            None => break,
        }
    }
    end
}

/// The index of the first byte at or after `from` that is
/// [`is_write_special`], or `bytes.len()` if there is none.
#[inline]
pub(crate) fn find_write_special(bytes: &[u8], from: usize) -> usize {
    let end = bytes.len();
    let mut i = from;
    while i + 8 <= end {
        let Some(chunk) = bytes.get(i..).and_then(<[u8]>::first_chunk::<8>) else {
            break;
        };
        let x = u64::from_le_bytes(*chunk);
        let hits =
            lanes_equal(x, b'"') | lanes_equal(x, b'\\') | lanes_control(x) | lanes_equal(x, 0xE2);
        if hits != 0 {
            return i + (hits.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    while let Some(&b) = bytes.get(i) {
        if is_write_special(b) {
            return i;
        }
        i += 1;
    }
    end
}

/// JSON's four whitespace bytes (RFC 8259 §2). Nothing else is whitespace:
/// not form feed, not vertical tab, not U+00A0, not a byte-order mark.
#[inline(always)]
pub(crate) const fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// The value of an ASCII hexadecimal digit.
#[inline(always)]
pub(crate) const fn hex_value(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u32),
        b'a'..=b'f' => Some((b - b'a' + 10) as u32),
        b'A'..=b'F' => Some((b - b'A' + 10) as u32),
        _ => None,
    }
}

/// The lowercase hexadecimal digit for the low four bits of `nibble`.
#[inline(always)]
pub(crate) const fn hex_digit(nibble: u8) -> u8 {
    let n = nibble & 0x0F;
    if n < 10 { b'0' + n } else { b'a' + n - 10 }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use proptest::prelude::*;

    use super::*;

    fn naive_read(bytes: &[u8], from: usize, end: usize) -> usize {
        let end = end.min(bytes.len());
        (from..end)
            .find(|&i| is_read_special(bytes[i]))
            .unwrap_or(end)
    }

    fn naive_write(bytes: &[u8], from: usize) -> usize {
        (from..bytes.len())
            .find(|&i| is_write_special(bytes[i]))
            .unwrap_or(bytes.len())
    }

    #[test]
    fn test_find_read_special_each_lane() {
        for lane in 0..16 {
            for special in [b'"', b'\\', 0x00, 0x1F, b'\n'] {
                let mut bytes = Vec::from([b'a'; 16]);
                bytes[lane] = special;
                assert_eq!(find_read_special(&bytes, 0, 16), lane);
            }
        }
        assert_eq!(find_read_special(b"abcdefghijklmnop", 0, 16), 16);
        assert_eq!(find_read_special(b"abc\"", 0, 3), 3);
        assert_eq!(find_read_special(b"", 0, 0), 0);
    }

    #[test]
    fn test_find_write_special_finds_e2_and_ignores_high_bytes() {
        let s = "abcdefgh\u{2028}";
        assert_eq!(find_write_special(s.as_bytes(), 0), 8);
        let s = "ÿÿÿÿÿÿÿÿ"; // 0xC3 0xBF pairs: high bytes, nothing special
        assert_eq!(find_write_special(s.as_bytes(), 0), s.len());
        assert_eq!(find_write_special(b" \x7f", 0), 2);
    }

    #[test]
    fn test_hex_helpers() {
        assert_eq!(hex_value(b'0'), Some(0));
        assert_eq!(hex_value(b'f'), Some(15));
        assert_eq!(hex_value(b'F'), Some(15));
        assert_eq!(hex_value(b'g'), None);
        assert_eq!(hex_digit(0xA), b'a');
        assert_eq!(hex_digit(0x3F), b'f');
    }

    proptest! {
        #[test]
        fn prop_swar_scans_match_naive(bytes in proptest::collection::vec(any::<u8>(), 0..64), from in 0usize..64, end in 0usize..70) {
            let from = from.min(bytes.len());
            let end = end.max(from);
            prop_assert_eq!(find_read_special(&bytes, from, end), naive_read(&bytes, from, end));
            prop_assert_eq!(find_write_special(&bytes, from), naive_write(&bytes, from));
        }
    }
}
