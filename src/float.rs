//! Shortest round-trip text for `f64` and `f32`, in integer arithmetic.
//!
//! The digits come from Raffaello Giulietti's Schubfach algorithm ("The
//! Schubfach way to render doubles", 2020; Java's `Double.toString` uses it
//! since JDK 19), written here from the paper and in its notation: three
//! multiplications by a 126-bit power of ten from one table, rounded to
//! odd, place the float's rounding interval on a decimal grid, and at most
//! four candidates are compared. The result is the decimal with the fewest
//! digits that reads back as the same float and, of those, the one closest
//! to it: the digits of `core`'s `{:e}`. The tests compare the two on about
//! a million values and every exponent and boundary of both types, and on
//! demand on every `f32` and a billion `f64`s.
//!
//! Two details differ from Java's use of it. Java keeps at least two digits
//! (`4.9E-324`); this keeps the shortest (`5e-324`), as `core` does, so the
//! shorter candidate is tried from two digits up and the smallest
//! subnormals are not scaled by ten first. And on an exact tie, where two
//! shortest decimals are equally close to the float, `core` takes the
//! higher one; that is the default here, and [`Ties::Even`] takes the one
//! whose last digit is even, as Java, JavaScript, Python and `serde_json`
//! do. Ties need 16 or 17 significant digits for an `f64` (8 for an
//! `f32`), so they are rare.

use alloc::string::String;

/// How an exact tie between the two closest shortest decimals is broken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ties {
    /// The higher of the two, as `core`'s formatting picks it.
    Up,
    /// The one whose last digit is even.
    Even,
}

/// A finite float's magnitude as `digits × 10^exponent`, with no trailing
/// zeros in `digits` (zero is `0 × 10^0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Decimal {
    pub(crate) digits: u64,
    pub(crate) exponent: i32,
}

/// The shortest decimal of a finite `f64`'s magnitude.
pub(crate) fn f64_decimal(value: f64, ties: Ties) -> Decimal {
    const FRACTION_BITS: u32 = 52;
    let bits = value.to_bits();
    let fraction = bits & ((1 << FRACTION_BITS) - 1);
    // 11 bits: the cast cannot truncate.
    let biased = ((bits >> FRACTION_BITS) & 0x7ff) as i32;
    binary_to_decimal(fraction, biased, FRACTION_BITS, -1074, ties)
}

/// The shortest decimal of a finite `f32`'s magnitude (shortest for `f32`:
/// `0.1f32` is `1 × 10^-1`).
pub(crate) fn f32_decimal(value: f32, ties: Ties) -> Decimal {
    const FRACTION_BITS: u32 = 23;
    let bits = value.to_bits();
    let fraction = u64::from(bits & ((1 << FRACTION_BITS) - 1));
    // 8 bits: the cast cannot truncate.
    let biased = ((bits >> FRACTION_BITS) & 0xff) as i32;
    binary_to_decimal(fraction, biased, FRACTION_BITS, -149, ties)
}

/// The shortest decimal of the float with this stored `fraction` and
/// `biased` exponent, in a format with `fraction_bits` stored fraction bits
/// whose smallest subnormal is `2^q_min`. `biased` is not the all-ones
/// exponent (the value is finite).
fn binary_to_decimal(
    fraction: u64,
    biased: i32,
    fraction_bits: u32,
    q_min: i32,
    ties: Ties,
) -> Decimal {
    if biased == 0 {
        if fraction == 0 {
            return Decimal {
                digits: 0,
                exponent: 0,
            };
        }
        // Subnormal: value = fraction × 2^q_min, evenly spaced.
        return schubfach(fraction, q_min, true, ties);
    }
    let c = (1u64 << fraction_bits) | fraction;
    let q = q_min - 1 + biased;
    // An integer with fewer bits than the significand: its own digits are
    // the shortest (the rounding interval is narrower than 1).
    if q < 0 && q > -(fraction_bits as i32) - 1 {
        let shift = q.unsigned_abs();
        let whole = c >> shift;
        if whole << shift == c {
            return strip(whole, 0);
        }
    }
    // The interval below a power of two is half as wide as the one above,
    // except at the smallest normal exponent.
    schubfach(c, q, fraction != 0 || biased == 1, ties)
}

/// Schubfach for `value = c × 2^q`, `c > 0`. `regular`: the neighbours
/// below and above are equally far (false only for a power of two whose
/// lower neighbour is half as far).
fn schubfach(c: u64, q: i32, regular: bool, ties: Ties) -> Decimal {
    // Even significands have their interval's ends included: they round to
    // this value under round-half-to-even reading.
    let out = c & 1;
    // In units of a quarter of the spacing: the value and the interval ends.
    let cb = c << 2;
    let cbr = cb + 2;
    let (cbl, k) = if regular {
        (cb - 2, flog10_pow2(q))
    } else {
        (cb - 1, flog10_three_quarters_pow2(q))
    };
    // Scaled by 10^-k, the interval is between 1 and 10 units wide; `h` (1
    // to 4) lines the product up so `rop` returns quarter units.
    let h = (q + flog2_pow10(-k) + 2).unsigned_abs();
    let g = power_of_ten(k);
    let vb = rop(g, cb << h);
    let vbl = rop(g, cbl << h);
    let vbr = rop(g, cbr << h);
    let s = vb >> 2;
    // One digit shorter: the interval holds at most one multiple of ten.
    if s >= 10 {
        let sp10 = s / 10 * 10;
        let tp10 = sp10 + 10;
        let upin = vbl + out <= sp10 << 2;
        let wpin = (tp10 << 2) + out <= vbr;
        if upin != wpin {
            return strip(if upin { sp10 } else { tp10 }, k);
        }
    }
    // The full length: s or s + 1, whichever is inside, else the closer.
    let t = s + 1;
    let uin = vbl + out <= s << 2;
    let win = (t << 2) + out <= vbr;
    if uin != win {
        return strip(if uin { s } else { t }, k);
    }
    // `vb` is 4s to 4s + 3, its last bit sticky: 4s + 2 is exactly halfway.
    let lower = match vb & 3 {
        0 | 1 => true,
        2 => ties == Ties::Even && s % 2 == 0,
        _ => false,
    };
    strip(if lower { s } else { t }, k)
}

/// `g × cp / 2^127` rounded to odd (the integer part, with its last bit
/// set if what was cut off is not zero), computed as the paper does it,
/// with `g = g1 × 2^63 + g0` in two 63-bit halves: the low 64 bits of
/// `g0 × cp` are dropped before the rounding, which is what makes an exact
/// product (`k = 0`, where `g` is `2^125 + 1`) come out exact. `cp` is below
/// 2^63.
#[inline]
fn rop(g: u128, cp: u64) -> u64 {
    const MASK_63: u64 = (1 << 63) - 1;
    // g < 2^126: both halves have 63 bits.
    let g1 = (g >> 63) as u64;
    let g0 = (g as u64) & MASK_63;
    let x1 = ((u128::from(g0) * u128::from(cp)) >> 64) as u64;
    let y = u128::from(g1) * u128::from(cp);
    let (y1, y0) = ((y >> 64) as u64, y as u64);
    // Below 2^63 + 2^62: no overflow.
    let z = (y0 >> 1) + x1;
    let vbp = y1 + (z >> 63);
    vbp | (((z & MASK_63) + MASK_63) >> 63)
}

/// Removes the trailing zeros of `digits` (nonzero) into the exponent.
#[inline]
fn strip(mut digits: u64, mut exponent: i32) -> Decimal {
    if digits != 0 {
        while digits % 100_000_000 == 0 {
            digits /= 100_000_000;
            exponent += 8;
        }
        // Fewer than eight zeros are left: take them as 4, 2 and 1.
        if digits % 10_000 == 0 {
            digits /= 10_000;
            exponent += 4;
        }
        if digits % 100 == 0 {
            digits /= 100;
            exponent += 2;
        }
        if digits % 10 == 0 {
            digits /= 10;
            exponent += 1;
        }
    }
    Decimal { digits, exponent }
}

/// `floor(e × log10(2))`, for `|e| <= 5456721`.
#[inline]
const fn flog10_pow2(e: i32) -> i32 {
    // The product fits 64 bits for that range; the shifted result fits i32.
    ((e as i64 * 661_971_961_083) >> 41) as i32
}

