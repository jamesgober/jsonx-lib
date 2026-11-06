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

/// The lanes of `x` that may hold a [`is_read_special`] byte; the lowest
/// set one is the first that does (see the module documentation).
#[inline(always)]
const fn read_hits(x: u64) -> u64 {
    lanes_equal(x, b'"') | lanes_equal(x, b'\\') | lanes_control(x)
}

/// The index of the first byte at or after `from` in `bytes[..end]` that is
/// [`is_read_special`], or `end` if there is none.
///
/// Most strings are short (keys, identifiers), so the first word is tested
/// here, inline in the caller; a longer run continues out of line.
#[inline(always)]
pub(crate) fn find_read_special(bytes: &[u8], from: usize, end: usize) -> usize {
    let end = end.min(bytes.len());
    if from + 8 <= end {
        if let Some(chunk) = bytes.get(from..).and_then(<[u8]>::first_chunk::<8>) {
            let hits = read_hits(u64::from_le_bytes(*chunk));
            if hits != 0 {
                return from + (hits.trailing_zeros() / 8) as usize;
            }
            return find_read_special_run(bytes, from + 8, end);
        }
    }
    find_read_special_run(bytes, from, end)
}

/// [`find_read_special`] past its first word.
#[inline(never)]
fn find_read_special_run(bytes: &[u8], from: usize, end: usize) -> usize {
    let mut i = from;
    // Sixteen bytes per step while there is room: two independent words, so
    // the CPU tests both at once and branches once.
    while i + 16 <= end {
        let Some(pair) = bytes.get(i..).and_then(<[u8]>::first_chunk::<16>) else {
            break;
        };
        let (Some(lo), Some(hi)) = (pair.first_chunk::<8>(), pair.last_chunk::<8>()) else {
            break;
        };
        let lo = read_hits(u64::from_le_bytes(*lo));
        let hi = read_hits(u64::from_le_bytes(*hi));
        if lo | hi != 0 {
            return if lo != 0 {
                i + (lo.trailing_zeros() / 8) as usize
            } else {
                i + 8 + (hi.trailing_zeros() / 8) as usize
            };
        }
        i += 16;
    }
    while i + 8 <= end {
        let Some(chunk) = bytes.get(i..).and_then(<[u8]>::first_chunk::<8>) else {
            break;
        };
        let hits = read_hits(u64::from_le_bytes(*chunk));
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

/// The lanes of `x` that may hold a [`is_write_special`] byte. Zero exactly
/// when no lane does: a false positive needs a true match below it.
#[inline(always)]
const fn write_hits(x: u64) -> u64 {
    read_hits(x) | lanes_equal(x, 0xE2)
}

/// How many leading bytes of `bytes` are known not to be
/// [`is_write_special`]: `bytes.len()` exactly when none is, so the string
/// can be copied between quotes as it is; otherwise at most the index of the
/// first one, where the escaping scan can start.
///
/// Whole words, with no byte loop. Up to 16 bytes take one test: two
/// overlapping words (or halves of one, from four bytes; from one to three,
/// the first, middle and last byte padded with spaces). So a key or a short
/// value costs one test, and for a literal key, inlined into the caller, the
/// test folds away at compile time. Longer strings go 16 bytes per step out
/// of line.
#[inline(always)]
pub(crate) fn plain_prefix_for_write(bytes: &[u8]) -> usize {
    let n = bytes.len();
    if n > 16 {
        return plain_prefix_run(bytes);
    }
    let hits = if let (Some(lo), Some(hi)) = (bytes.first_chunk::<8>(), bytes.last_chunk::<8>()) {
        write_hits(u64::from_le_bytes(*lo)) | write_hits(u64::from_le_bytes(*hi))
    } else if let (Some(lo), Some(hi)) = (bytes.first_chunk::<4>(), bytes.last_chunk::<4>()) {
        write_hits(u64::from(u32::from_le_bytes(*lo)) | (u64::from(u32::from_le_bytes(*hi)) << 32))
    } else if let (Some(&a), Some(&b), Some(&c)) = (bytes.first(), bytes.get(n / 2), bytes.last()) {
        write_hits(u64::from_le_bytes([a, b, c, b' ', b' ', b' ', b' ', b' ']))
    } else {
        0
    };
    if hits == 0 { n } else { 0 }
}

/// [`plain_prefix_for_write`] past 16 bytes: two words per step, one branch,
/// then the last 16 bytes (overlapping what was tested).
#[inline(never)]
fn plain_prefix_run(bytes: &[u8]) -> usize {
    let n = bytes.len();
    let block_hits = |block: &[u8; 16]| match (block.first_chunk::<8>(), block.last_chunk::<8>()) {
        (Some(lo), Some(hi)) => {
            write_hits(u64::from_le_bytes(*lo)) | write_hits(u64::from_le_bytes(*hi))
        }
        _ => 1,
    };
    let mut i = 0;
    while i + 16 <= n {
        match bytes.get(i..).and_then(<[u8]>::first_chunk::<16>) {
            Some(block) if block_hits(block) == 0 => i += 16,
            _ => return i,
        }
    }
    match bytes.last_chunk::<16>() {
        Some(block) if block_hits(block) == 0 => n,
        _ => i,
    }
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

/// High bit set in each lane of `x` exactly equal to `byte`: no false
/// positives, unlike [`lanes_equal`], at the cost of two more operations.
/// `(v & 0x7F) + 0x7F` sets a lane's high bit exactly when its low seven
/// bits are not all zero, and cannot carry into the next lane; OR-ing `v`
/// adds the lane's own high bit. So the high bit stays clear exactly in the
/// zero lanes.
#[inline(always)]
const fn lanes_exactly(x: u64, byte: u8) -> u64 {
    let v = x ^ (ONES * byte as u64);
    !(((v & !HIGHS).wrapping_add(!HIGHS)) | v) & HIGHS
}

/// The index of the first byte at or after `from` that is not JSON
/// whitespace, or `bytes.len()` if there is none.
///
/// Compact JSON has no whitespace or a single space between tokens, so the
/// first two bytes are checked one at a time; a longer run (indentation in
/// pretty-printed JSON) is skipped eight bytes at a time.
#[inline(always)]
pub(crate) fn skip_whitespace(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    for _ in 0..2 {
        match bytes.get(i) {
            Some(&b) if is_whitespace(b) => i += 1,
            _ => return i,
        }
    }
    skip_whitespace_run(bytes, i)
}

/// [`skip_whitespace`] past its first two bytes, out of line.
#[inline(never)]
fn skip_whitespace_run(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while let Some(chunk) = bytes.get(i..).and_then(<[u8]>::first_chunk::<8>) {
        let x = u64::from_le_bytes(*chunk);
        let blank = lanes_exactly(x, b' ')
            | lanes_exactly(x, b'\n')
            | lanes_exactly(x, b'\t')
            | lanes_exactly(x, b'\r');
        let other = !blank & HIGHS;
        if other != 0 {
            // `trailing_zeros / 8` is below 8.
            return i + (other.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    while bytes.get(i).copied().is_some_and(is_whitespace) {
        i += 1;
    }
    i
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
    fn test_plain_prefix_every_length_and_position() {
        for len in 0..70 {
            let plain = alloc::vec![b'a'; len];
            assert_eq!(plain_prefix_for_write(&plain), len, "{len}");
            for at in 0..len {
                for special in [b'"', b'\\', 0, 0x1f, 0xE2] {
                    let mut bytes = plain.clone();
                    if let Some(slot) = bytes.get_mut(at) {
                        *slot = special;
                    }
                    let prefix = plain_prefix_for_write(&bytes);
                    assert!(prefix <= at, "{len} {at} {special}: {prefix}");
                }
                // Bytes next to the special ones in value are plain.
                for other in [b' ', b'#', b'[', b']', 0x7f, 0x80, 0xE1, 0xE3, 0xFF] {
                    let mut bytes = plain.clone();
                    if let Some(slot) = bytes.get_mut(at) {
                        *slot = other;
                    }
                    assert_eq!(plain_prefix_for_write(&bytes), len, "{len} {at} {other}");
                }
            }
        }
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

    fn naive_skip(bytes: &[u8], from: usize) -> usize {
        (from..bytes.len())
            .find(|&i| !is_whitespace(bytes[i]))
            .unwrap_or(bytes.len())
    }

    #[test]
    fn test_skip_whitespace_each_lane_and_run_length() {
        for run in 0..40 {
            for stop in [b'x', 0x00, 0x0B, 0x0C, 0x1F, 0x21, 0x8A, 0xA0, 0xFF] {
                let mut bytes = Vec::new();
                for k in 0..run {
                    bytes.push(b" \n\t\r"[k % 4]);
                }
                bytes.push(stop);
                bytes.extend_from_slice(b"   ");
                assert_eq!(skip_whitespace(&bytes, 0), run, "run {run} stop {stop:#x}");
            }
        }
        assert_eq!(skip_whitespace(b"        ", 0), 8);
        assert_eq!(skip_whitespace(b"", 0), 0);
        assert_eq!(skip_whitespace(b"ab  ", 2), 4);
    }

    proptest! {
        #[test]
        fn prop_skip_whitespace_matches_naive(bytes in proptest::collection::vec(prop_oneof![Just(b' '), Just(b'\n'), Just(b'\t'), Just(b'\r'), any::<u8>()], 0..80), from in 0usize..80) {
            let from = from.min(bytes.len());
            prop_assert_eq!(skip_whitespace(&bytes, from), naive_skip(&bytes, from));
        }

        #[test]
        fn prop_swar_scans_match_naive(bytes in proptest::collection::vec(any::<u8>(), 0..64), from in 0usize..64, end in 0usize..70) {
            let from = from.min(bytes.len());
            let end = end.max(from);
            prop_assert_eq!(find_read_special(&bytes, from, end), naive_read(&bytes, from, end));
            prop_assert_eq!(find_write_special(&bytes, from), naive_write(&bytes, from));
            let first = naive_write(&bytes, 0);
            let prefix = plain_prefix_for_write(&bytes);
            prop_assert!(prefix <= first);
            prop_assert_eq!(prefix == bytes.len(), first == bytes.len());
        }
    }
}
