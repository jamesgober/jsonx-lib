//! Whole-tree operations that never recurse per level of nesting: drop, deep
//! clone, equality, and conversion to an owned tree.
//!
//! A derived `Clone`, `PartialEq`, or drop glue recurses once per level of
//! nesting, so a deep enough value would overflow the stack in code that never
//! asked for recursion. Each operation here keeps its own explicit stack on the
//! heap instead, so any value the parser can build, at any depth, can also be
//! cloned, compared, converted, and dropped. Drop is the one exception that
//! recurses at all: up to a fixed 32 levels, which frees ordinary documents
//! without allocating, and flattens anything deeper onto an explicit stack.

use alloc::{borrow::Cow, vec, vec::Vec};
use core::{
    hash::{Hash, Hasher},
    mem, slice,
};

use crate::{Array, Object, Value};

/// One object member: the key and its value.
pub(crate) type Member<'a> = (Cow<'a, str>, Value<'a>);

/// How many levels [`drop_bounded`] descends by plain recursion before it
/// switches to [`dismantle`]. A fixed number, whatever the input: the stack
/// used by a drop is bounded by it, never by the value's depth.
const DROP_RECURSION: usize = 32;

/// Drops the children of a container being dropped.
pub(crate) fn drop_items(items: &mut Vec<Value<'_>>) {
    for value in items.drain(..) {
        drop_bounded(value, DROP_RECURSION);
    }
}

/// Drops the members of an object being dropped.
pub(crate) fn drop_members(members: &mut Vec<Member<'_>>) {
    for (_, value) in members.drain(..) {
        drop_bounded(value, DROP_RECURSION);
    }
}

/// Drops `value`, recursing at most `budget` more levels; deeper parts are
/// flattened by [`dismantle`]. Shallow documents, the common case, are freed
/// without any extra allocation.
fn drop_bounded(value: Value<'_>, budget: usize) {
    match value {
        Value::Array(mut a) => {
            let items = a.items_mut();
            if budget == 0 {
                // The child's own buffer becomes the work stack.
                dismantle(mem::take(items));
            } else {
                for child in items.drain(..) {
                    drop_bounded(child, budget - 1);
                }
            }
        }
        Value::Object(mut o) => {
            let members = o.members_mut();
            if budget == 0 {
                dismantle(members.drain(..).map(|(_, v)| v).collect());
            } else {
                for (_, child) in members.drain(..) {
                    drop_bounded(child, budget - 1);
                }
            }
        }
        _ => {}
    }
}

/// Drops every value in `stack`, flattening nested containers into the stack
/// first, so each container is dropped with no children left in it.
pub(crate) fn dismantle(mut stack: Vec<Value<'_>>) {
    while let Some(value) = stack.pop() {
        match value {
            Value::Array(mut a) => stack.append(a.items_mut()),
            Value::Object(mut o) => stack.extend(o.members_mut().drain(..).map(|(_, v)| v)),
            _ => {}
        }
    }
}

/// Where a container being built collects its children.
enum Out<'a> {
    Array(Vec<Value<'a>>),
    /// The members so far, and the key of the member whose value is pending.
    Object(Vec<Member<'a>>, Cow<'a, str>),
}

impl<'a> Out<'a> {
    #[inline]
    fn push(&mut self, value: Value<'a>) {
        match self {
            Self::Array(items) => items.push(value),
            Self::Object(members, key) => members.push((mem::take(key), value)),
        }
    }

    #[inline]
    fn into_value(self) -> Value<'a> {
        match self {
            Self::Array(items) => Value::Array(Array::from(items)),
            Self::Object(members, _) => Value::Object(Object::from(members)),
        }
    }
}

/// The children of a container being cloned, still to visit.
enum Borrowed<'r, 'a> {
    Array(slice::Iter<'r, Value<'a>>),
    Object(slice::Iter<'r, Member<'a>>),
}

/// A copy of `value` when it has no children to visit.
fn clone_shallow<'a>(value: &Value<'a>) -> Value<'a> {
    match value {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(*b),
        Value::Number(n) => Value::Number(n.clone()),
        Value::String(s) => Value::String(s.clone()),
        Value::Array(_) => Value::Array(Array::new()),
        Value::Object(_) => Value::Object(Object::new()),
    }
}

/// Deep-copies the container described by `src` into `out`.
fn clone_container<'a>(src: Borrowed<'_, 'a>, out: Out<'a>) -> Value<'a> {
    let mut current = (src, out);
    let mut parents = Vec::new();
    loop {
        let next = match &mut current.0 {
            Borrowed::Array(it) => it.next(),
            Borrowed::Object(it) => it.next().map(|(key, value)| {
                if let Out::Object(_, pending) = &mut current.1 {
                    *pending = key.clone();
                }
                value
            }),
        };
        match next {
            Some(Value::Array(a)) if !a.is_empty() => {
                let child = (
                    Borrowed::Array(a.iter()),
                    Out::Array(Vec::with_capacity(a.len())),
                );
                parents.push(mem::replace(&mut current, child));
            }
            Some(Value::Object(o)) if !o.is_empty() => {
                let child = (
                    Borrowed::Object(o.as_slice().iter()),
                    Out::Object(Vec::with_capacity(o.len()), Cow::Borrowed("")),
                );
                parents.push(mem::replace(&mut current, child));
            }
            Some(value) => current.1.push(clone_shallow(value)),
            None => {
                let Some(parent) = parents.pop() else {
                    return current.1.into_value();
                };
                let done = mem::replace(&mut current, parent).1.into_value();
                current.1.push(done);
            }
        }
    }
}

