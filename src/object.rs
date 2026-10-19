//! JSON objects.

use alloc::{borrow::Cow, vec, vec::Vec};
use core::{fmt, iter::FusedIterator, ops::Index, slice};

use crate::{Value, tree, tree::Member};

/// A JSON object: its members in document order.
///
/// Members are kept in a vector, in the order they were read or added, so a
/// document is written back with its keys where they were. Lookup by key is a
/// linear scan, which beats hashing for the small objects JSON documents are
/// mostly made of; for a large object read once, iterate it instead of
/// looking each key up.
///
/// Whether a parsed object can hold a key twice is up to the
/// [`DuplicateKeys`](crate::DuplicateKeys) policy the parser used (by default
/// a duplicate is an error). An object built in code can: [`insert`](Self::insert)
/// replaces an existing member, [`push`](Self::push) always appends.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{parse, Object, Value};
///
/// let doc = parse(r#"{"name": "jsonx", "tags": ["json"]}"#)?;
/// let obj: &Object = doc.as_object().unwrap();
/// assert_eq!(obj.get("name").and_then(Value::as_str), Some("jsonx"));
/// assert_eq!(obj.keys().collect::<Vec<_>>(), ["name", "tags"]);
///
/// let mut built = Object::new();
/// built.insert("b", 1);
/// built.insert("a", true);
/// built.insert("b", 2); // replaces in place
/// assert_eq!(Value::Object(built).to_string(), r#"{"b":2,"a":true}"#);
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Default)]
pub struct Object<'a> {
    members: Vec<Member<'a>>,
}