/// `floor(log10(3/4 × 2^e))`, for `|e| <= 5456721`.
#[inline]
const fn flog10_three_quarters_pow2(e: i32) -> i32 {
    ((e as i64 * 661_971_961_083 - 274_743_187_321) >> 41) as i32
}

/// `floor(e × log2(10))`, for `|e| <= 1838394`.
#[inline]
const fn flog2_pow10(e: i32) -> i32 {
    ((e as i64 * 913_124_641_741) >> 38) as i32
}

/// The smallest `k` in [`POWERS`].
const K_MIN: i32 = -324;

/// `g(k) = floor(10^-k / 2^r) + 1`, with `r` such that `2^125 <= g - 1 <
/// 2^126`. Every `k` a finite `f64` or `f32` needs is in the table (the
/// tests walk every exponent of both).
#[inline]
fn power_of_ten(k: i32) -> u128 {
    usize::try_from(k - K_MIN)
        .ok()
        .and_then(|i| POWERS.get(i))
        .copied()
        .unwrap_or(0)
}

/// Two decimal digits for each value below 100.
const PAIRS: &[u8; 200] = b"\
0001020304050607080910111213141516171819\
2021222324252627282930313233343536373839\
4041424344454647484950515253545556575859\
6061626364656667686970717273747576777879\
8081828384858687888990919293949596979899";

/// Writes the decimal digits of `value` so they end just before `end` in
/// `buf`, and returns where they start. `buf` must have room: 20 bytes
/// before `end` hold any `u64`.
#[inline]
pub(crate) fn put_digits(buf: &mut [u8], end: usize, value: u64) -> usize {
    let mut at = end;
    let mut rest = value;
    // Eight digits at a time in 32-bit arithmetic, then pairs.
    while rest >= 100_000_000 {
        // Below 10^8: fits u32.
        let mut chunk = (rest % 100_000_000) as u32;
        rest /= 100_000_000;
        for _ in 0..4 {
            at -= 2;
            put_pair(buf, at, chunk % 100);
            chunk /= 100;
        }
    }
    // Below 10^8: fits u32.
    let mut rest = rest as u32;
    while rest >= 100 {
        at -= 2;
        put_pair(buf, at, rest % 100);
        rest /= 100;
    }
    if rest >= 10 {
        at -= 2;
        put_pair(buf, at, rest);
    } else {
        at -= 1;
        if let Some(slot) = buf.get_mut(at) {
            // Below 10.
            *slot = b'0' + rest as u8;
        }
    }
    at
}

#[inline(always)]
fn put_pair(buf: &mut [u8], at: usize, pair: u32) {
    let from = pair as usize * 2;
    if let (Some(slot), Some(digits)) = (buf.get_mut(at..at + 2), PAIRS.get(from..from + 2)) {
        slot.copy_from_slice(digits);
    }
}

/// Appends ASCII bytes (digits and signs). One `push` each after one
/// `reserve`: for the few bytes of a number this is cheaper than checking
/// a stack buffer as UTF-8 to `push_str` it. A byte above 0x7F would be
/// pushed as its two-byte character, so the `String` stays UTF-8 whatever
/// the bytes are; only ASCII is ever passed.
#[inline]
pub(crate) fn push_ascii(out: &mut String, bytes: &[u8]) {
    out.reserve(bytes.len());
    for &b in bytes {
        out.push(char::from(b));
    }
}

/// Appends `count` ASCII zeros.
#[inline]
fn push_zeros(out: &mut String, count: usize) {
    for _ in 0..count {
        out.push('0');
    }
}

/// Appends `decimal` laid out as JSON: a decimal for exponents from -5 up
/// to 15 (`0.00001`, `1.0`, `1000000000000000.0`), `d.ddde-N` / `d.ddde+N`
/// (`+` only if `plus`, else `d.dddeN`) otherwise. Always with a fraction
/// or an exponent, so it reads back as a float.
pub(crate) fn write_decimal(out: &mut String, negative: bool, decimal: Decimal, plus: bool) {
    let mut buf = [0u8; 20];
    let start = put_digits(&mut buf, 20, decimal.digits);
    let digits = buf.get(start..).unwrap_or_default();
    // At most 20 digits.
    let count = digits.len() as i32;
    // The exponent of the first digit: d.ddd × 10^sci.
    let sci = decimal.exponent + count - 1;
    // The longest text is 24 bytes (`-2.2250738585072014e-308`, or the
    // decimal form of an exponent from -5 to 15).
    out.reserve(26);
    if negative {
        out.push('-');
    }
    if (-5..16).contains(&sci) {
        if sci >= 0 {
            // `sci + 1` digits before the point.
            let whole = sci.unsigned_abs() as usize + 1;
            let (int, frac) = digits.split_at(whole.min(digits.len()));
            push_ascii(out, int);
            push_zeros(out, whole - int.len());
            out.push('.');
            if frac.is_empty() {
                out.push('0');
            } else {
                push_ascii(out, frac);
            }
        } else {
            out.push_str("0.");
            push_zeros(out, sci.unsigned_abs() as usize - 1);
            push_ascii(out, digits);
        }
    } else {
        let (lead, frac) = digits.split_at(1.min(digits.len()));
        push_ascii(out, lead);
        if !frac.is_empty() {
            out.push('.');
            push_ascii(out, frac);
        }
        // Separate pushes, each a constant store.
        out.push('e');
        if sci < 0 {
            out.push('-');
        } else if plus {
            out.push('+');
        }
        let mut exp = [0u8; 4];
        let from = put_digits(&mut exp, 4, u64::from(sci.unsigned_abs()));
        push_ascii(out, exp.get(from..).unwrap_or_default());
    }
}

