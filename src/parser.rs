//! The configured parser and the tree builder.

use alloc::{borrow::Cow, vec::Vec};
use core::mem::size_of;

use crate::{
    Array, Limits, Object, Reader, Value, dupes,
    error::{Error, ErrorKind},
    reader::Event,
    tree::Member,
};

/// What to do when an object names the same key twice.
///
/// RFC 8259 says names within an object SHOULD be unique and leaves the
/// behaviour for duplicates to the implementation; implementations disagree,
/// which makes duplicates a classic way to smuggle a different value past one
/// reader than another sees. The default is therefore to refuse them.
/// Keys are compared after unescaping, so `"a"` and `"\u0061"` are the same
/// key.
///
/// The policy applies to every way of reading: [`Parser::parse`], and the
/// [`Reader`]s it makes, whose events and raw slices refuse a duplicate
/// under `Error`. Under `First`, `Last`, and `KeepAll` a reader's events
/// carry every member as written; the values a parse or
/// [`Reader::read_value`] builds apply the policy.
///
/// The enum is `#[non_exhaustive]`: a later version may add a policy.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{DuplicateKeys, ErrorKind, Parser};
///
/// let text = r#"{"a": 1, "b": 2, "a": 3}"#;
///
/// let err = Parser::new().parse(text).unwrap_err();
/// assert_eq!((err.kind(), err.offset()), (ErrorKind::DuplicateKey, 17));
///
/// let first = Parser::new().duplicate_keys(DuplicateKeys::First).parse(text)?;
/// assert_eq!(first.to_string(), r#"{"a":1,"b":2}"#);
///
/// let last = Parser::new().duplicate_keys(DuplicateKeys::Last).parse(text)?;
/// assert_eq!(last.to_string(), r#"{"a":3,"b":2}"#);
///
/// let all = Parser::new().duplicate_keys(DuplicateKeys::KeepAll).parse(text)?;
/// assert_eq!(all.to_string(), r#"{"a":1,"b":2,"a":3}"#);
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DuplicateKeys {
    /// Refuse the document with [`ErrorKind::DuplicateKey`] at the second
    /// occurrence's opening quote. The default.
    #[default]
    Error,
    /// Keep the first occurrence and its value; drop the later ones.
    First,
    /// Keep the last occurrence's value, at the first occurrence's position,
    /// as inserting each member into an ordered map would.
    Last,
    /// Keep every member as written. The writer reproduces the duplicates.
    KeepAll,
}

/// How typed reads ([`FromJson`](crate::FromJson)) of integers treat a
/// number written as a fraction or with an exponent.
///
/// JSON has one number type, and writers differ on how they spell a whole
/// number: some write `100`, some `100.0` or `1e2`. The default reads an
/// integer type only from integer text, as [`Number::as_i64`](crate::Number::as_i64)
/// does, so a value that was meant as a float is never silently taken as an
/// integer. `WholeValues` also accepts any spelling of a whole number, read
/// exactly ([`Number::whole_value`](crate::Number::whole_value)); a
/// fractional value is still refused.
///
/// The enum is `#[non_exhaustive]`.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{Numbers, Parser};
///
/// assert!(Parser::new().decode::<u32>("1e2").is_err());
/// let lenient = Parser::new().numbers(Numbers::WholeValues);
/// assert_eq!(lenient.decode::<u32>("1e2")?, 100);
/// assert_eq!(lenient.decode::<u32>("7.0")?, 7);
/// assert!(lenient.decode::<u32>("7.5").is_err());
/// # Ok::<(), jsonx_lib::DecodeError>(())
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Numbers {
    /// Integers from integer text only (`100`); `1e2` and `100.0` are
    /// refused for an integer type. The default.
    #[default]
    IntegerText,
    /// Integers from any spelling of a whole number (`100`, `1e2`,
    /// `100.0`), within `i128`'s range for exponent or fraction forms.
    WholeValues,
}

/// A configured parser: [`Limits`] and a [`DuplicateKeys`] policy.
///
/// [`parse`](crate::parse) is `Parser::new().parse`. Build a `Parser` to
/// change a budget or the duplicate-key policy; it is a small `Copy` value
/// that can be kept in a `const` or a struct and reused for any number of
/// documents.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{DuplicateKeys, ErrorKind, Limits, Parser};
///
/// // Messages from the network: small and shallow.
/// const WIRE: Parser = Parser::new()
///     .limits(Limits::new().with_max_input_bytes(1 << 20).with_max_depth(32))
///     .duplicate_keys(DuplicateKeys::Error);
///
/// assert!(WIRE.parse(r#"{"ok": true}"#).is_ok());
/// let deep = "[".repeat(33);
/// assert_eq!(WIRE.parse(&deep).unwrap_err().kind(), ErrorKind::DepthLimitExceeded);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[must_use]
pub struct Parser {
    limits: Limits,
    duplicates: DuplicateKeys,
    numbers: Numbers,
}