impl<'a> Object<'a> {
    /// An empty object. Does not allocate.
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(jsonx_lib::Object::new().is_empty());
    /// ```
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            members: Vec::new(),
        }
    }

    /// An empty object with room for `capacity` members.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut o = jsonx_lib::Object::with_capacity(2);
    /// o.push("k", "v");
    /// assert_eq!(o.len(), 1);
    /// ```
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            members: Vec::with_capacity(capacity),
        }
    }

    /// The number of members.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse(r#"{"a": 1, "b": 2}"#)?;
    /// assert_eq!(doc.as_object().map(|o| o.len()), Some(2));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether the object has no members.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse("{}")?;
    /// assert!(doc.as_object().is_some_and(|o| o.is_empty()));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The value of the first member named `key`.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse(r#"{"id": 7}"#)?;
    /// let obj = doc.as_object().unwrap();
    /// assert_eq!(obj.get("id").and_then(|v| v.as_u64()), Some(7));
    /// assert!(obj.get("missing").is_none());
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value<'a>> {
        self.members.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// A mutable reference to the value of the first member named `key`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{json, Value};
    ///
    /// let mut v = json!({ "n": 1 });
    /// if let Some(n) = v.as_object_mut().and_then(|o| o.get_mut("n")) {
    ///     *n = Value::from(2);
    /// }
    /// assert_eq!(v["n"], 2);
    /// ```
    #[must_use]
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value<'a>> {
        self.members
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Whether a member is named `key`.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse(r#"{"a": null}"#)?;
    /// assert!(doc.as_object().is_some_and(|o| o.contains_key("a")));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.members.iter().any(|(k, _)| k == key)
    }

    /// Sets the member named `key` to `value`, as a map would: an existing
    /// member keeps its position and its old value is returned; otherwise the
    /// member is appended and `None` is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Object;
    ///
    /// let mut o = Object::new();
    /// assert_eq!(o.insert("a", 1), None);
    /// assert_eq!(o.insert("a", 2), Some(1.into()));
    /// assert_eq!(o.len(), 1);
    /// ```
    pub fn insert(
        &mut self,
        key: impl Into<Cow<'a, str>>,
        value: impl Into<Value<'a>>,
    ) -> Option<Value<'a>> {
        let key = key.into();
        let value = value.into();
        match self.get_mut(&key) {
            Some(slot) => Some(core::mem::replace(slot, value)),
            None => {
                self.members.push((key, value));
                None
            }
        }
    }

    /// Appends a member without looking for an existing one with the same
    /// key. Constant time; the writer then emits both members. Use
    /// [`insert`](Self::insert) for map semantics.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Object, Value};
    ///
    /// let mut o = Object::new();
    /// o.push("k", 1);
    /// o.push("k", 2);
    /// assert_eq!(Value::Object(o).to_string(), r#"{"k":1,"k":2}"#);
    /// ```
    pub fn push(&mut self, key: impl Into<Cow<'a, str>>, value: impl Into<Value<'a>>) {
        self.members.push((key.into(), value.into()));
    }

    /// Removes the first member named `key` and returns its value. The other
    /// members keep their order.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::json;
    ///
    /// let mut v = json!({ "a": 1, "b": 2, "c": 3 });
    /// let obj = v.as_object_mut().unwrap();
    /// assert_eq!(obj.remove("b"), Some(2.into()));
    /// assert_eq!(obj.keys().collect::<Vec<_>>(), ["a", "c"]);
    /// ```
    pub fn remove(&mut self, key: &str) -> Option<Value<'a>> {
        let at = self.members.iter().position(|(k, _)| k == key)?;
        Some(self.members.remove(at).1)
    }

    /// Keeps only the members for which `keep` returns `true`, in order;
    /// `keep` may change the values it keeps.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut v = jsonx_lib::json!({ "a": 1, "b": null, "c": 3 });
    /// v.as_object_mut().unwrap().retain(|_, value| !value.is_null());
    /// assert_eq!(v.to_string(), r#"{"a":1,"c":3}"#);
    /// ```
    pub fn retain(&mut self, mut keep: impl FnMut(&str, &mut Value<'a>) -> bool) {
        self.members.retain_mut(|(key, value)| keep(key, value));
    }

    /// Sorts the members by key (byte order; members with equal keys keep
    /// their order), for output that does not depend on how the object was
    /// built.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut v = jsonx_lib::json!({ "b": 1, "a": 2 });
    /// v.as_object_mut().unwrap().sort_keys();
    /// assert_eq!(v.to_string(), r#"{"a":2,"b":1}"#);
    /// ```
    pub fn sort_keys(&mut self) {
        self.members.sort_by(|(a, _), (b, _)| a.cmp(b));
    }

    /// The member named `key`, to read, change, or insert in one lookup.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{json, Value};
    ///
    /// let mut v = json!({ "hits": 1 });
    /// let obj = v.as_object_mut().unwrap();
    /// for key in ["hits", "misses", "hits"] {
    ///     let count = obj.entry(key).or_insert(0);
    ///     *count = Value::from(count.as_i64().unwrap_or(0) + 1);
    /// }
    /// assert_eq!(v.to_string(), r#"{"hits":3,"misses":1}"#);
    /// ```
    pub fn entry(&mut self, key: impl Into<Cow<'a, str>>) -> Entry<'_, 'a> {
        let key = key.into();
        match self.members.iter().position(|(k, _)| *k == key) {
            Some(index) => Entry::Occupied(OccupiedEntry {
                object: self,
                index,
            }),
            None => Entry::Vacant(VacantEntry { object: self, key }),
        }
    }

    /// Iterates over the members as `(key, value)` pairs, in order.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse(r#"{"x": 1, "y": 2}"#)?;
    /// let sum: i64 = doc.as_object().unwrap().iter().filter_map(|(_, v)| v.as_i64()).sum();
    /// assert_eq!(sum, 3);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    pub fn iter(&self) -> Iter<'_, 'a> {
        Iter {
            inner: self.members.iter(),
        }
    }

    /// Iterates over the members with mutable values, in order.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{json, Value};
    ///
    /// let mut v = json!({ "a": 1, "b": 2 });
    /// for (_, value) in v.as_object_mut().unwrap().iter_mut() {
    ///     *value = Value::Null;
    /// }
    /// assert_eq!(v.to_string(), r#"{"a":null,"b":null}"#);
    /// ```
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, 'a> {
        IterMut {
            inner: self.members.iter_mut(),
        }
    }

    /// The keys, in order.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse(r#"{"b": 1, "a": 2}"#)?;
    /// assert_eq!(doc.as_object().unwrap().keys().collect::<Vec<_>>(), ["b", "a"]);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn keys(&self) -> impl DoubleEndedIterator<Item = &str> + ExactSizeIterator {
        self.members.iter().map(|(k, _)| k.as_ref())
    }

    /// The values, in order.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse(r#"{"b": 1, "a": 2}"#)?;
    /// let values: Vec<_> = doc.as_object().unwrap().values().filter_map(|v| v.as_i64()).collect();
    /// assert_eq!(values, [1, 2]);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    pub fn values(&self) -> impl DoubleEndedIterator<Item = &Value<'a>> + ExactSizeIterator {
        self.members.iter().map(|(_, v)| v)
    }

    /// The members as a slice of `(key, value)` pairs.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse(r#"{"k": "v"}"#)?;
    /// let (key, value) = &doc.as_object().unwrap().as_slice()[0];
    /// assert_eq!((key.as_ref(), value.as_str()), ("k", Some("v")));
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[(Cow<'a, str>, Value<'a>)] {
        &self.members
    }

    /// Takes the members out as a `Vec` of `(key, value)` pairs.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::json;
    ///
    /// let v = json!({ "a": 1 });
    /// let members = v.as_object().cloned().unwrap_or_default().into_vec();
    /// assert_eq!(members[0].0, "a");
    /// ```
    #[inline]
    #[must_use]
    pub fn into_vec(mut self) -> Vec<(Cow<'a, str>, Value<'a>)> {
        core::mem::take(&mut self.members)
    }

    /// Copies every borrowed key and string so the object no longer borrows
    /// from the input. Iterative at any depth.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{parse, Object};
    ///
    /// let owned: Object<'static> = {
    ///     let text = String::from(r#"{"k": "v"}"#);
    ///     parse(&text)?.as_object().cloned().unwrap_or_default().into_owned()
    /// };
    /// assert_eq!(owned["k"], "v");
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn into_owned(self) -> Object<'static> {
        match tree::into_owned(Value::Object(self)) {
            Value::Object(o) => o,
            _ => Object::new(),
        }
    }

    /// The value of the member at `index`. Only the entry API calls this,
    /// with an index it just found or pushed while it holds the object
    /// exclusively, so the index is always in bounds.
    #[allow(clippy::indexing_slicing)]
    fn member_value(&mut self, index: usize) -> &mut Value<'a> {
        &mut self.members[index].1
    }

    #[inline]
    pub(crate) fn members_mut(&mut self) -> &mut Vec<Member<'a>> {
        &mut self.members
    }
}

