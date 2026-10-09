//! The document model.

use alloc::{
    borrow::Cow,
    string::{String, ToString},
    vec::Vec,
};
use core::{fmt, ops::Index};

use crate::{Array, Number, Object, tree};

/// The `null` that indexing returns for a missing member or item.
pub(crate) static NULL: Value<'static> = Value::Null;

/// A JSON value, borrowing from the text it was parsed from.
///
/// Strings without escape sequences borrow their bytes from the input
/// (`Cow::Borrowed`); only a string that had to be unescaped owns a copy.
/// Numbers keep their exact text (see [`Number`]). Object members keep their
/// document order. The lifetime `'a` is the input's; [`into_owned`](Self::into_owned)
/// detaches a value from it, and a value built in code from `'static` data is
/// a `Value<'static>`.
///
/// Every operation on a `Value` (drop, clone, comparison, writing) uses a
/// bounded amount of stack whatever the depth, so a value of any depth is safe
/// to use.
///
/// Equality is structural and exact: numbers compare by text (`1` and `1.0`
/// differ), and object members compare in order. A `Value` also compares
/// directly with Rust strings, booleans, and numbers, which is what
/// `assert_eq!(doc["id"], 7)` uses.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{parse, Value};
///
/// let doc = parse(r#"{"user": {"name": "ada", "langs": ["en", "fr"]}, "admin": false}"#)?;
///
/// // Index freely: a missing member or item reads as `null`.
/// assert_eq!(doc["user"]["name"], "ada");
/// assert_eq!(doc["user"]["langs"][1], "fr");
/// assert!(doc["nope"][3].is_null());
///
/// // Or ask with typed accessors that say whether the shape matched.
/// assert_eq!(doc["admin"].as_bool(), Some(false));
/// assert_eq!(doc.pointer("/user/langs/0").and_then(Value::as_str), Some("en"));
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Default)]
pub enum Value<'a> {
    /// `null`.
    #[default]
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, kept as its exact text.
    Number(Number<'a>),
    /// A string: borrowed from the input when it had no escapes, owned otherwise.
    String(Cow<'a, str>),
    /// An array.
    Array(Array<'a>),
    /// An object.
    Object(Object<'a>),
}

/// Which of JSON's six kinds of value a [`Value`] is.
///
/// Useful for error messages ("expected a string, found an array").
///
/// # Examples
///
/// ```
/// use jsonx_lib::{parse, Kind};
///
/// let doc = parse("[1]")?;
/// assert_eq!(doc.kind(), Kind::Array);
/// assert_eq!(doc[0].kind().to_string(), "number");
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool,
    /// A number.
    Number,
    /// A string.
    String,
    /// An array.
    Array,
    /// An object.
    Object,
}

impl Kind {
    /// The kind's name in lowercase: `null`, `boolean`, `number`, `string`,
    /// `array`, or `object`.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::Kind::Bool.name(), "boolean");
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool => "boolean",
            Self::Number => "number",
            Self::String => "string",
            Self::Array => "array",
            Self::Object => "object",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl<'a> Value<'a> {
    /// Which kind of value this is.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{json, Kind};
    ///
    /// assert_eq!(json!({}).kind(), Kind::Object);
    /// ```
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Self::Null => Kind::Null,
            Self::Bool(_) => Kind::Bool,
            Self::Number(_) => Kind::Number,
            Self::String(_) => Kind::String,
            Self::Array(_) => Kind::Array,
            Self::Object(_) => Kind::Object,
        }
    }

    /// Whether the value is `null`.
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(jsonx_lib::parse("null")?.is_null());
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The boolean, if the value is one.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::parse("true")?.as_bool(), Some(true));
    /// assert_eq!(jsonx_lib::parse("1")?.as_bool(), None);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The number, if the value is one.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse("6.02e23")?;
    /// assert_eq!(doc.as_number().map(|n| n.as_str()), Some("6.02e23"));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub const fn as_number(&self) -> Option<&Number<'a>> {
        match self {
            Self::Number(n) => Some(n),
            _ => None,
        }
    }

    /// The value as an `i64`: see [`Number::as_i64`]. `None` for anything but
    /// an integer number in range.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse("[-5, 5.5, \"5\"]")?;
    /// assert_eq!(doc[0].as_i64(), Some(-5));
    /// assert_eq!(doc[1].as_i64(), None);
    /// assert_eq!(doc[2].as_i64(), None);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        self.as_number().and_then(Number::as_i64)
    }

    /// The value as a `u64`: see [`Number::as_u64`].
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::parse("42")?.as_u64(), Some(42));
    /// assert_eq!(jsonx_lib::parse("-42")?.as_u64(), None);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        self.as_number().and_then(Number::as_u64)
    }

    /// The value as an `f64`: see [`Number::as_f64`]. Integers convert too.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::parse("2")?.as_f64(), Some(2.0));
    /// assert_eq!(jsonx_lib::parse("\"2\"")?.as_f64(), None);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        self.as_number().and_then(Number::as_f64)
    }

    /// The string, if the value is one.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::parse(r#""a\tb""#)?.as_str(), Some("a\tb"));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// The array, if the value is one.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse("[1, 2, 3]")?;
    /// assert_eq!(doc.as_array().map(|a| a.len()), Some(3));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub const fn as_array(&self) -> Option<&Array<'a>> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The array, mutably, if the value is one.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut v = jsonx_lib::json!([1]);
    /// v.as_array_mut().unwrap().push(2.into());
    /// assert_eq!(v.to_string(), "[1,2]");
    /// ```
    #[inline]
    #[must_use]
    pub fn as_array_mut(&mut self) -> Option<&mut Array<'a>> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The object, if the value is one.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse(r#"{"k": 1}"#)?;
    /// assert!(doc.as_object().is_some_and(|o| o.contains_key("k")));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub const fn as_object(&self) -> Option<&Object<'a>> {
        match self {
            Self::Object(o) => Some(o),
            _ => None,
        }
    }

    /// The object, mutably, if the value is one.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut v = jsonx_lib::json!({});
    /// v.as_object_mut().unwrap().insert("k", "v");
    /// assert_eq!(v.to_string(), r#"{"k":"v"}"#);
    /// ```
    #[inline]
    #[must_use]
    pub fn as_object_mut(&mut self) -> Option<&mut Object<'a>> {
        match self {
            Self::Object(o) => Some(o),
            _ => None,
        }
    }

    /// The value of the member named `key`, if this is an object that has one.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse(r#"{"jsonrpc": "2.0", "id": 1}"#)?;
    /// assert_eq!(doc.get("jsonrpc").and_then(|v| v.as_str()), Some("2.0"));
    /// assert!(doc.get("method").is_none());
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value<'a>> {
        self.as_object().and_then(|o| o.get(key))
    }

    /// A mutable reference to the member named `key`, if this is an object
    /// that has one.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut v = jsonx_lib::json!({ "n": 1 });
    /// if let Some(n) = v.get_mut("n") {
    ///     *n = 2.into();
    /// }
    /// assert_eq!(v["n"], 2);
    /// ```
    #[must_use]
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value<'a>> {
        self.as_object_mut().and_then(|o| o.get_mut(key))
    }

    /// The item at `index`, if this is an array that long.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse("[10, 20]")?;
    /// assert_eq!(doc.get_index(1).and_then(|v| v.as_i64()), Some(20));
    /// assert!(doc.get_index(2).is_none());
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn get_index(&self, index: usize) -> Option<&Value<'a>> {
        self.as_array().and_then(|a| a.get(index))
    }

    /// Looks a value up by [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901)
    /// JSON Pointer: `""` is the whole value, `/a/0` is item 0 of member `a`,
    /// and `~1` and `~0` stand for `/` and `~` inside a key.
    ///
    /// Returns `None` when the pointer is malformed (it does not start with
    /// `/`, or has a `~` not followed by `0` or `1`) or names something that
    /// is not there. An array index must be a plain decimal number without
    /// leading zeros; `-` (past the end) never resolves.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse(r#"{"a/b": {"c~d": [true]}}"#)?;
    /// assert_eq!(doc.pointer("/a~1b/c~0d/0"), Some(&true.into()));
    /// assert_eq!(doc.pointer(""), Some(&doc));
    /// assert!(doc.pointer("/a~1b/c~0d/01").is_none());
    /// assert!(doc.pointer("a").is_none());
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn pointer(&self, pointer: &str) -> Option<&Value<'a>> {
        if pointer.is_empty() {
            return Some(self);
        }
        let rest = pointer.strip_prefix('/')?;
        let mut target = self;
        for token in rest.split('/') {
            let token = unescape_pointer_token(token)?;
            target = match target {
                Self::Object(o) => o.get(&token)?,
                Self::Array(a) => a.get(array_index(&token)?)?,
                _ => return None,
            };
        }
        Some(target)
    }

    /// Takes the value out, leaving `null` in its place.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut v = jsonx_lib::json!({ "big": [1, 2, 3] });
    /// let big = v.get_mut("big").map(|b| b.take()).unwrap_or_default();
    /// assert_eq!(big.to_string(), "[1,2,3]");
    /// assert!(v["big"].is_null());
    /// ```
    #[inline]
    pub fn take(&mut self) -> Value<'a> {
        core::mem::take(self)
    }

    /// Copies every borrowed key, string, and number so the value no longer
    /// borrows from the input. Iterative at any depth.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{parse, Value};
    ///
    /// fn load() -> Result<Value<'static>, jsonx_lib::Error> {
    ///     let text = String::from(r#"{"kept": "after the text is gone"}"#);
    ///     Ok(parse(&text)?.into_owned())
    /// }
    /// assert_eq!(load()?["kept"], "after the text is gone");
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn into_owned(self) -> Value<'static> {
        tree::into_owned(self)
    }
}