/// Deep-copies an array's items.
pub(crate) fn clone_items<'a>(items: &[Value<'a>]) -> Vec<Value<'a>> {
    let src = Borrowed::Array(items.iter());
    match clone_container(src, Out::Array(Vec::with_capacity(items.len()))) {
        Value::Array(mut a) => mem::take(a.items_mut()),
        _ => Vec::new(),
    }
}

/// Deep-copies an object's members.
pub(crate) fn clone_members<'a>(members: &[Member<'a>]) -> Vec<Member<'a>> {
    let out = Out::Object(Vec::with_capacity(members.len()), Cow::Borrowed(""));
    match clone_container(Borrowed::Object(members.iter()), out) {
        Value::Object(mut o) => mem::take(o.members_mut()),
        _ => Vec::new(),
    }
}

/// Structural equality: same kinds, same scalars (numbers by exact text),
/// same array items in order, same object members in order.
pub(crate) fn eq(a: &Value<'_>, b: &Value<'_>) -> bool {
    let mut stack = Vec::new();
    let (mut x, mut y) = (a, b);
    loop {
        match (x, y) {
            (Value::Null, Value::Null) => {}
            (Value::Bool(p), Value::Bool(q)) if p == q => {}
            (Value::Number(p), Value::Number(q)) if p == q => {}
            (Value::String(p), Value::String(q)) if p == q => {}
            (Value::Array(p), Value::Array(q)) if p.len() == q.len() => {
                stack.extend(p.iter().zip(q.iter()));
            }
            (Value::Object(p), Value::Object(q)) if p.len() == q.len() => {
                for ((pk, pv), (qk, qv)) in p.as_slice().iter().zip(q.as_slice()) {
                    if pk != qk {
                        return false;
                    }
                    stack.push((pv, qv));
                }
            }
            _ => return false,
        }
        match stack.pop() {
            Some((p, q)) => (x, y) = (p, q),
            None => return true,
        }
    }
}

/// What [`hash`] still has to feed the hasher.
enum Pending<'r, 'a> {
    Value(&'r Value<'a>),
    Key(&'r str),
}

/// Feeds `value` to `state`, consistently with [`eq`]: the kind, then the
/// scalar (a number by its text), or a container's length and children in
/// order (an object's keys with their values). Iterative at any depth; a
/// scalar costs no allocation.
pub(crate) fn hash<H: Hasher>(value: &Value<'_>, state: &mut H) {
    let mut stack: Vec<Pending<'_, '_>> = Vec::new();
    let mut next = Some(Pending::Value(value));
    while let Some(item) = next.take().or_else(|| stack.pop()) {
        let value = match item {
            Pending::Key(key) => {
                key.hash(state);
                continue;
            }
            Pending::Value(value) => value,
        };
        mem::discriminant(value).hash(state);
        match value {
            Value::Null => {}
            Value::Bool(b) => b.hash(state),
            Value::Number(n) => n.as_str().hash(state),
            Value::String(s) => s.as_ref().hash(state),
            Value::Array(a) => {
                a.len().hash(state);
                stack.extend(a.iter().rev().map(Pending::Value));
            }
            Value::Object(o) => {
                o.len().hash(state);
                for (key, child) in o.as_slice().iter().rev() {
                    stack.push(Pending::Value(child));
                    stack.push(Pending::Key(key));
                }
            }
        }
    }
}

/// The children of a container being converted, still to visit.
enum Owned<'a> {
    Array(vec::IntoIter<Value<'a>>),
    Object(vec::IntoIter<Member<'a>>),
}

/// Converts `value` so that it borrows nothing, copying borrowed strings.
pub(crate) fn into_owned(value: Value<'_>) -> Value<'static> {
    let (src, out) = match value {
        Value::Array(mut a) if !a.is_empty() => {
            let items = mem::take(a.items_mut());
            let out = Out::Array(Vec::with_capacity(items.len()));
            (Owned::Array(items.into_iter()), out)
        }
        Value::Object(mut o) if !o.is_empty() => {
            let members = mem::take(o.members_mut());
            let out = Out::Object(Vec::with_capacity(members.len()), Cow::Borrowed(""));
            (Owned::Object(members.into_iter()), out)
        }
        other => return own_shallow(other),
    };
    let mut current = (src, out);
    let mut parents = Vec::new();
    loop {
        let next = match &mut current.0 {
            Owned::Array(it) => it.next(),
            Owned::Object(it) => it.next().map(|(key, value)| {
                if let Out::Object(_, pending) = &mut current.1 {
                    *pending = Cow::Owned(key.into_owned());
                }
                value
            }),
        };
        match next {
            Some(Value::Array(mut a)) if !a.is_empty() => {
                let items = mem::take(a.items_mut());
                let out = Out::Array(Vec::with_capacity(items.len()));
                parents.push(mem::replace(
                    &mut current,
                    (Owned::Array(items.into_iter()), out),
                ));
            }
            Some(Value::Object(mut o)) if !o.is_empty() => {
                let members = mem::take(o.members_mut());
                let out = Out::Object(Vec::with_capacity(members.len()), Cow::Borrowed(""));
                let child = (Owned::Object(members.into_iter()), out);
                parents.push(mem::replace(&mut current, child));
            }
            Some(value) => current.1.push(own_shallow(value)),
            None => {
                let Some(parent) = parents.pop() else {
                    return current.1.into_value();
                };
                let done = mem::replace(&mut current, parent).1.into_value();
                current.1.push(done);
            }
        }
    }
}

/// [`into_owned`] for a value with no children.
fn own_shallow(value: Value<'_>) -> Value<'static> {
    match value {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(b),
        Value::Number(n) => Value::Number(n.into_owned()),
        Value::String(s) => Value::String(Cow::Owned(s.into_owned())),
        Value::Array(_) => Value::Array(Array::new()),
        Value::Object(_) => Value::Object(Object::new()),
    }
}