impl Parser {
    /// A parser with the default [`Limits`] and [`DuplicateKeys::Error`].
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::Parser::new().parse("[]")?.to_string(), "[]");
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub const fn new() -> Self {
        Self {
            limits: Limits::new(),
            duplicates: DuplicateKeys::Error,
            numbers: Numbers::IntegerText,
        }
    }

    /// Replaces the budgets.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Limits, Parser};
    ///
    /// let p = Parser::new().limits(Limits::new().with_max_string_bytes(3));
    /// assert_eq!(p.parse(r#""four""#).unwrap_err().kind(), ErrorKind::StringTooLong);
    /// ```
    pub const fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Sets the duplicate-key policy.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{DuplicateKeys, Parser};
    ///
    /// let p = Parser::new().duplicate_keys(DuplicateKeys::Last);
    /// assert_eq!(p.parse(r#"{"k": 1, "k": 2}"#)?["k"], 2);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub const fn duplicate_keys(mut self, policy: DuplicateKeys) -> Self {
        self.duplicates = policy;
        self
    }

    /// Sets how typed reads take integers; see [`Numbers`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Numbers, Parser};
    ///
    /// let p = Parser::new().numbers(Numbers::WholeValues);
    /// assert_eq!(p.get_numbers(), Numbers::WholeValues);
    /// ```
    pub const fn numbers(mut self, numbers: Numbers) -> Self {
        self.numbers = numbers;
        self
    }

    /// The parser's [`Numbers`] setting.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::Parser::new().get_numbers(), jsonx_lib::Numbers::IntegerText);
    /// ```
    #[must_use]
    pub const fn get_numbers(&self) -> Numbers {
        self.numbers
    }

    /// The parser's budgets.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::Parser::new().get_limits().max_depth, 128);
    /// ```
    #[must_use]
    pub const fn get_limits(&self) -> Limits {
        self.limits
    }

    /// The parser's duplicate-key policy.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{DuplicateKeys, Parser};
    ///
    /// assert_eq!(Parser::new().get_duplicate_keys(), DuplicateKeys::Error);
    /// ```
    #[must_use]
    pub const fn get_duplicate_keys(&self) -> DuplicateKeys {
        self.duplicates
    }

    /// Parses one JSON document.
    ///
    /// The whole input must be exactly one value, with optional whitespace
    /// around it (space, tab, line feed, carriage return; nothing else, not
    /// even a byte-order mark). Strings without escapes and all numbers borrow
    /// from `text`.
    ///
    /// # Errors
    ///
    /// The first syntax error or exceeded budget, at its byte offset; see
    /// [`ErrorKind`] for every case.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Parser};
    ///
    /// let v = Parser::new().parse(" [1, \"two\"] ")?;
    /// assert_eq!(v[1], "two");
    ///
    /// let err = Parser::new().parse("[1] [2]").unwrap_err();
    /// assert_eq!((err.kind(), err.offset()), (ErrorKind::TrailingCharacters, 4));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn parse<'a>(&self, text: &'a str) -> Result<Value<'a>, Error> {
        let mut reader = self.reader(text);
        let value = reader
            .read_value()?
            .ok_or(Error::new(ErrorKind::UnexpectedEnd, text.len()))?;
        reader.finish()?;
        Ok(value)
    }

    /// Parses one JSON document from bytes, which must be UTF-8 (RFC 8259
    /// §8.1). The size budget is checked first, then the encoding, then the
    /// syntax.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InputTooLarge`], then [`ErrorKind::InvalidUtf8`] at the
    /// first byte of the first invalid sequence, then as
    /// [`parse`](Self::parse).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Parser};
    ///
    /// assert_eq!(Parser::new().parse_bytes(b"[\"\xC3\xA9\"]")?[0], "\u{e9}");
    /// let err = Parser::new().parse_bytes(b"[\"\xC3\"]").unwrap_err();
    /// assert_eq!((err.kind(), err.offset()), (ErrorKind::InvalidUtf8, 2));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn parse_bytes<'a>(&self, bytes: &'a [u8]) -> Result<Value<'a>, Error> {
        self.parse(self.check_bytes(bytes)?)
    }

    /// A [`Reader`] over `text` with this parser's budgets and duplicate-key
    /// policy (which [`Reader::read_value`] applies).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Limits, Parser};
    ///
    /// let mut r = Parser::new().limits(Limits::new().with_max_depth(1)).reader("[[]]");
    /// assert!(r.next_event().is_ok());
    /// assert_eq!(r.next_event().unwrap_err().kind(), ErrorKind::DepthLimitExceeded);
    /// ```
    #[must_use]
    pub fn reader<'a>(&self, text: &'a str) -> Reader<'a> {
        Reader::with_config(text, self.limits, self.duplicates).with_numbers(self.numbers)
    }

    /// A [`Reader`] over bytes, which must be UTF-8. The size budget and the
    /// encoding are checked here, before any event is read.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InputTooLarge`] or [`ErrorKind::InvalidUtf8`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Event, Parser};
    ///
    /// let mut r = Parser::new().reader_bytes(b"null")?;
    /// assert_eq!(r.next_event()?, Some(Event::Null));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn reader_bytes<'a>(&self, bytes: &'a [u8]) -> Result<Reader<'a>, Error> {
        Ok(self.reader(self.check_bytes(bytes)?))
    }

    fn check_bytes<'a>(&self, bytes: &'a [u8]) -> Result<&'a str, Error> {
        if bytes.len() > self.limits.max_input_bytes {
            return Err(Error::new(
                ErrorKind::InputTooLarge,
                self.limits.max_input_bytes,
            ));
        }
        core::str::from_utf8(bytes).map_err(|e| Error::new(ErrorKind::InvalidUtf8, e.valid_up_to()))
    }
}