/// `g(k)` for `k` from `K_MIN` to 292, generated with exact integer
/// arithmetic (the tests recompute every entry).
#[rustfmt::skip]
static POWERS: [u128; 617] = [
    0x278676e4ad38c6ea5b01e8b09aa0d1b5,
    0x3f3d8b077b8e0b10919ca780f767b5ee,
    0x3297a26c62d808da0e16ec672c52f7f2,
    0x28794ebd1be00714d81256b8f0425ff5,
    0x20610bca7cb338dd79a84560c0351991,
    0x33ce7943fab85afbf5da089acd21c281,
    0x2971fa9cc8937bfcc4ae6d48a41b0201,
    0x2127fbb0a075fcca36f1f106e9af34cd,
    0x350cc5e767232e1057e981a4a918547b,
    0x2a709e52b8e8f1a6acbace1d541376c9,
    0x21f3b1dbc720c15223c8a4e44342c56e,
    0x3652b62c71ce021d060dd4a06b9e08b0,
    0x2b755e89f4a4ce7d9e7176e6bc7e6d59,
    0x22c44ba19083d8647ec12bebc9febde1,
    0x37a0790280d2f3d3fe01dfdfa9979635,
    0x2c8060cecd758fdccb34b319547944f7,
    0x2399e70bd7913fe3d5c3c27aa9fa9d93,
    0x38f63e7958e86639560603f7765dc8ea,
    0x2d91cb94472051c77804cff92b7e3a55,
    0x2474a2dd05b3749f93370cc755fe9511,
    0x3a5437c8091f20ff51f1ae0bbcca881b,
    0x2ea9c639a0e5b3ff74c1580963d539af,
    0x25549e9480b7c332c3cde0078310faf3,
    0x3bba97540126051e0616333f381b2b1e,
    0x2fc8791000eb374b3811c298f9af55b1,
    0x2639fa7333ef5f6f600e35472e25de28,
    0x3d2990b8531898b23349eed849d6303f,
    0x30ee0d60427a13c1c2a18be03b11c033,
    0x2724d780352e76349bb46fe695a7ccf5,
    0x3ea158cd21e3f0542c53e63dbc3fae55,
    0x321aad70e7e98d10237651cafcffbeaa,
    0x2815578d865470d9b5f8416f30cc9888,
    0x201112d79ea9f3e15e603458f3d6e06d,
    0x334e848c310fec9bca3386f4b957cd7b,
    0x290b9d3cf40cbd496e8f9f2a2ddfd796,
    0x20d61763f670976df20c7f54f17fdfab,
    0x3489bf06571a8be31ce0cbbb1bffcc45,
    0x2a07cc05127ba31c171a3c95afffd69e,
    0x219fd66a752fb5b0127b63aaf3331218,
    0x35cc8a43eeb2bc4cea5f05de51eb5026,
    0x2b0a0836588efd0a5518d17ea7ef7352,
    0x226e6cf846d8ca6eaa7a41321ff2c2a8,
    0x371714c0715add7ddd906850331e043f,
    0x2c1277005aaf1797e47386a68f4b3699,
    0x2341f8cd1558dfacb6c2d21ed908f87b,
    0x38698e14eef49914579e1cfe280e5a5d,
    0x2d213e7725907a76ac7e7d98200b7b7e,
    0x241a985f514061f889fecae019a2c932,
    0x39c426fee867032743314499c29e0eb6,
    0x2e368598b9ec0285cf5a9d47cee4d891,
    0x24f86ae094bcced172aee4397250ad41,
    0x3b27116754614ae8b77e39f583b44868,
    0x2f527452a9e76f2092cb61913629d387,
    0x25db90422185f280756f8140f8217605,
    0x3c928069cf3cb733ef18cece59cf233c,
    0x30753387d8fd5f5cbf470bd847d8e8fd,
    0x26c429397a644c4a329f3cad064720ca,
    0x3e06a85bf706e076b7652de1a3a50143,
    0x319eed165f38b3922c50f1814fb73436,
    0x27b2574518fa2941bd0d8e010c92902b,
    0x3f83bed4f4c37535fb48e334e0ea8045,
    0x32cfcbdd909c5dc4c9071c2a4d88669d,
    0x28a63cb1407d17d0a0d27ceea46d1ee4,
    0x2084fd5a99fdaca6e70eca58838a7f1d,
    0x3407fbc42995e10b0b4add5a6c10cb62,
    0x299ffc9cee1180d5a2a24aaebcda3c4e,
    0x214cca1724dacd77b54ea22563e1c9d8,
    0x3547a9bea15e158c554a9d089fcfa95a,
    0x2a9fbafee77e77a3776ee406e63fbaae,
    0x2219626585fec61c5f8be99f1e996225,
    0x368f03d5a3313cfa327975cb64289d08,
    0x2ba59caae8f430c828612b091ced4a6d,
    0x22eae3bbed90270686b4226db0bdd524,
    0x37de392caf4d0b3da4536a491ac95506,
    0x2cb1c756f2a408fe1d0f883a7bd44405,
    0x23c16c458ee9a0cb4a72d361fca9d004,
    0x39357a08e4a9014543eaebcffaa94cd3,
    0x2dc461a0b6ed9a9dcfef230cc88770a9,
    0x249d1ae6f8be154b0cbf4f3d6d3926ee,
    0x3a94f7d7f4635544e1321862485b717c,
    0x2edd931329e91103e75b46b506af8dfd,
    0x257e0f4287eda73652af6bc405593e64,
    0x3bfce5373fe2a523b77f12d33bc1fd6d,
    0x2ffd842c331bb74fc5ff42429634cabd,
    0x266469bcf5afc5d96b329b68782a3bcb,
    0x3d6d75fb22b2d628ab842bda59dd2c77,
    0x31245e628228ab53bc69bcaeae4a89f9,
    0x27504b8201ba22a96387ca25583ba194,
    0x3ee6df366929d10f05a6103bc05f68ed,
    0x32524c2b8754a73f37b80cfc99e5ed8a,
    0x2841d689391085cc2c933d96e184be08,
    0x2034aba0fa739e3cf075cadf1ad09807,
    0x3387790190b8fd2e4d8944982ae759a4,
    0x29392d9ada2d97583e076a135585e150,
    0x20fa8ae24824791364d2bb42aad1810d,
    0x34c4116a0d07281f07b7920444826815,
    0x2a367454d738ece59fc60e69d0685344,
    0x21c529dd78fa571e196b3ebb0d20429d,
    0x360842fbf4c3be968f11fdf815006a94,
    0x2b39cf2ff702feded8db319344005543,
    0x2294a5bff8cf324be0af5adc3666aa9c,
    0x37543c665ae51d46344bc4938a3dddc7,
    0x2c4363851584176b5d096a0fa1cb17d2,
    0x23691c6a779cdf89173abb3fb4a27975,
    0x38a82d7725c7cc0e8b912b992103f588,
    0x2d535792849fd6720940efadb4032ad3,
    0x2442ac7536e6452807672624900288a9,
    0x3a044721f1706ea6723ea36db337410e,
    0x2e69d2818df38bb85b654f8af5c5cda5,
    0x25217534718fa2f9e2b772d5916b0aeb,
    0x3b68bb871c1904c30458b7bc1bde77dd,
    0x2f86fc6c167a6a359d13c630164b9318,
    0x260596bcdec854f7b0dc9e8cdea2dc13,
    0x3cd5bdfafe0d54bf8160fdae31049351,
    0x30aafe6264d776ff9ab3fe24f403a90e,
    0x26ef31e850ac5f32e229981d9002eda5,
    0x3e4b830d4de0985169dc2695b337e2a1,
    0x31d602710b1a137454b01ede28f9821b,
    0x27de685a6f480f9043c018b1ba6134e2,
    0x3fca4090b20ce5b39f99c11c5d68549d,
    0x330833a6f4d71e294c7b00e37ded107e,
    0x28d35c8590ac182109fc00b5fe574065,
    0x20a916d14089ace73b3000919845cd1d,
    0x3441be1b9a75e171f84ccdb5c06fae95,
    0x29ce31afaec4b45b2d0a3e2b00595877,
    0x2171c159589d5d15bda1cb5599e11393,
    0x3582cef55a9561bc629c7888f634ec1e,
    0x2acf0bf77baab496b549fa072b5d89b1,
    0x223f3cc5fc8890789107fb38ef7e07c1,
    0x36cb946ffa741a5a81a65ec17f300c68,
    0x2bd610599529aeaece1eb23465c009ed,
    0x2311a6ae10ee2558a4e55b5d1e333b24,
    0x381c3de34e49d55aa16ef894fd1ec506,
    0x2ce364b5d83b11154df2607730e56a6c,
    0x23e91d5e4695a7443e5b805f5a5121f0,
    0x3974fbca0a890ba063c59a322a1b697f,
    0x2df72fd4d53a6fb383047b5b54e2bacc,
    0x24c5bfdd7761f2f60269fc4910b5623d,
    0x3ad5ffc8bf031e566a432d41b45569fb,
    0x2f11996d659c184521cf5767c37787fc,
    0x25a7adf11e1679d0e7d912b9692c6cca,
    0x3c3f7cb4fcf0c2e7d95b5128a8471476,
    0x3032ca2a63f3cf1fe115da86ed05a9f8,
    0x268f0821e98fd8e64dab1538bd9e2193,
    0x3db1a69ca8e627d6e2ab552795c9cf52,
    0x315aebb0871e86458222aa86116e3f75,
    0x277befc06c186b6ace822204dabe992a,
    0x3f2cb2cd79c0abde17369cd49130f510,
    0x328a28a46166efe4df5ee3dd40f3f740,
    0x286e86e9e7858cb71918b64a9a5cc5cd,
    0x20586bee52d13d5f4746f83baeb09e3e,
    0x33c0acb08481fbcba53e59f91780fd2f,
    0x2966f08d36ce630950feae60df9a6426,
    0x211f26d75f0b826dda65584d7faeb685,
    0x34fea48bcb459d7c90a226e265e4573b,
    0x2a65506fd5d14aca0d4e8581eb1d1295,
    0x21eaa6bfde4108a1a43ed134bc174211,
    0x36443dffca01a76906cae85460253682,
    0x2b69cb33080152ba6bd586a9e6842b9b,
    0x22bb08f5a0010efb89779eee52035616,
    0x3791a7ef666817f8dbf297e3b66bbcef,
    0x2c7486591eb9acc7165bacb62b8963f3,
    0x23906b7a7efaf09f451623c4efa11cc2,
    0x38e7125d97f7e7653b569fa17f682e03,
    0x2d85a84adff985ea95dee61acc535803,
    0x246aed08b32e04bbab18b8157042accf,
    0x3a44ae7451e33ac5de8df355806aae18,
    0x2e9d585d0e4f6237e53e5c4466bbbe7a,
    0x254aad173ea5e82cb765169d1efc9861,
    0x3baaae8b976fd9e1256e8a94fe60f3cf,
    0x2fbbbed612bfe180eabed543feb3f63f,
    0x262fcbde75664e00bbcbddcffef65e99,
    0x3d194630bbd6e3345fac961997f0975b,
    0x30e104f3c978b5c37fbd44e1465a12af,
    0x271a6a5ca12d5e35ffca9d810514dbbf,
    0x3e90aa2dceaefd2332ddc8ce6e87c5ff,
    0x320d54f17225974f5be4a0a525396b32,
    0x280aaa5ac1b7ac3f7cb6e6ea842def5c,
    0x200888489af9569930925255368b25e3,
    0x3340da0dc4c224284db6ea21f0dea304,
    0x2900ae716a34e9b9d7c5881b2718826a,
    0x20cd585abb5d87c7dfd139af527a01ef,
    0x347bc0912bc8d93fcc81f5e550c3364a,
    0x29fc9a0dbca0adcca39b2b1dda35c508,
    0x2196e1a496e6f17082e288e4ae916a6d,
    0x35be35d424a4b580d16a74a1174f10ae,
    0x2afe917683b6f79a4121f6e745d8da25,
    0x2265412b9c925fae9a8192529e4714eb,
    0x37086845c75099175d9c1d50fd3e87dd,
    0x2c06b9d16c407a7917b01773fdcb9fe4,
    0x233894a789cd2ec74626792997d61984,
    0x385a8772761517a53d0a5b75bfbcf59f,
    0x2d1539285e77461dca6eaf916630c47f,
    0x2410fa86b1f904e4a1f2260deb5a36cc,
    0x39b4c40ab65b3b0769837016455d247a,
    0x2e2a366ef848fc05ee02c011d1175062,
    0x24ee91f2603a6337f19bccdb0dac404e,
    0x3b174fea33909ebfe8f947c4e2ad33b0,
    0x2f45d98829407effed94396a4ef0f627,
    0x25d17ad3543398ccbe102deea58d91b9,
    0x3c825e1eed1f5ae13019e3176f48e927,
    0x30684b4bf0e5e24dc014b5ac590720ec,
    0x26b9d5d65a5181d7ccdd5e237a6c1a57,
    0x3df622f09082695947c8969f2a46908a,
    0x3191b58d406854476ca0787f5505406f,
    0x27a7c4710053769f8a19f9ff773766bf,
    0x3f72d3e800858a98dcf65ccbf1f23dfe,
    0x32c24320006ad547172b7d6ff4c1cb32,
    0x289b68e666bbddd278ef978cc3ce3c28,
    0x207c53eb856317db93f2dfa3cfd83020,
    0x33fa1fdf3bd1bfc5b98499061959e699,
    0x2994e64c2fdaffd16136e0d1ade18548,
    0x2143eb702648cca780f8b3daf181376d,
    0x353978b370747aa59b27862b1c01f247,
    0x2a94608f8d29fbb7af52d1bc1667f506,
    0x22104d3fa421962c8c424163451ff738,
    0x36807b99069c237a7a039bd208332526,
    0x2b99fc7a6bb01c61fb361641a028ea85,
    0x22e196c856267d1b2f5e78348020bb9e,
    0x37cf57a6f03d94f84bca59ed99cdf8fc,
    0x2ca5dfb8c03143f9d63b7b247b0b2d96,
    0x23b7e62d668dcffb11c92f50626f57ac,
    0x39263d1570e2e65e82db7ee703e55912,
    0x2db830ddf3e8b84b9be2cbec031de0dc,
    0x24935a4b2986f9d6164f09899c17e716,
    0x3a855d450f3e5c89bd4b4275c68ca4f0,
    0x2ed1176a72984a07caa29b916ba3b726,
    0x257412bb8ee03b396ee87c74561c9285,
    0x3beceac5b166c528b173fa53bcfa8408,
    0x2ff0bbd15ab89dba278ffb7630c869a0,
    0x265a2fdaaefa17c81fa662c4f3d387b3,
    0x3d5d195de4c3594032a3d13b1fb8d91f,
    0x3117477e509c47668ee9742f4c93e0e6,
    0x2745d2cb73b0391ed8bac3590a0fe71e,
    0x3ed61e1252b38e97c12ad228101971c9,
    0x3244e4db755c721300ef0e8673478e3b,
    0x28371d7c5de38e759a58d86b8f6c71c9,
    0x202c1796b182d85e1513e0560c56c16e,
    0x3379bf57826af3c9bb530089ad579be2,
    0x292e32ac68558fd495dc006e2446164f,
    0x20f1c22386aad976de4999f1b69e783f,
    0x34b6036c0aaaf58afd428fe92430c065,
    0x2a2b35f00888c46f31020cba835a3384,
    0x21bc2b266d3a36bf5a680a2ecf7b5c69,
    0x35f9dea3e1f6bdfef70cdd17b25efa42,
    0x2b2e4bb64e5efe659270b0dfc1e59502,
    0x228b6fc50b7f31eadb8d5a4c9b1e10ce,
    0x37457fa1abfeb644927bc3adc4fce7b0,
    0x2c37994e23322b6a0ec96957d0ca52f3,
    0x235fadd81c2822bb3f07877973d50f29,
    0x3899162693736ac531a5a58f1fbb4b75,
    0x2d4744eba92922375aeaead8e62f6f91,
    0x243903efba874e92af22557a51bf8c74,
    0x39f4d3192a7217511836ef2a1c65ad86,
    0x2e5d75adbb8e790dacf8bf54e3848ad2,
    0x25179157c93ec73e23fa32aa4f9d3bdb,
    0x3b58e88c75313ec9d329eaaa18fb92f8,
    0x2f7a53a390f4323b0f54bbbb472fa8c6,
    0x25fb761c73f68e95a5dd62fc38f2ed6c,
    0x3cc589c71ff0e422a2fbd1938e517bdf,
    0x309e07d27ff3e9b54f2fdadc71dac97f,
    0x26e4d30eccc3215dd8f3157d27e23acc,
    0x3e3aeb4ae1383562f4b82261d969f7ad,
    0x31c8bc3be7602ab590934eb4adee5fbe,
    0x27d3c9c985e688914075d8908b251965,
    0x3fb942dc0970da8200bc8db411d4f56e,
    0x32fa9be33ac0aece66fd3e29a7dd9125,
    0x28c87cb5c89a2571ebfdcb54864ada84,
    0x20a063c4a07b5127effe3c439ea2486a,
    0x3433d2d433f881d97ffd2d38fdd073dc,
    0x29c30f1029939b146664242d97d9f64a,
    0x2168d8d9badc7c1051e9b68adfe191d5,
    0x35748e292afa601a1ca924116635b621,
    0x2ac3a4edbbfb8014e3ba83411e915e81,
    0x22361d8afcc93343e962029a7edab201,
    0x36bcfc1194751ed30f03375d97c45001,
    0x2bca63414390e575a59c2c4adfd04001,
    0x23084f676940b7915149bd08b30d0001,
    0x380d4bd8a8678c1bb542c80deb480001,
    0x2cd76fe086b93ce2f768a00b22a00001,
    0x23df8cb39efa971bf9208008e8800001,
    0x3965adec3190f1c65b67334174000001,
    0x2deaf189c140c16b7c528f6790000001,
    0x24bbf46e3433cdef96a872b940000001,
    0x3ac653e386b9497f5773eac200000001,
    0x2f050fe938943acc45f6556800000001,
    0x259da6542d43623d04c5112000000001,
    0x3c2f7086aed236c807a1b50000000001,
    0x3025f39ef241c56cd2e7c40000000001,
    0x2684c2e58e9b04570f1fd00000000001,
    0x3da137d5b0f806f1b1cc800000000001,
    0x314dc6448d9338c15b0a000000000001,
    0x27716b6a0adc2d677c08000000000001,
    0x3f1bdf10116048a59340000000000001,
    0x327cb2734119d3b7a900000000000001,
    0x2863c1f5cdae42f95400000000000001,
    0x204fce5e3e2502611000000000000001,
    0x33b2e3c9fd0803ce8000000000000001,
    0x295be96e640669720000000000000001,
    0x21165458500521280000000000000001,
    0x34f086f3b33b68400000000000000001,
    0x2a5a058fc295ed000000000000000001,
    0x21e19e0c9bab24000000000000000001,
    0x3635c9adc5dea0000000000000000001,
    0x2b5e3af16b1880000000000000000001,
    0x22b1c8c1227a00000000000000000001,
    0x3782dace9d9000000000000000000001,
    0x2c68af0bb14000000000000000000001,
    0x2386f26fc10000000000000000000001,
    0x38d7ea4c680000000000000000000001,
    0x2d79883d200000000000000000000001,
    0x246139ca800000000000000000000001,
    0x3a352944000000000000000000000001,
    0x2e90edd0000000000000000000000001,
    0x2540be40000000000000000000000001,
    0x3b9aca00000000000000000000000001,
    0x2faf0800000000000000000000000001,
    0x2625a000000000000000000000000001,
    0x3d090000000000000000000000000001,
    0x30d40000000000000000000000000001,
    0x27100000000000000000000000000001,
    0x3e800000000000000000000000000001,
    0x32000000000000000000000000000001,
    0x28000000000000000000000000000001,
    0x20000000000000000000000000000001,
    0x33333333333333333333333333333334,
    0x28f5c28f5c28f5c28f5c28f5c28f5c29,
    0x20c49ba5e353f7ced916872b020c49bb,
    0x346dc5d63886594af4f0d844d013a92b,
    0x29f16b11c6d1e108c3f3e0370cdc8755,
    0x218def416bdb1a6d698fe69270b06c44,
    0x35afe535795e90af0f4ca41d811a46d4,
    0x2af31dc4611873bf3f70834acdae9f10,
    0x225c17d04dad2965cc5a02a23e254c0d,
    0x36f9bfb3af7b756fad5cd10396a21347,
    0x2bfaffc2f2c92abfbde3da69454e75d3,
    0x232f33025bd42232fe4fe1edd10b9175,
    0x384b84d092ed0384ca19697c81ac1bef,
    0x2d09370d42573603d4e1213067bce326,
    0x24075f3dceac2b3643e74dc052fd8285,
    0x39a5652fb1137856d30baf9a1e626a6d,
    0x2e1dea8c8da92d12426fbfae7eb521f1,
    0x24e4bba3a4875741cebfcc8b9890e7f4,
    0x3b07929f6da558694acc7a78f41b0cba,
    0x2f394219248446baa23d2ec729af3d62,
    0x25c768141d369efbb4fdbf05baf29781,
    0x3c7240202ebdcb2c54c931a2c4b758cf,
    0x305b66802564a289dd6dc14f03c5e0a5,
    0x26af8533511d4ed4b1249aa59c9e4d51,
    0x3de5a1ebb4fbb1544ea0f76f60fd4882,
    0x318481895d962776a54d92bf80caa068,
    0x279d346de4781f921dd7a89933d54d20,
    0x3f61ed7ca0c0328362f2a75b86221500,
    0x32b4bdfd4d668ecf825bb91604e810cd,
    0x289097fdd7853f0c684960de6a5340a4,
    0x2073accb12d0ff3d203ab3e521dc33b6,
    0x33ec47ab514e652e99f7863b696052bd,
    0x2989d2ef743eb7587b2c6b62bab37564,
    0x213b0f25f69892ad2f56bc4efbc2c450,
    0x352b4b6ff0f41de1e55793b192d13a1a,
    0x2a8909265a5ce4b4b77942f475742e7b,
    0x22073a8515171d5d5f9435905df68b96,
    0x3671f73b54f1c89565b9ef4d63241289,
    0x2b8e5f62aa5b06ddeafb25d782834207,
    0x22d84c4eeeaf38b188c8eb12cecf6806,
    0x37c07a17e44b8de8dadb11b7b14bd9a3,
    0x2c99fb46503c7187157c0e2c8dd647b5,
    0x23ae629ea696c138ddfcd823a4ab6c91,
    0x391704310a8acec1632e269f6ddf141b,
    0x2dac035a6ed572344f581ee5f17f4349,
    0x24899c4858aac1c372ace584c1329c3b,
    0x3a75c6da27779c6beaae3c079b842d2a,
    0x2ec49f14ec5fb0565558300616035755,
    0x256a18dd89e626ab7779c004de6912ab,
    0x3bdcf495a9703ddf258f99a163db5111,
    0x2fe3f6de212697e5b7a614811caf740d,
    0x264ff8b1b41edfeaf951aa00e3bf900b,
    0x3d4cc11c53649977f54f7667d2cc19ab,
    0x310a3416a91d47932aa5f8530f09ae22,
    0x273b5cdeedb1060f55519375a5a1581b,
    0x3ec56164af81a34bbbb5b8bc3c3559c5,
    0x3237811d593482a2fc9160969691149e,
    0x282c674aadc39bb596dab3ababa743b2,
    0x202385d557cfafc478aef622efb902f5,
    0x336c0955594c4c6d8de4bd04b2c19e54,
    0x29233aaaadd6a38ad7ea30d08f014b76,
    0x20e8fbbbbe454fa24654f3da0c01092c,
    0x34a7f92c63a21903a3bb1fc346680eac,
    0x2a1ffa89e94e7a694fc8e635d1ecd88a,
    0x21b32ed4baa52ebaa63a51c4a7f0ad3b,
    0x35eb7e212aa1e45dd6c3b607731aaec4,
    0x2b22cb4dbbb4b6b1789c919f8f488bd0,
    0x22823c3e2fc3c55ac6e3a7b2d906d640,
    0x3736c6c9e60608913e390c515b3e239a,
    0x2c2bd23b1e6b3a0dcb60d6a77c31b615,
    0x235641c8e52294d7d5e7121f968e2b44,
    0x388a02db0837548c8971b698f0e3786d,
    0x2d3b357c0692aa0a078e2bad8d82c6bd,
    0x242f5dfcd20eee6e6c71bc8ad79bd231,
    0x39e5632e1ce4b0b0ad82c7448c2c8382,
    0x2e511c24e3ea26f3be023903a356cf9b,
    0x250db01d8321b8c2fe682d9c82abd949,
    0x3b4919c8d1cf8e04ca4048fa6aac8edb,
    0x2f6dae3a4172d803d5003a61eef07249,
    0x25f1582e9ac24669773361e7f259f507,
    0x3cb559e42ad070a8beb89ca6508fee71,
    0x309114b688a6c086fefa16eb73a6585b,
    0x26da76f86d52339f3261abef8fb846af,
    0x3e2a57f3e21d1f651d691318e5f3a44b,
    0x31bb798fe8174c50e4540f471e5c836f,
    0x27c92e0cb9ac3d0d8376729f4b7d35f3,
    0x3fa849adf5e061af38bd84321261efeb,
    0x32ed07be5e4d1af293cad0280eb4bfef,
    0x28bd9fcb7ea4158edca240200bc3ccbf,
    0x2097b309321cde0be3b50019a3030a33,
    0x3425eb41e9c7c9ac9f88002904d1a9ea,
    0x29b7ef67ee396e23b2d3335403daee55,
    0x215ff2b98b6124e95bdc291003158b77,
    0x35665128df01d4a892f9db4cd1bc1258,
    0x2ab840ed7f34aa207594af70a7c9a847,
    0x222d00bdff5d54e6c476f2c0863aed06,
    0x36ae6796656221713a57eacda3917b3c,
    0x2bbeb9451de81ac0fb7988a482dac8fd,
    0x22fefa9db1867bcd95fad3b6cf156d97,
    0x37fe5dc91c0a5faf565e1f8ae4ef15be,
    0x2ccb7e3a7cd5195911e4e608b725aaff,
    0x23d5fe9530aa7aada7ea51a0928488cc,
    0x39566421e7772aaf7310829a84074146,
    0x2ddeb68185f8eef2c2739baed005cdd2,
    0x24b22b9ad193f25bcec2e2f24004a4a8,
    0x3ab6ac2ae8ecb6f94ad16b1d333aa10c,
    0x2ef889bbed8a2bfaa241227dc2954da3,
    0x2593a163246e89954e9a81fe35443e1c,
    0x3c1f689ea0b0dc22175d9cc9eed39694,
    0x3019207ee6f3e34e7917b0a18bdc7876,
    0x267a8065858fe90b9412f3b46fe39392,
    0x3d90cd6f3c1974df535185ed7fd285b6,
    0x3140a458fce12a4c42a79e57997537c5,
    0x2766e9e0ca4dbb703552e512e12a9304,
    0x3f0b0fce107c5f19eeeb081e3510eb39,
    0x326f3fd80d304c14bf226ce4f740bc2e,
    0x2858ffe00a8d09aa3281f0b72c33c9be,
    0x20473319a20a6e21c2018d5f568fd498,
    0x33a51e8f69aa49cf9ccf48988a7fba8d,
    0x2950e53f87bb6e3fb0a5d3ad3b99620b,
    0x210d8432d2fc5832f3b7dc8a96144e6f,
    0x34e26d1e1e608d1e52bfc7442353b0b1,
    0x2a4ebdb1b1e6d74b756639034f7626f4,
    0x21d897c15b1f12a2c451c735d92b525d,
    0x362759355e981dd13a1c71efc1deea2e,
    0x2b52adc44bace4a761b05b2634b254f2,
    0x22a88b036fbd83b91af37c1e908eaa5b,
    0x3774119f192f39282b1f2cfdb41776f8,
    0x2c5cdae5adbf60ecef4c23fe29ac5f2d,
    0x237d7beaf165e723f2a34ffe87bd18f1,
    0x38c8c644b56fd83984387ffda5fb5b1b,
    0x2d6d6b6a2abfe02e0360666484c915af,
    0x24578921bbccb35802b3851d3707448c,
    0x3a25a835f94785599dec082ebe720746,
    0x2e8486919439377ae4bcd358985b3905,
    0x2536d20e102dc5fbea30a913ad15c738,
    0x3b8ae9b019e2d65fdd1aa81f7b560b8c,
    0x2fa2548ce18245197daeece5fc44d609,
    0x261b76d71ace9dadfe258a51969d7808,
    0x3cf8be24f7b0fc4996a276e8f0fbf33f,
    0x30c6fe83f95a636e121b9253f3fcc299,
    0x2705986994484f8b41afa84329970214,
    0x3e6f5a4286da18decf7f739ea8f19ced,
    0x31f2ae9b9f14e0b23f99294bba5ae3f1,
    0x27f5587c7f43e6f4ffadbaa2fb7be98d,
    0x3feef3fa65397187ff7c5dd1925fdc15,
    0x33258ffb842df46ccc637e4141e649ab,
    0x28ead9960357f6bd704f983434b83aef,
    0x20bbe144cf79923126a6135cf6f9c8bf,
    0x345fced47f28e9e83dd685618b294132,
    0x29e63f1065ba54b9cb12044e08edcdc2,
    0x2184ff405161dd616f419d0b3a57d7ce,
    0x35a19866e89c9568b20294dec3bfbfb0,
    0x2ae7ad1f207d4453c19baa4bcfcc995a,
    0x2252f0e5b39769dc9ae2eea30ca3ade1,
    0x36eb1b091f58a960f7d17dd1add2afcf,
    0x2bef48d41913bab3f97464a7be42263f,
    0x2325d3dce0dc955cc790508631ce84ff,
    0x383c862e3494222e0c1a1a704fb0d4cc,
    0x2cfd3824f6dce824d67b4859d95a43d6,
    0x23fdc683f8b0b9b711fc39e17aae9cab,
    0x39960a6cc11ac2be832d2968c44a9445,
    0x2e11a1f09a7bcefecf575453d03ba9d1,
    0x24dae7f3aec9726572ac4376402fbb0e,
    0x3af7d985e47583d58446d256cd192b49,
    0x2f2cae04b6c469779d0575123dadbc3a,
    0x25bd5803c569edf94a6ac40e97be302f,
    0x3c62266c6f0fe328771139b0f2c9e6b1,
    0x304e85238c0cb5b9f8da948d8f07ebc1,
    0x26a5374fa33d5e2e60aedd3e0c065634,
    0x3dd5254c3862304a344afb9679a3bd20,
    0x31775109c6b4f36e903bfc78614fca80,
    0x2792a73b055d8f8ba6966393810ca200,
    0x3f510b91a22f4c12a423d2859b476999,
    0x32a73c7481bf700ee9b642047c392148,
    0x2885c9f6ce32c00bee2b680396941aa0,
    0x206b07f8a4f5666ff1bc53361210154d,
    0x33de73276e5570b31c6085235019bbae,
    0x297ec285f1ddf3c27d1a041c40149625,
    0x21323537f4b18fceca7b367d0010781d,
    0x351d21f3211c194add91f0c8001a59c8,
    0x2a7db4c280e3476f17a7f3d3334847d4,
    0x21fe2a3533e905f279532975c2a03976,
    0x366376bb8641a31d8eeb75893766c256,
    0x2b82c562d1ce1c17a5892ad42c523512,
    0x22cf044f0e3e7cdfb7a0ef102374f742,
    0x37b1a07e7d30c7cc59017e8038bb2536,
    0x2c8e19feca8d6ca37a67986693c8ea91,
    0x23a4e198a20abd4f951fad1edca0bba8,
    0x3907cf5a9cddfbb28832ae97c76792a5,
    0x2d9fd9154a4b2fc2068ef21305ec7551,
    0x247fe0ddd508f3019ed8c1a8d189f774,
    0x3a66349621a7eb35caf4690e1c0ff253,
    0x2eb82a11b48655c4a25d20d816732843,
    0x256021a7c39eab03b5174d79ab8f5369,
    0x3bcd02a605caab3921bee25c45b21f0e,
    0x2fd735519e3bbc2db498b5169e2818d8,
    0x2645c4414b62fcf15d46f7454b534713,
    0x3d3c6d35456b2e4efba4bed545520b52,
    0x30fd242a9def583f2fb6ff110441a2a8,
    0x2730e9bbb18c4698f2f8cc0d9d014eed,
    0x3eb4a92c4f46d75b1e5ae015c80217e1,
    0x322a20f03f6bdf7c1848b344a001acb4,
    0x2821b3f365efe5fce03a2903b3348a2a,
    0x201af65c518cb7fd802e873628f6d4ee,
    0x335e56fa1c14599599e40b89db2487e3,
    0x29184594e3437ade14b66fa17c1d3983,
    0x20e037aa4f692f181091f2e7967dc79c,
    0x3499f2aa18a84b59b41cb7d8f0c93f5f,
    0x2a14c221ad536f7af67d5fe0c0a0ff80,
    0x21aa34e7bddc592f2b977fe70080cc66,
    0x35dd2172c9608eb1df58cca4cd9ae0a3,
    0x2b174df56de6d88e4c470a1d7148b3b6,
    0x22790b2abe5246d83d05a1b1276d5c92,
    0x372811ddfd507159fb3c35e83f1560e9,
    0x2c200e4b310d277b2f635e5365aab3ed,
    0x234cd83c273db92f591c4b75eaeef658,
    0x387af39371fc5b7ef4fa125644b18a26,
    0x2d2f2942c196af98c3fb41de9d5ad4eb,
    0x2425ba9bce122613cffc34b2177bdd89,
    0x39d5f75fb01d09b94cc6bab68bf96274,
    0x2e44c5e6267da1610a38955ed6611b90,
    0x2503d184eb97b44da1c6dde5784dafa7,
    0x3b394f3b128c53af693e2fd58d49190b,
    0x2f610c2f4209dc8c5431bfde0aa0e0d5,
    0x25e73cf29b3b16d6a9c1664b3bb3e711,
    0x3ca52e50f85e8af10f9bd6dec5eca4e8,
    0x3084250d937ed58da616457f04bd50ba,
    0x26d01da475ff113e1e783798d09773c8,
    0x3e19c9072331b53030c058f480f252d9,
    0x31ae3a6c1c27c4268d66ad9067284247,
    0x27be952349b969b8711ef14052869b6c,
    0x3f97550542c242c0b4fe4ecd50d75f14,
    0x32df7737689b689a2a650bd773df7f43,
    0x28b2c5c5ed49207b551da312c319329c,
    0x208f049e576db395ddb14f4235adc217,
    0x34180763bf15ec22fc4ee536bc49368a,
    0x29acd2b63277f01bfd0bea92303a9208,
    0x21570ef8285ff349973cbba8269541a0,
    0x355817f373ccb875bec792a6a422029a,
    0x2aacdff5f63d605e3239421ee9b4cee1,
    0x2223e65e5e97804b5b6101b25490a581,
    0x369fd6fd64259a122bce691d541aa268,
    0x2bb31264501e14db563eba7ddce21b87,
    0x22f5a850401810af78322ecb171b4939,
    0x37ef73b399c01ab259e9e47824f87527,
    0x2cbf8fc2e1667bc1e187e9f9b72d2a86,
    0x23cc73024deb9634b46cbb2e2c242205,
    0x39471e6a1645bd2120adf849e039d007,
    0x2dd27ebb4504974db3be603b19c7d99f,
    0x24a865629d9d45d7c2feb3627b0647b3,
    0x3aa7089dc8fba2f2d197856a5e7072b8,
    0x2eec06e4a0c94f28a7ac6abb7ec05bc6,
    0x25899f1d4d6dd8ed52f05562cbcd1638,
    0x3c0f64fbaf1627e21e4d556adfae89f3,
    0x300c50c958de864e7ea444557fbed4c3,
    0x267040a113e5383ecbb69d1132ff109c,
    0x3d8067681fd526cadf8a94e851981a93,
    0x313385ece6441f08b2d543ed0e134875,
    0x275c6b23eb69b26d5bddcff0d80f6d2b,
    0x3efa45064575ea4892fc7fe7c018aeab,
    0x3261d0d1d12b21d3a8c9ffec99ad5889,
    0x284e40a7da88e7dc8707fff07af113a1,
    0x203e9a1fe2071fe39f39998d2f2742e7,
    0x33975cffd00b6638fec28f484b7204a4,
    0x2945e3ffd9a2b82d989ba5d36f8e6a1d,
    0x2104b66647b560247a161e42bfa521b1,
    0x34d4570a0c5566a0c35696d132a1cf81,
    0x2a4378d4d6aab8809c454574288172ce,
    0x21cf93dd7888939a169dd129ba0128a5,
    0x3618ec958da75290242fb50f9001daa1,
    0x2b4723aad7b90ed9b68c90d940017bb4,
    0x229f4fbbdfc73f14920a0d7a999ac95d,
    0x37654c5fcc71fe8750101590f5c47561,
    0x2c5109e63d27fed2a6734473f7d05de8,
    0x237407eb641fff0eeb8f69f65fd9e4b9,
    0x38b9a6456cfffe7e45b24323cc8fd45c,
    0x2d6151d123fffecb6af502830a0ca9e3,
    0x244ddb0db666656f88c402026e7087e9,
    0x3a162b4923d708b2746cd003e3e73fdb,
    0x2e7822a0e978d3c1f6bd73364fec3315,
    0x252ce880bac70fce5efdf5c50cbcf5ab,
    0x3b7b0d9ac471b2e3cb2fefa1adfb22ab,
    0x2f95a47bd05af58308f3261af195b555,
    0x261150630d159135a0c284e25ade2aab,
    0x3ce8809e7b55b5229ad0d49d5e304444,
    0x30ba007ec9115db548a7107de4f369d0,
    0x26fb3398a0dab15dd3b8d9fe50c2bb0d,
    0x3e5eb8f434911bc952c15cca1ad12b48,
    0x31e560c35d40e30775677d6e7bda8906,
    0x27eab3cf7dcd826c5dec645863153a6c,
    0x3fddec7f2faf3713c97a3a2704eec3df,
];

