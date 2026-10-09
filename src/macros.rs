//! The `json!` macro.

/// Builds a [`Value`](crate::Value) from JSON-like syntax.
///
/// `null`, `true`, `false`, arrays `[...]`, and objects `{...}` are written as
/// in JSON, nested to any depth. Anything else is a Rust expression converted
/// with [`Value::from`](crate::Value), so variables, numbers, strings, `Option`s,
/// `Vec`s, and other values drop straight in. An object key is a string
/// literal, or any expression in parentheses that converts into
/// `Cow<str>` (a `&str` or a `String`).
///
/// Members are appended in the order written, as [`Object::push`](crate::Object::push)
/// does; a key written twice appears twice. An expression containing a
/// top-level comma (a generic with two parameters, say) must be wrapped in
/// parentheses. Each item costs the compiler a few levels of macro
/// recursion, so a very long literal may need `#![recursion_limit]` raised;
/// build large values in code instead.
///
/// # Examples
///
/// ```
/// use jsonx_lib::json;
///
/// let id = 7;
/// let method = String::from("textDocument/hover");
/// let field = "line";
/// let msg = json!({
///     "jsonrpc": "2.0",
///     "id": id,
///     "method": method,
///     "params": { "position": { (field): 3, "character": 1 + 1 } },
///     "flags": [true, null, -1.5],
/// });
/// assert_eq!(
///     msg.to_string(),
///     r#"{"jsonrpc":"2.0","id":7,"method":"textDocument/hover","params":{"position":{"line":3,"character":2}},"flags":[true,null,-1.5]}"#
/// );
/// ```
#[macro_export]
macro_rules! json {
    ($($tt:tt)+) => {
        $crate::__json_value!($($tt)+)
    };
}

/// Implementation detail of [`json!`].
#[doc(hidden)]
#[macro_export]
macro_rules! __json_value {
    (null) => {
        $crate::Value::Null
    };
    (true) => {
        $crate::Value::Bool(true)
    };
    (false) => {
        $crate::Value::Bool(false)
    };
    ([]) => {
        $crate::Value::Array($crate::Array::new())
    };
    ([ $($tt:tt)+ ]) => {
        $crate::Value::Array($crate::Array::from(
            $crate::__json_array!([] [] $($tt)+)
        ))
    };
    ({}) => {
        $crate::Value::Object($crate::Object::new())
    };
    ({ $($tt:tt)+ }) => {
        $crate::Value::Object($crate::Object::from(
            $crate::__json_object!([] $($tt)+)
        ))
    };
    ($other:expr) => {
        $crate::Value::from($other)
    };
}

/// Implementation detail of [`json!`]: munches array items, splitting at
/// top-level commas. State: `[done items] [tokens of the current item] rest`.
#[doc(hidden)]
#[macro_export]
macro_rules! __json_array {
    // The end, after a trailing comma or with nothing pending.
    ([$($done:expr,)*] []) => {
        $crate::__private::vec![$($done,)*]
    };
    // The end, with the last item pending.
    ([$($done:expr,)*] [$($item:tt)+]) => {
        $crate::__private::vec![$($done,)* $crate::__json_value!($($item)+)]
    };
    // A comma ends the pending item.
    ([$($done:expr,)*] [$($item:tt)+] , $($rest:tt)*) => {
        $crate::__json_array!([$($done,)* $crate::__json_value!($($item)+),] [] $($rest)*)
    };
    // Any other token joins the pending item.
    ([$($done:expr,)*] [$($item:tt)*] $next:tt $($rest:tt)*) => {
        $crate::__json_array!([$($done,)*] [$($item)* $next] $($rest)*)
    };
}

/// Implementation detail of [`json!`]: munches object members. A key is a
/// literal or a parenthesized expression, then `:`, then the value's tokens up
/// to a top-level comma.
#[doc(hidden)]
#[macro_export]
macro_rules! __json_object {
    // The end.
    ([$($done:expr,)*]) => {
        $crate::__private::vec![$($done,)*]
    };
    // A key; start collecting its value.
    ([$($done:expr,)*] $key:literal : $($rest:tt)+) => {
        $crate::__json_member!([$($done,)*] ($key) [] $($rest)+)
    };
    ([$($done:expr,)*] ($key:expr) : $($rest:tt)+) => {
        $crate::__json_member!([$($done,)*] ($key) [] $($rest)+)
    };
}

/// Implementation detail of [`json!`]: collects one member's value.
#[doc(hidden)]
#[macro_export]
macro_rules! __json_member {
    // The last member, without a trailing comma.
    ([$($done:expr,)*] ($key:expr) [$($value:tt)+]) => {
        $crate::__json_object!([$($done,)* $crate::__private::member($key, $crate::__json_value!($($value)+)),])
    };
    // A comma ends the value.
    ([$($done:expr,)*] ($key:expr) [$($value:tt)+] , $($rest:tt)*) => {
        $crate::__json_object!([$($done,)* $crate::__private::member($key, $crate::__json_value!($($value)+)),] $($rest)*)
    };
    // Any other token joins the value.
    ([$($done:expr,)*] ($key:expr) [$($value:tt)*] $next:tt $($rest:tt)*) => {
        $crate::__json_member!([$($done,)*] ($key) [$($value)* $next] $($rest)*)
    };
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};

    use crate::Value;

    #[test]
    fn test_json_macro_shapes() {
        assert_eq!(json!(null), Value::Null);
        assert_eq!(json!([]).to_string(), "[]");
        assert_eq!(json!({}).to_string(), "{}");
        assert_eq!(json!([1, [2, [3]], {}]).to_string(), "[1,[2,[3]],{}]");
        assert_eq!(json!([1,]).to_string(), "[1]");
        assert_eq!(json!({ "a": 1, }).to_string(), r#"{"a":1}"#);
        assert_eq!(json!(-5).to_string(), "-5");
        assert_eq!(json!([-5, -6]).to_string(), "[-5,-6]");
        let key = String::from("k");
        assert_eq!(
            json!({ (key.clone()): [key.as_str()] }).to_string(),
            r#"{"k":["k"]}"#
        );
        let none: Option<i32> = None;
        assert_eq!(json!([none, Some(2)]).to_string(), "[null,2]");
        assert_eq!(
            json!({ "a": { "b": { "c": [null, true, false] } } }).to_string(),
            r#"{"a":{"b":{"c":[null,true,false]}}}"#
        );
        assert_eq!(json!({ "f": 0.5 }).to_string(), r#"{"f":0.5}"#);
    }
}