/// Decodes `~1` and `~0` in one JSON Pointer reference token.
fn unescape_pointer_token(token: &str) -> Option<Cow<'_, str>> {
    if !token.contains('~') {
        return Some(Cow::Borrowed(token));
    }
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        if c == '~' {
            match chars.next() {
                Some('0') => out.push('~'),
                Some('1') => out.push('/'),
                _ => return None,
            }
        } else {
            out.push(c);
        }
    }
    Some(Cow::Owned(out))
}

/// An RFC 6901 array index: `0`, or a decimal without a leading zero.
fn array_index(token: &str) -> Option<usize> {
    let digits = token.as_bytes();
    let canonical = !digits.is_empty()
        && digits.iter().all(u8::is_ascii_digit)
        && (digits.len() == 1 || digits.first() != Some(&b'0'));
    if canonical { token.parse().ok() } else { None }
}

impl Clone for Value<'_> {
    fn clone(&self) -> Self {
        match self {
            Self::Null => Self::Null,
            Self::Bool(b) => Self::Bool(*b),
            Self::Number(n) => Self::Number(n.clone()),
            Self::String(s) => Self::String(s.clone()),
            Self::Array(a) => Self::Array(a.clone()),
            Self::Object(o) => Self::Object(o.clone()),
        }
    }
}