/// An open container in the builder: where its children start on the shared
/// value and key stacks, and where it starts in the input.
#[derive(Clone, Copy)]
struct Frame {
    values: usize,
    keys: usize,
    start: usize,
}

/// The heap a build holds, against [`Limits::max_memory`].
struct Budget {
    max: usize,
    /// The finished arrays and objects, and decoded strings.
    held: usize,
}

impl Budget {
    /// Refuses the value at `at` if `extra` more bytes, on top of what is
    /// held and the stacks' `stacks` bytes, would pass the budget.
    #[inline]
    fn allow(&self, stacks: usize, extra: usize, at: usize) -> Result<(), Error> {
        if self.held.saturating_add(stacks).saturating_add(extra) > self.max {
            Err(Error::new(ErrorKind::MemoryLimitExceeded, at))
        } else {
            Ok(())
        }
    }
}

/// The bytes the builder's stacks hold, at their capacity.
#[inline]
fn stacks(values: &Vec<Value<'_>>, keys: &Vec<Cow<'_, str>>) -> usize {
    values.capacity() * size_of::<Value<'_>>() + keys.capacity() * size_of::<Cow<'_, str>>()
}

/// The bytes one more push onto `v` would allocate: none while it has room,
/// otherwise its whole new buffer (vectors double, from four). The old
/// buffer is still counted in [`stacks`] until the move, so the check covers
/// the moment both exist.
#[inline]
fn growth<T>(v: &Vec<T>) -> usize {
    if v.len() < v.capacity() {
        0
    } else {
        (v.capacity() * 2).max(4) * size_of::<T>()
    }
}

/// Buffers reused by every object's duplicate check in one build.
#[derive(Default)]
struct Scratch {
    order: Vec<usize>,
    keep: Vec<bool>,
}