impl Drop for Object<'_> {
    fn drop(&mut self) {
        tree::drop_members(&mut self.members);
    }
}

impl Clone for Object<'_> {
    fn clone(&self) -> Self {
        Self {
            members: tree::clone_members(&self.members),
        }
    }
}

impl PartialEq for Object<'_> {
    /// Members are compared in order: `{"a":1,"b":2}` and `{"b":2,"a":1}` are
    /// different documents and compare unequal.
    fn eq(&self, other: &Self) -> bool {
        self.members.len() == other.members.len()
            && self
                .members
                .iter()
                .zip(&other.members)
                .all(|((ka, va), (kb, vb))| ka == kb && tree::eq(va, vb))
    }
}

impl Eq for Object<'_> {}

impl core::hash::Hash for Object<'_> {
    /// Hashes as the [`Value::Object`] it would be: consistent with `==`.
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        for (key, value) in self {
            key.hash(state);
            value.hash(state);
        }
    }
}

/// One member of an [`Object`], there or not; from [`Object::entry`].
#[derive(Debug)]
pub enum Entry<'o, 'a> {
    /// The object has a member with the key (the first, if several).
    Occupied(OccupiedEntry<'o, 'a>),
    /// It does not.
    Vacant(VacantEntry<'o, 'a>),
}