impl PartialEq for Value<'_> {
    fn eq(&self, other: &Self) -> bool {
        tree::eq(self, other)
    }
}

impl Eq for Value<'_> {}

impl<'a> Index<&str> for Value<'a> {
    type Output = Value<'a>;

    /// The member named `key`, or `null` if this is not an object or has no
    /// such member. Never panics.
    fn index(&self, key: &str) -> &Value<'a> {
        self.get(key).unwrap_or(&NULL)
    }
}

impl<'a> Index<usize> for Value<'a> {
    type Output = Value<'a>;

    /// The item at `index`, or `null` if this is not an array or is shorter.
    /// Never panics.
    fn index(&self, index: usize) -> &Value<'a> {
        self.get_index(index).unwrap_or(&NULL)
    }
}

// ---- Conversions into `Value` ------------------------------------------------

impl From<bool> for Value<'_> {
    #[inline]
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

macro_rules! from_integer {
    ($($t:ty),*) => {$(
        impl From<$t> for Value<'_> {
            #[inline]
            fn from(n: $t) -> Self {
                Self::Number(Number::from(n))
            }
        }
    )*};
}

from_integer!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

impl From<f64> for Value<'_> {
    /// A number in shortest round-trip form (see [`Number::from_f64`]), or
    /// `null` for NaN and the infinities, which JSON cannot represent.
    fn from(x: f64) -> Self {
        Number::from_f64(x).map_or(Self::Null, Self::Number)
    }
}

impl From<f32> for Value<'_> {
    /// The `f32` widened to `f64` (exactly), then as `From<f64>`.
    fn from(x: f32) -> Self {
        Self::from(f64::from(x))
    }
}

impl<'a> From<Number<'a>> for Value<'a> {
    #[inline]
    fn from(n: Number<'a>) -> Self {
        Self::Number(n)
    }
}

impl<'a> From<&'a str> for Value<'a> {
    #[inline]
    fn from(s: &'a str) -> Self {
        Self::String(Cow::Borrowed(s))
    }
}