/// Builds the value whose first event comes next from `reader`.
///
/// Children of every open container sit on one shared value stack; when a
/// container closes, its children are moved off the top into a vector of
/// exactly their size, except the outermost array, which takes the stack
/// itself. Nothing recurses, and no container's vector ever grows by
/// reallocation. What the build holds is charged against `max_memory`
/// before it is allocated (see [`Limits`]).
pub(crate) fn build<'a>(
    reader: &mut Reader<'a>,
    policy: DuplicateKeys,
    max_memory: usize,
) -> Result<Value<'a>, Error> {
    let mut values: Vec<Value<'a>> = Vec::new();
    let mut keys: Vec<Cow<'a, str>> = Vec::new();
    let mut frames: Vec<Frame> = Vec::new();
    let mut scratch = Scratch::default();
    let mut budget = Budget {
        max: max_memory,
        held: 0,
    };
    loop {
        let Some(event) = reader.next_event()? else {
            // The reader ends a document only after a complete value.
            return Err(Error::new(ErrorKind::UnexpectedEnd, reader.offset()));
        };
        let at = reader.event_offset();
        let value = match event {
            Event::StartArray | Event::StartObject => {
                frames.push(Frame {
                    values: values.len(),
                    keys: keys.len(),
                    start: at,
                });
                continue;
            }
            Event::Key(key) => {
                budget.allow(stacks(&values, &keys), growth(&keys), at)?;
                if let Cow::Owned(decoded) = &key {
                    budget.held += decoded.capacity();
                }
                keys.push(key);
                continue;
            }
            Event::EndArray => {
                let Some(frame) = frames.pop() else {
                    return Err(Error::new(ErrorKind::ExpectedValue, at));
                };
                if frames.is_empty() && frame.values == 0 {
                    // The outermost array: its items are the whole stack,
                    // which becomes the array in place, already counted.
                    let mut items = core::mem::take(&mut values);
                    items.shrink_to_fit();
                    return Ok(Value::Array(Array::from(items)));
                }
                let bytes = (values.len() - frame.values) * size_of::<Value<'a>>();
                budget.allow(stacks(&values, &keys), bytes, frame.start)?;
                budget.held += bytes;
                Value::Array(Array::from(values.split_off(frame.values)))
            }
            Event::EndObject => {
                let Some(frame) = frames.pop() else {
                    return Err(Error::new(ErrorKind::ExpectedValue, at));
                };
                let bytes = (keys.len() - frame.keys) * size_of::<Member<'a>>();
                budget.allow(stacks(&values, &keys), bytes, frame.start)?;
                budget.held += bytes;
                Value::Object(close_object(
                    &mut values,
                    &mut keys,
                    frame,
                    policy,
                    &mut scratch,
                ))
            }
            Event::String(s) => {
                if let Cow::Owned(decoded) = &s {
                    budget.held += decoded.capacity();
                    budget.allow(stacks(&values, &keys), 0, at)?;
                }
                Value::String(s)
            }
            Event::Number(n) => Value::Number(n),
            Event::Bool(b) => Value::Bool(b),
            Event::Null => Value::Null,
        };
        if frames.is_empty() {
            return Ok(value);
        }
        let start = match &value {
            Value::Array(_) | Value::Object(_) => reader.event_offset(),
            _ => at,
        };
        budget.allow(stacks(&values, &keys), growth(&values), start)?;
        values.push(value);
    }
}

#[inline]
fn key_at<'k>(keys: &'k [Cow<'_, str>], i: usize) -> &'k str {
    keys.get(i).map_or("", |k| k)
}

/// Moves the closing object's members off the stacks, applying `policy`.
/// Under [`DuplicateKeys::Error`] the reader has already refused any
/// duplicate (when the object closed), so only `First` and `Last` have
/// anything to do here.
fn close_object<'a>(
    values: &mut Vec<Value<'a>>,
    keys: &mut Vec<Cow<'a, str>>,
    frame: Frame,
    policy: DuplicateKeys,
    scratch: &mut Scratch,
) -> Object<'a> {
    let member_keys = keys.get(frame.keys..).unwrap_or(&[]);
    let key = |i| key_at(member_keys, i);
    let dedupe = matches!(policy, DuplicateKeys::First | DuplicateKeys::Last)
        && dupes::has_duplicates(member_keys.len(), &key, &mut scratch.order);
    if dedupe {
        mark_duplicates(member_keys, policy, scratch, values, frame.values);
    }
    let pairs = keys.drain(frame.keys..).zip(values.drain(frame.values..));
    let members: Vec<Member<'a>> = if dedupe {
        // Sized exactly up front: a filtered iterator's size is unknown, and
        // collecting it would grow the vector by reallocation.
        let kept = scratch.keep.iter().filter(|&&keep| keep).count();
        let mut members = Vec::with_capacity(kept);
        members.extend(
            pairs
                .zip(scratch.keep.iter())
                .filter_map(|(member, &keep)| keep.then_some(member)),
        );
        members
    } else {
        pairs.collect()
    };
    Object::from(members)
}