#[cfg(test)]
mod tests {
    use alloc::{format, string::String, vec, vec::Vec};

    use super::*;
    use crate::number::core_text;

    /// A little-endian unsigned integer of any size, for checking the table.
    struct Big(Vec<u32>);

    impl Big {
        fn one() -> Self {
            Self(vec![1])
        }

        fn mul_small(&mut self, m: u32) {
            let mut carry = 0u64;
            for limb in &mut self.0 {
                let v = u64::from(*limb) * u64::from(m) + carry;
                *limb = v as u32;
                carry = v >> 32;
            }
            if carry != 0 {
                self.0.push(carry as u32);
            }
        }

        fn div_small(&mut self, d: u32) {
            let mut rem = 0u64;
            for limb in self.0.iter_mut().rev() {
                let v = (rem << 32) | u64::from(*limb);
                *limb = (v / u64::from(d)) as u32;
                rem = v % u64::from(d);
            }
            while self.0.len() > 1 && self.0.last() == Some(&0) {
                let _ = self.0.pop();
            }
        }

        fn to_u128(&self) -> Option<u128> {
            if self.0.len() > 4 {
                return None;
            }
            Some(
                self.0
                    .iter()
                    .rev()
                    .fold(0u128, |acc, &limb| (acc << 32) | u128::from(limb)),
            )
        }
    }