impl<'a> From<&'a String> for Value<'a> {
    #[inline]
    fn from(s: &'a String) -> Self {
        Self::String(Cow::Borrowed(s))
    }
}

impl From<String> for Value<'_> {
    #[inline]
    fn from(s: String) -> Self {
        Self::String(Cow::Owned(s))
    }
}

impl<'a> From<Cow<'a, str>> for Value<'a> {
    #[inline]
    fn from(s: Cow<'a, str>) -> Self {
        Self::String(s)
    }
}

impl From<char> for Value<'_> {
    #[inline]
    fn from(c: char) -> Self {
        Self::String(Cow::Owned(c.to_string()))
    }
}

impl<'a> From<Array<'a>> for Value<'a> {
    #[inline]
    fn from(a: Array<'a>) -> Self {
        Self::Array(a)
    }
}

impl<'a> From<Object<'a>> for Value<'a> {
    #[inline]
    fn from(o: Object<'a>) -> Self {
        Self::Object(o)
    }
}

impl<'a, T: Into<Value<'a>>> From<Vec<T>> for Value<'a> {
    fn from(items: Vec<T>) -> Self {
        Self::Array(items.into_iter().collect())
    }
}

impl<'a, T: Into<Value<'a>>> From<Option<T>> for Value<'a> {
    /// `None` is `null`.
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Null, Into::into)
    }
}

impl From<()> for Value<'_> {
    /// `()` is `null`.
    #[inline]
    fn from((): ()) -> Self {
        Self::Null
    }
}

impl<'a, T: Into<Value<'a>>> FromIterator<T> for Value<'a> {
    /// Collects into an array.
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::Array(iter.into_iter().collect())
    }
}

// ---- Comparisons with Rust values --------------------------------------------

impl PartialEq<str> for Value<'_> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == Some(other)
    }
}

impl PartialEq<&str> for Value<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == Some(*other)
    }
}

impl PartialEq<String> for Value<'_> {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == Some(other.as_str())
    }
}

impl PartialEq<bool> for Value<'_> {
    fn eq(&self, other: &bool) -> bool {
        self.as_bool() == Some(*other)
    }
}

impl PartialEq<f64> for Value<'_> {
    /// Compares the number's value, so `1.0`, `1`, and `1e0` all equal `1.0`.
    fn eq(&self, other: &f64) -> bool {
        self.as_f64() == Some(*other)
    }
}

macro_rules! eq_signed {
    ($($t:ty),*) => {$(
        impl PartialEq<$t> for Value<'_> {
            /// True when the value is an integer number equal to `other`.
            fn eq(&self, other: &$t) -> bool {
                self.as_i64() == Some(i64::from(*other))
            }
        }
    )*};
}

macro_rules! eq_unsigned {
    ($($t:ty),*) => {$(
        impl PartialEq<$t> for Value<'_> {
            /// True when the value is an integer number equal to `other`.
            fn eq(&self, other: &$t) -> bool {
                self.as_u64() == Some(u64::from(*other))
            }
        }
    )*};
}

eq_signed!(i8, i16, i32, i64);
eq_unsigned!(u8, u16, u32, u64);

impl PartialEq<isize> for Value<'_> {
    /// True when the value is an integer number equal to `other`.
    fn eq(&self, other: &isize) -> bool {
        i64::try_from(*other).is_ok_and(|n| self.as_i64() == Some(n))
    }
}

impl PartialEq<usize> for Value<'_> {
    /// True when the value is an integer number equal to `other`.
    fn eq(&self, other: &usize) -> bool {
        u64::try_from(*other).is_ok_and(|n| self.as_u64() == Some(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_is_four_words() {
        assert!(core::mem::size_of::<Value<'_>>() <= 4 * core::mem::size_of::<usize>());
    }

    #[test]
    fn test_pointer_tokens() {
        assert_eq!(array_index("0"), Some(0));
        assert_eq!(array_index("10"), Some(10));
        assert_eq!(array_index("01"), None);
        assert_eq!(array_index("-"), None);
        assert_eq!(array_index(""), None);
        assert_eq!(array_index("+1"), None);
        assert_eq!(unescape_pointer_token("a~1b~0c").as_deref(), Some("a/b~c"));
        assert_eq!(unescape_pointer_token("~2"), None);
        assert_eq!(unescape_pointer_token("~"), None);
    }

    #[test]
    fn test_index_out_of_shape_is_null() {
        let v = Value::from(3);
        assert!(v["k"].is_null());
        assert!(v[0].is_null());
    }
}