impl<'o, 'a> Entry<'o, 'a> {
    /// The member's value, inserting `default` first if it is missing.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut o = jsonx_lib::Object::new();
    /// o.entry("list").or_insert(jsonx_lib::json!([])).as_array_mut().unwrap().push(1.into());
    /// assert_eq!(o["list"][0], 1);
    /// ```
    pub fn or_insert(self, default: impl Into<Value<'a>>) -> &'o mut Value<'a> {
        match self {
            Self::Occupied(entry) => entry.into_mut(),
            Self::Vacant(entry) => entry.insert(default),
        }
    }

    /// The member's value, inserting what `default` makes first if it is
    /// missing.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut o = jsonx_lib::Object::new();
    /// assert!(o.entry("k").or_insert_with(|| "made".into()) == "made");
    /// ```
    pub fn or_insert_with(self, default: impl FnOnce() -> Value<'a>) -> &'o mut Value<'a> {
        match self {
            Self::Occupied(entry) => entry.into_mut(),
            Self::Vacant(entry) => entry.insert(default()),
        }
    }

    /// The member's value, inserting `null` first if it is missing.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut o = jsonx_lib::Object::new();
    /// assert!(o.entry("k").or_default().is_null());
    /// assert_eq!(o.len(), 1);
    /// ```
    pub fn or_default(self) -> &'o mut Value<'a> {
        self.or_insert(Value::Null)
    }

    /// Runs `change` on the value if the member is there.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut v = jsonx_lib::json!({ "n": 1 });
    /// let o = v.as_object_mut().unwrap();
    /// o.entry("n").and_modify(|n| *n = 2.into()).or_insert(0);
    /// o.entry("m").and_modify(|n| *n = 2.into()).or_insert(0);
    /// assert_eq!(v.to_string(), r#"{"n":2,"m":0}"#);
    /// ```
    #[must_use]
    pub fn and_modify(self, change: impl FnOnce(&mut Value<'a>)) -> Self {
        match self {
            Self::Occupied(mut entry) => {
                change(entry.get_mut());
                Self::Occupied(entry)
            }
            vacant @ Self::Vacant(_) => vacant,
        }
    }

    /// The member's key.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(jsonx_lib::Object::new().entry("k").key(), "k");
    /// ```
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Occupied(entry) => entry.key(),
            Self::Vacant(entry) => &entry.key,
        }
    }
}

/// A member that is there; see [`Entry`].
#[derive(Debug)]
pub struct OccupiedEntry<'o, 'a> {
    object: &'o mut Object<'a>,
    index: usize,
}

impl<'o, 'a> OccupiedEntry<'o, 'a> {
    /// The key.
    #[must_use]
    pub fn key(&self) -> &str {
        self.object.members.get(self.index).map_or("", |(k, _)| k)
    }

    /// The value.
    #[must_use]
    pub fn get(&self) -> &Value<'a> {
        self.object
            .members
            .get(self.index)
            .map_or(&crate::value::NULL, |(_, v)| v)
    }

    /// The value, mutably.
    pub fn get_mut(&mut self) -> &mut Value<'a> {
        let index = self.index;
        self.object.member_value(index)
    }

    /// The value, mutably, for as long as the object is borrowed.
    pub fn into_mut(self) -> &'o mut Value<'a> {
        self.object.member_value(self.index)
    }

    /// Replaces the value and returns the old one.
    pub fn insert(&mut self, value: impl Into<Value<'a>>) -> Value<'a> {
        core::mem::replace(self.get_mut(), value.into())
    }

    /// Removes the member, keeping the others' order, and returns its value.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Entry;
    ///
    /// let mut v = jsonx_lib::json!({ "a": 1, "b": 2 });
    /// if let Entry::Occupied(e) = v.as_object_mut().unwrap().entry("a") {
    ///     assert_eq!(e.remove(), 1);
    /// }
    /// assert_eq!(v.to_string(), r#"{"b":2}"#);
    /// ```
    pub fn remove(self) -> Value<'a> {
        if self.index < self.object.members.len() {
            self.object.members.remove(self.index).1
        } else {
            Value::Null
        }
    }
}

/// A member that is not there; see [`Entry`].
#[derive(Debug)]
pub struct VacantEntry<'o, 'a> {
    object: &'o mut Object<'a>,
    key: Cow<'a, str>,
}

