//! JSON arrays.

use alloc::vec::{self, Vec};
use core::{
    fmt,
    ops::{Deref, DerefMut},
    slice,
};

use crate::{Value, tree};

/// A JSON array: a list of values in document order.
///
/// `Array` dereferences to `Vec<Value>`, so every slice and vector method
/// works on it directly (`len`, `iter`, `push`, indexing, sorting). It exists
/// as its own type rather than a bare `Vec` so that dropping, cloning, and
/// comparing an arbitrarily deep value uses bounded stack.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{json, parse, Array, Value};
///
/// let doc = parse("[1, [2, 3]]")?;
/// let items: &Array = doc.as_array().unwrap();
/// assert_eq!(items.len(), 2);
/// assert_eq!(items[1][0], 2);
///
/// let mut built = Array::new();
/// built.push(Value::from("a"));
/// built.push(json!(null));
/// assert_eq!(Value::Array(built).to_string(), r#"["a",null]"#);
/// # Ok::<(), jsonx_lib::Error>(())
/// ```
#[derive(Default)]
pub struct Array<'a> {
    items: Vec<Value<'a>>,
}

impl<'a> Array<'a> {
    /// An empty array. Does not allocate.
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(jsonx_lib::Array::new().is_empty());
    /// ```
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// An empty array with room for `capacity` items.
    ///
    /// # Examples
    ///
    /// ```
    /// let a = jsonx_lib::Array::with_capacity(8);
    /// assert!(a.capacity() >= 8);
    /// ```
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
        }
    }

    /// Takes the items out as a `Vec`.
    ///
    /// # Examples
    ///
    /// ```
    /// let doc = jsonx_lib::parse("[true, false]")?;
    /// let items = doc.as_array().cloned().unwrap_or_default().into_vec();
    /// assert_eq!(items, [true, false]);
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn into_vec(mut self) -> Vec<Value<'a>> {
        core::mem::take(&mut self.items)
    }

    /// Copies every borrowed string so the array no longer borrows from the
    /// input. Iterative at any depth.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{parse, Array};
    ///
    /// let owned: Array<'static> = {
    ///     let text = String::from(r#"["x"]"#);
    ///     parse(&text)?.as_array().cloned().unwrap_or_default().into_owned()
    /// };
    /// assert_eq!(owned[0], "x");
    /// # Ok::<(), jsonx_lib::Error>(())
    /// ```
    #[must_use]
    pub fn into_owned(self) -> Array<'static> {
        match tree::into_owned(Value::Array(self)) {
            Value::Array(a) => a,
            _ => Array::new(),
        }
    }

    #[inline]
    pub(crate) fn items_mut(&mut self) -> &mut Vec<Value<'a>> {
        &mut self.items
    }
}

impl<'a> Deref for Array<'a> {
    type Target = Vec<Value<'a>>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

impl DerefMut for Array<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.items
    }
}

impl Drop for Array<'_> {
    fn drop(&mut self) {
        // Recursion is capped at a fixed depth; deeper parts are flattened
        // onto an explicit stack, so a deep value never exhausts the stack.
        tree::drop_items(&mut self.items);
    }
}

impl Clone for Array<'_> {
    fn clone(&self) -> Self {
        Self {
            items: tree::clone_items(&self.items),
        }
    }
}

impl PartialEq for Array<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.items.len() == other.items.len()
            && self
                .items
                .iter()
                .zip(&other.items)
                .all(|(a, b)| tree::eq(a, b))
    }
}

impl Eq for Array<'_> {}

impl fmt::Debug for Array<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.items.iter()).finish()
    }
}

impl<'a> From<Vec<Value<'a>>> for Array<'a> {
    #[inline]
    fn from(items: Vec<Value<'a>>) -> Self {
        Self { items }
    }
}

impl<'a, T: Into<Value<'a>>> FromIterator<T> for Array<'a> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            items: iter.into_iter().map(Into::into).collect(),
        }
    }
}

impl<'a, T: Into<Value<'a>>> Extend<T> for Array<'a> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.items.extend(iter.into_iter().map(Into::into));
    }
}

impl<'a> IntoIterator for Array<'a> {
    type Item = Value<'a>;
    type IntoIter = vec::IntoIter<Value<'a>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<'r, 'a> IntoIterator for &'r Array<'a> {
    type Item = &'r Value<'a>;
    type IntoIter = slice::Iter<'r, Value<'a>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<'r, 'a> IntoIterator for &'r mut Array<'a> {
    type Item = &'r mut Value<'a>;
    type IntoIter = slice::IterMut<'r, Value<'a>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter_mut()
    }
}