    #[test]
    fn test_every_power_of_ten_is_exact() {
        for (i, &entry) in POWERS.iter().enumerate() {
            let k = K_MIN + i as i32;
            let e = -k;
            let r = flog2_pow10(e) - 125;
            // floor(10^e / 2^r), then + 1: multiply by every factor first,
            // then divide, so the floors are of the exact quotient.
            let mut n = Big::one();
            for _ in 0..e.max(0) {
                n.mul_small(10);
            }
            for _ in 0..(-r).max(0) {
                n.mul_small(2);
            }
            for _ in 0..r.max(0) {
                n.div_small(2);
            }
            for _ in 0..(-e).max(0) {
                n.div_small(10);
            }
            let g = n.to_u128().unwrap() + 1;
            assert_eq!(entry, g, "k = {k}");
            assert!(g > 1 << 125 && g <= 1 << 126, "k = {k}");
        }
        assert_eq!(POWERS.len(), 617);
    }

    #[test]
    fn test_logarithm_approximations() {
        let log10_2 = core::f64::consts::LOG10_2;
        let log2_10 = core::f64::consts::LOG2_10;
        let log10_3_4 = -0.124_938_736_608_299_95_f64;
        for e in -1200..=1200 {
            let ef = f64::from(e);
            for (got, exact) in [
                (flog10_pow2(e), ef * log10_2),
                (flog10_three_quarters_pow2(e), ef * log10_2 + log10_3_4),
                (flog2_pow10(e), ef * log2_10),
            ] {
                // Far enough from an integer for f64 to decide the floor.
                let gap = (exact - exact.round()).abs();
                if gap > 1e-9 {
                    assert_eq!(got, exact.floor() as i32, "e = {e}");
                } else {
                    assert_eq!(e, 0, "e = {e}");
                }
            }
        }
        assert_eq!(flog10_pow2(0), 0);
        assert_eq!(flog2_pow10(0), 0);
        assert_eq!(flog10_three_quarters_pow2(0), -1);
    }