impl<'o, 'a> VacantEntry<'o, 'a> {
    /// The key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Appends the member and returns its value.
    pub fn insert(self, value: impl Into<Value<'a>>) -> &'o mut Value<'a> {
        let index = self.object.members.len();
        self.object.members.push((self.key, value.into()));
        self.object.member_value(index)
    }
}

impl fmt::Debug for Object<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<'a> From<Vec<(Cow<'a, str>, Value<'a>)>> for Object<'a> {
    /// Wraps the members as given; duplicates are kept.
    #[inline]
    fn from(members: Vec<(Cow<'a, str>, Value<'a>)>) -> Self {
        Self { members }
    }
}

impl<'a, K: Into<Cow<'a, str>>, V: Into<Value<'a>>> FromIterator<(K, V)> for Object<'a> {
    /// Appends the members in order, as [`push`](Object::push) does;
    /// duplicates are kept.
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self {
            members: iter
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
}

impl<'a, K: Into<Cow<'a, str>>, V: Into<Value<'a>>> Extend<(K, V)> for Object<'a> {
    /// Appends the members in order, as [`push`](Object::push) does.
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        self.members
            .extend(iter.into_iter().map(|(k, v)| (k.into(), v.into())));
    }
}

impl<'a> Index<&str> for Object<'a> {
    type Output = Value<'a>;

    /// The value of the first member named `key`, or `null` if there is none.
    fn index(&self, key: &str) -> &Value<'a> {
        self.get(key).unwrap_or(&crate::value::NULL)
    }
}

impl<'a> IntoIterator for Object<'a> {
    type Item = (Cow<'a, str>, Value<'a>);
    type IntoIter = vec::IntoIter<(Cow<'a, str>, Value<'a>)>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<'r, 'a> IntoIterator for &'r Object<'a> {
    type Item = (&'r str, &'r Value<'a>);
    type IntoIter = Iter<'r, 'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'r, 'a> IntoIterator for &'r mut Object<'a> {
    type Item = (&'r str, &'r mut Value<'a>);
    type IntoIter = IterMut<'r, 'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// An iterator over an [`Object`]'s members, from [`Object::iter`].
///
/// # Examples
///
/// ```
/// let doc = jsonx_lib::parse(r#"{"a": 1}"#)?;
/// let mut it = doc.as_object().unwrap().iter();
/// assert_eq!(it.next().map(|(k, _)| k), Some("a"));
/// assert!(it.next().is_none());
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct Iter<'r, 'a> {
    inner: slice::Iter<'r, Member<'a>>,
}

impl<'r, 'a> Iterator for Iter<'r, 'a> {
    type Item = (&'r str, &'r Value<'a>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(k, v)| (k.as_ref(), v))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl DoubleEndedIterator for Iter<'_, '_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(k, v)| (k.as_ref(), v))
    }
}

impl ExactSizeIterator for Iter<'_, '_> {}
impl FusedIterator for Iter<'_, '_> {}

/// An iterator over an [`Object`]'s members with mutable values, from
/// [`Object::iter_mut`].
///
/// # Examples
///
/// ```
/// use jsonx_lib::json;
///
/// let mut v = json!({ "a": 1 });
/// let mut it = v.as_object_mut().unwrap().iter_mut();
/// assert_eq!(it.next().map(|(k, _)| k), Some("a"));
/// ```
#[derive(Debug)]
pub struct IterMut<'r, 'a> {
    inner: slice::IterMut<'r, Member<'a>>,
}

impl<'r, 'a> Iterator for IterMut<'r, 'a> {
    type Item = (&'r str, &'r mut Value<'a>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(split_mut)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl DoubleEndedIterator for IterMut<'_, '_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(split_mut)
    }
}

/// A member as a shared key and a mutable value.
#[inline]
fn split_mut<'r, 'a>(member: &'r mut Member<'a>) -> (&'r str, &'r mut Value<'a>) {
    let (key, value) = member;
    let key: &'r Cow<'a, str> = key;
    (key, value)
}

impl ExactSizeIterator for IterMut<'_, '_> {}
impl FusedIterator for IterMut<'_, '_> {}