/// Fills `scratch.keep` with which members survive the policy, and for
/// [`DuplicateKeys::Last`] moves each repeated key's last value to its first
/// position. `scratch.order` holds the keys' sorted order.
fn mark_duplicates(
    keys: &[Cow<'_, str>],
    policy: DuplicateKeys,
    scratch: &mut Scratch,
    values: &mut [Value<'_>],
    base: usize,
) {
    let keep = &mut scratch.keep;
    keep.clear();
    keep.resize(keys.len(), true);
    let key = |i| key_at(keys, i);
    dupes::for_each_run(&key, &scratch.order, |run| {
        let (Some(&first), Some(&last)) = (run.first(), run.last()) else {
            return;
        };
        if policy == DuplicateKeys::Last {
            values.swap(base + first, base + last);
        }
        for &i in run.get(1..).unwrap_or(&[]) {
            if let Some(slot) = keep.get_mut(i) {
                *slot = false;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duplicate_detection_small_and_large() {
        let small = r#"{"a":1,"b":2,"c":3}"#;
        assert!(Parser::new().parse(small).is_ok());
        let mut large = alloc::string::String::from("{");
        for i in 0..100 {
            large.push_str(&alloc::format!("\"k{i}\":{i},"));
        }
        large.push_str("\"k50\":0,\"k7\":0}");
        let err = Parser::new().parse(&large).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DuplicateKey);
        assert_eq!(large.get(err.offset()..err.offset() + 6), Some("\"k50\":"));
        let last = Parser::new()
            .duplicate_keys(DuplicateKeys::Last)
            .parse(&large);
        let last = last.unwrap_or_default();
        assert_eq!(last.as_object().map(Object::len), Some(100));
        assert_eq!(last["k50"], 0);
        assert_eq!(last["k7"], 0);
        assert_eq!(last.as_object().and_then(|o| o.keys().nth(50)), Some("k50"));
        let first = Parser::new()
            .duplicate_keys(DuplicateKeys::First)
            .parse(&large);
        assert_eq!(first.unwrap_or_default()["k50"], 50);
    }

    #[test]
    fn test_duplicates_compare_decoded_keys() {
        let err = Parser::new().parse(r#"{"a":1,"\u0061":2}"#).unwrap_err();
        assert_eq!((err.kind(), err.offset()), (ErrorKind::DuplicateKey, 7));
    }

    proptest::proptest! {
        /// Every policy matches a naive quadratic reference, across the
        /// pairwise and sorting paths, with keys that share long prefixes
        /// (equal fingerprints, different strings).
        #[test]
        fn prop_duplicate_policies_match_naive(picks in proptest::collection::vec(0usize..12, 0..40)) {
            use alloc::{format, string::String, vec::Vec};
            const KEYS: [&str; 12] = [
                "a", "b", "abcdefgh1", "abcdefgh2", "abcdefgh12", "abcdefghX", "", "k",
                "abcdefgh", "abcdefgi", "zz", "abcdefgh1\\u0000",
            ];
            let decoded = |k: &str| k.replace("\\u0000", "\u{0}");
            let mut text = String::from("{");
            let mut offsets = Vec::new();
            for (i, &p) in picks.iter().enumerate() {
                if i > 0 { text.push(','); }
                offsets.push(text.len());
                text.push_str(&format!("\"{}\":{i}", KEYS[p]));
            }
            text.push('}');
            let names: Vec<String> = picks.iter().map(|&p| decoded(KEYS[p])).collect();

            // Error: the earliest member whose key appeared before.
            let repeat = (0..names.len()).find(|&i| names[..i].contains(&names[i]));
            let got = Parser::new().parse(&text).err().map(|e| (e.kind(), e.offset()));
            proptest::prop_assert_eq!(got, repeat.map(|i| (ErrorKind::DuplicateKey, offsets[i])));

            // First and Last: one member per key, at its first position.
            let mut order: Vec<&String> = Vec::new();
            for n in &names { if !order.contains(&n) { order.push(n); } }
            let first = Parser::new().duplicate_keys(DuplicateKeys::First).parse(&text).unwrap_or_default();
            let last = Parser::new().duplicate_keys(DuplicateKeys::Last).parse(&text).unwrap_or_default();
            let (fo, lo) = (first.as_object().cloned().unwrap_or_default(), last.as_object().cloned().unwrap_or_default());
            proptest::prop_assert_eq!(fo.keys().collect::<Vec<_>>(), order.iter().map(|s| s.as_str()).collect::<Vec<_>>());
            proptest::prop_assert_eq!(lo.keys().collect::<Vec<_>>(), order.iter().map(|s| s.as_str()).collect::<Vec<_>>());
            for key in &order {
                let want_first = names.iter().position(|n| n == *key);
                let want_last = names.iter().rposition(|n| n == *key);
                proptest::prop_assert_eq!(fo.get(key).and_then(Value::as_u64), want_first.map(|i| i as u64));
                proptest::prop_assert_eq!(lo.get(key).and_then(Value::as_u64), want_last.map(|i| i as u64));
            }
        }
    }

    #[test]
    fn test_duplicate_reported_at_earliest_repeat() {
        // `b` repeats before `a` does.
        let err = Parser::new()
            .parse(r#"{"a":1,"b":2,"b":3,"a":4}"#)
            .unwrap_err();
        assert_eq!(err.offset(), 13);
    }
}