    fn ours64(x: f64, ties: Ties, plus: bool) -> String {
        let mut out = String::new();
        write_decimal(&mut out, x.is_sign_negative(), f64_decimal(x, ties), plus);
        out
    }

    fn ours32(x: f32, ties: Ties) -> String {
        let mut out = String::new();
        write_decimal(&mut out, x.is_sign_negative(), f32_decimal(x, ties), false);
        out
    }

    fn check64(x: f64) {
        if x.is_finite() {
            assert_eq!(
                ours64(x, Ties::Up, false),
                core_text(&format!("{x:e}")),
                "{x:e}"
            );
        }
    }

    fn check32(x: f32) {
        if x.is_finite() {
            assert_eq!(ours32(x, Ties::Up), core_text(&format!("{x:e}")), "{x:e}");
        }
    }

    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// A 53-bit integer scaled by a power of two: every significand bit is
    /// used, so the last ones decide the digits, which is where exact ties
    /// between two shortest texts are.
    fn full_width(r: u64) -> f64 {
        let int = (r >> 11) | (1 << 52);
        let scale = (r % 120) as i32 - 100;
        (int as f64) * 2f64.powi(scale)
    }

    #[test]
    fn test_f64_matches_core_on_random_bits() {
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        for _ in 0..300_000 {
            check64(f64::from_bits(rng.next()));
        }
    }

    #[test]
    fn test_f64_matches_core_where_ties_are() {
        let mut rng = XorShift(0x2545_F491_4F6C_DD1D);
        let mut ties = 0;
        for _ in 0..300_000 {
            let x = full_width(rng.next());
            check64(x);
            if f64_decimal(x, Ties::Up) != f64_decimal(x, Ties::Even) {
                ties += 1;
            }
        }
        // The sweep must meet ties for the comparison to mean anything.
        assert!(ties > 100, "only {ties} ties met");
    }

    #[test]
    fn test_f64_every_exponent_and_boundary() {
        let mut rng = XorShift(0x0123_4567_89AB_CDEF);
        for biased in 0..2047u64 {
            for fraction in [0, 1, 2, 3, (1 << 52) - 1, (1 << 52) - 2, 1 << 51] {
                let x = f64::from_bits((biased << 52) | fraction);
                check64(x);
                check64(-x);
            }
            for _ in 0..20 {
                check64(f64::from_bits((biased << 52) | (rng.next() >> 12)));
            }
        }
        // The smallest subnormals, one by one.
        for fraction in 1..20_000u64 {
            check64(f64::from_bits(fraction));
        }
        // Powers of ten and their neighbours; integers around 2^53.
        for e in -325..=310 {
            let x: f64 = format!("1e{e}").parse().unwrap();
            for bits in [x.to_bits().saturating_sub(1), x.to_bits(), x.to_bits() + 1] {
                check64(f64::from_bits(bits));
            }
        }
        for n in (1u64 << 53) - 1000..(1 << 53) + 1000 {
            check64(n as f64);
        }
        for x in [
            0.0,
            -0.0,
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::MIN,
            f64::EPSILON,
            0.1,
            0.3,
            1.0 / 3.0,
        ] {
            check64(x);
        }
    }

    #[test]
    fn test_f32_matches_core() {
        let mut rng = XorShift(0x5DEE_CE66_D1CE_4E5B);
        for _ in 0..300_000 {
            check32(f32::from_bits(rng.next() as u32));
        }
        for biased in 0..255u32 {
            for fraction in [0, 1, 2, 3, (1 << 23) - 1, (1 << 23) - 2, 1 << 22] {
                let x = f32::from_bits((biased << 23) | fraction);
                check32(x);
                check32(-x);
            }
            for _ in 0..200 {
                check32(f32::from_bits((biased << 23) | (rng.next() as u32 >> 9)));
            }
        }
        for fraction in 1..100_000u32 {
            check32(f32::from_bits(fraction));
        }
    }

    /// Whether `digits × 10^power` is exactly `odd × 2^two` (`odd` odd), in
    /// integer arithmetic; `None` if the numbers do not fit.
    fn exactly(digits: u64, power: i32, odd: u64, two: i32) -> Option<bool> {
        let twos = digits.trailing_zeros() as i32;
        let digits = u128::from(digits >> twos);
        let fives = 5u128.checked_pow(power.unsigned_abs())?;
        let odd_parts = if power >= 0 {
            digits.checked_mul(fives)? == u128::from(odd)
        } else {
            u128::from(odd).checked_mul(fives)? == digits
        };
        Some(odd_parts && twos + power == two)
    }

    /// Where the two tie rules differ, the float is exactly halfway between
    /// the two texts, the even one is one unit below, and both read back.
    #[test]
    fn test_even_ties_differ_only_on_exact_ties() {
        let mut rng = XorShift(0x6A09_E667_F3BC_C909);
        let mut checked = 0;
        for i in 0..400_000 {
            let r = rng.next();
            let x = if i % 2 == 0 {
                full_width(r)
            } else {
                f64::from(r as u32) / 1024.0
            };
            let up = f64_decimal(x, Ties::Up);
            let even = f64_decimal(x, Ties::Even);
            if up == even {
                continue;
            }
            assert_eq!(up.digits % 2, 1, "{x:e}");
            assert_eq!(even, strip(up.digits - 1, up.exponent), "{x:e}");
            let text = ours64(x, Ties::Even, false);
            assert_eq!(
                text.parse::<f64>().map(f64::to_bits),
                Ok(x.to_bits()),
                "{text}"
            );
            // x = (2·up - 1) × 10^e / 2, exactly.
            let bits = x.to_bits();
            let significand = (bits & ((1 << 52) - 1)) | (1 << 52);
            let exp2 = ((bits >> 52) & 0x7ff) as i32 - 1075;
            let shift = significand.trailing_zeros();
            let halfway = exactly(
                2 * up.digits - 1,
                up.exponent,
                significand >> shift,
                exp2 + shift as i32 + 1,
            );
            assert_ne!(halfway, Some(false), "{x:e}");
            if halfway == Some(true) {
                checked += 1;
            }
        }
        assert!(checked > 100, "only {checked} ties checked");
    }

    #[test]
    fn test_layouts() {
        for (x, want) in [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (-1.5, "-1.5"),
            (100.0, "100.0"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e-5, "0.00001"),
            (1.25e-5, "0.0000125"),
            (1e-6, "1e-6"),
            (-2.5e-7, "-2.5e-7"),
            (1e15, "1000000000000000.0"),
            (1.5e15, "1500000000000000.0"),
            (1e16, "1e16"),
            (1.5e16, "1.5e16"),
            (123_456_789_012_345.67, "123456789012345.67"),
            (f64::MAX, "1.7976931348623157e308"),
            (f64::MIN_POSITIVE, "2.2250738585072014e-308"),
            (5e-324, "5e-324"),
        ] {
            assert_eq!(ours64(x, Ties::Up, false), want, "{x:e}");
        }
        assert_eq!(ours64(1e16, Ties::Up, true), "1e+16");
        assert_eq!(ours64(-1.5e300, Ties::Up, true), "-1.5e+300");
        assert_eq!(ours64(1e-7, Ties::Up, true), "1e-7");
        assert_eq!(ours64(1e15, Ties::Up, true), "1000000000000000.0");
        assert_eq!(
            ours64(164_696_460_098_373.0 + 0.625, Ties::Up, false),
            "164696460098373.63"
        );
        assert_eq!(
            ours64(164_696_460_098_373.0 + 0.625, Ties::Even, false),
            "164696460098373.62"
        );
        assert_eq!(
            ours64(140_737_488_355_328.0 + 0.125, Ties::Even, false),
            "140737488355328.12"
        );
        assert_eq!(
            ours64(140_737_488_355_328.0 + 0.875, Ties::Even, false),
            "140737488355328.88"
        );
        assert_eq!(ours32(0.1, Ties::Up), "0.1");
        assert_eq!(ours32(1e20, Ties::Up), "1e20");
        assert_eq!(ours32(f32::MAX, Ties::Up), "3.4028235e38");
        assert_eq!(ours32(f32::from_bits(1), Ties::Up), "1e-45");
    }

    #[test]
    fn test_put_digits() {
        let mut buf = [0u8; 24];
        for n in [
            0u64,
            1,
            9,
            10,
            99,
            100,
            101,
            12_345_678,
            99_999_999,
            100_000_000,
            123_456_789_012,
            u64::MAX,
        ] {
            let start = put_digits(&mut buf, 24, n);
            assert_eq!(buf.get(start..), Some(format!("{n}").as_bytes()), "{n}");
        }
    }

    /// A billion `f64`s against `core`, half random bit patterns, half
    /// full-width significands: run on demand, in release (`cargo test
    /// --release --lib -- --ignored many_f64`).
    #[test]
    #[ignore = "takes minutes; run on demand"]
    fn test_many_f64() {
        #[cfg(feature = "std")]
        {
            let threads = std::thread::available_parallelism().map_or(4, usize::from) as u64;
            let per_thread = 1_000_000_000 / threads;
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    std::thread::spawn(move || {
                        let mut rng = XorShift(
                            0x9E37_79B9_7F4A_7C15 ^ (t + 1).wrapping_mul(0xD1B5_4A32_D192_ED03),
                        );
                        for i in 0..per_thread {
                            let r = rng.next();
                            check64(if i % 2 == 0 {
                                f64::from_bits(r)
                            } else {
                                full_width(r)
                            });
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
        }
    }

    /// Every `f32`, against `core`: about four billion values, so run it on
    /// demand, in release (`cargo test --release --lib -- --ignored
    /// every_f32`).
    #[test]
    #[ignore = "takes minutes; run on demand"]
    fn test_every_f32() {
        #[cfg(feature = "std")]
        {
            let threads = std::thread::available_parallelism().map_or(4, usize::from) as u64;
            let span = (1u64 << 32) / threads;
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    std::thread::spawn(move || {
                        let end = if t + 1 == threads {
                            1 << 32
                        } else {
                            (t + 1) * span
                        };
                        for bits in t * span..end {
                            check32(f32::from_bits(bits as u32));
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
        }
    }
}
