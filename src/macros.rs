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
/// parentheses.
///
/// **Size.** The macro takes items that are a single token (`1`, `"s"`,
/// `x`, `null`, a nested `[...]` or `{...}`) up to thirty-two at a time, and
/// members with a string-literal key and a single-token value up to sixteen
/// at a time, so literals of thousands of such items expand within the
/// default `recursion_limit` (the tests check 2,000 array items and 1,000
/// members). Two-token array items (a negative number) go eight at a time.
/// Anything longer (`a + b`, `x.len()`, a member with a computed key or a
/// longer value) costs one level of recursion per token, so a literal holds
/// a few dozen of those at the default limit; build larger values in code.
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
    // Thirty-two single-token items at once, when nothing is pending.
    ([$($done:expr,)*] [] $a0:tt , $a1:tt , $a2:tt , $a3:tt , $a4:tt , $a5:tt , $a6:tt , $a7:tt , $a8:tt , $a9:tt , $a10:tt , $a11:tt , $a12:tt , $a13:tt , $a14:tt , $a15:tt , $a16:tt , $a17:tt , $a18:tt , $a19:tt , $a20:tt , $a21:tt , $a22:tt , $a23:tt , $a24:tt , $a25:tt , $a26:tt , $a27:tt , $a28:tt , $a29:tt , $a30:tt , $a31:tt , $($rest:tt)*) => {
        $crate::__json_array!([$($done,)*
            $crate::__json_value!($a0),
            $crate::__json_value!($a1),
            $crate::__json_value!($a2),
            $crate::__json_value!($a3),
            $crate::__json_value!($a4),
            $crate::__json_value!($a5),
            $crate::__json_value!($a6),
            $crate::__json_value!($a7),
            $crate::__json_value!($a8),
            $crate::__json_value!($a9),
            $crate::__json_value!($a10),
            $crate::__json_value!($a11),
            $crate::__json_value!($a12),
            $crate::__json_value!($a13),
            $crate::__json_value!($a14),
            $crate::__json_value!($a15),
            $crate::__json_value!($a16),
            $crate::__json_value!($a17),
            $crate::__json_value!($a18),
            $crate::__json_value!($a19),
            $crate::__json_value!($a20),
            $crate::__json_value!($a21),
            $crate::__json_value!($a22),
            $crate::__json_value!($a23),
            $crate::__json_value!($a24),
            $crate::__json_value!($a25),
            $crate::__json_value!($a26),
            $crate::__json_value!($a27),
            $crate::__json_value!($a28),
            $crate::__json_value!($a29),
            $crate::__json_value!($a30),
            $crate::__json_value!($a31),
        ] [] $($rest)*)
    };
    // Eight single-token items at once, when nothing is pending.
    ([$($done:expr,)*] [] $a:tt , $b:tt , $c:tt , $d:tt , $e:tt , $f:tt , $g:tt , $h:tt , $($rest:tt)*) => {
        $crate::__json_array!([$($done,)*
            $crate::__json_value!($a), $crate::__json_value!($b),
            $crate::__json_value!($c), $crate::__json_value!($d),
            $crate::__json_value!($e), $crate::__json_value!($f),
            $crate::__json_value!($g), $crate::__json_value!($h),
        ] [] $($rest)*)
    };
    // One single-token item, when nothing is pending.
    ([$($done:expr,)*] [] $a:tt , $($rest:tt)*) => {
        $crate::__json_array!([$($done,)* $crate::__json_value!($a),] [] $($rest)*)
    };
    // The end, with the last item pending.
    ([$($done:expr,)*] [$($item:tt)+]) => {
        $crate::__private::vec![$($done,)* $crate::__json_value!($($item)+)]
    };
    // Eight two-token items (negative numbers) at once.
    ([$($done:expr,)*] [] $a0:tt $b0:tt , $a1:tt $b1:tt , $a2:tt $b2:tt , $a3:tt $b3:tt , $a4:tt $b4:tt , $a5:tt $b5:tt , $a6:tt $b6:tt , $a7:tt $b7:tt , $($rest:tt)*) => {
        $crate::__json_array!([$($done,)*
            $crate::__json_value!($a0 $b0),
            $crate::__json_value!($a1 $b1),
            $crate::__json_value!($a2 $b2),
            $crate::__json_value!($a3 $b3),
            $crate::__json_value!($a4 $b4),
            $crate::__json_value!($a5 $b5),
            $crate::__json_value!($a6 $b6),
            $crate::__json_value!($a7 $b7),
        ] [] $($rest)*)
    };
    // One two-token item.
    ([$($done:expr,)*] [] $a:tt $b:tt , $($rest:tt)*) => {
        $crate::__json_array!([$($done,)* $crate::__json_value!($a $b),] [] $($rest)*)
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
    // Sixteen members with literal keys and single-token values at once.
    ([$($done:expr,)*] $k0:literal : $v0:tt , $k1:literal : $v1:tt , $k2:literal : $v2:tt , $k3:literal : $v3:tt , $k4:literal : $v4:tt , $k5:literal : $v5:tt , $k6:literal : $v6:tt , $k7:literal : $v7:tt , $k8:literal : $v8:tt , $k9:literal : $v9:tt , $k10:literal : $v10:tt , $k11:literal : $v11:tt , $k12:literal : $v12:tt , $k13:literal : $v13:tt , $k14:literal : $v14:tt , $k15:literal : $v15:tt , $($rest:tt)*) => {
        $crate::__json_object!([$($done,)*
            $crate::__private::member($k0, $crate::__json_value!($v0)),
            $crate::__private::member($k1, $crate::__json_value!($v1)),
            $crate::__private::member($k2, $crate::__json_value!($v2)),
            $crate::__private::member($k3, $crate::__json_value!($v3)),
            $crate::__private::member($k4, $crate::__json_value!($v4)),
            $crate::__private::member($k5, $crate::__json_value!($v5)),
            $crate::__private::member($k6, $crate::__json_value!($v6)),
            $crate::__private::member($k7, $crate::__json_value!($v7)),
            $crate::__private::member($k8, $crate::__json_value!($v8)),
            $crate::__private::member($k9, $crate::__json_value!($v9)),
            $crate::__private::member($k10, $crate::__json_value!($v10)),
            $crate::__private::member($k11, $crate::__json_value!($v11)),
            $crate::__private::member($k12, $crate::__json_value!($v12)),
            $crate::__private::member($k13, $crate::__json_value!($v13)),
            $crate::__private::member($k14, $crate::__json_value!($v14)),
            $crate::__private::member($k15, $crate::__json_value!($v15)),
        ] $($rest)*)
    };
    // Four members with literal keys and single-token values at once.
    ([$($done:expr,)*] $k1:literal : $v1:tt , $k2:literal : $v2:tt , $k3:literal : $v3:tt , $k4:literal : $v4:tt , $($rest:tt)*) => {
        $crate::__json_object!([$($done,)*
            $crate::__private::member($k1, $crate::__json_value!($v1)),
            $crate::__private::member($k2, $crate::__json_value!($v2)),
            $crate::__private::member($k3, $crate::__json_value!($v3)),
            $crate::__private::member($k4, $crate::__json_value!($v4)),
        ] $($rest)*)
    };
    // One such member.
    ([$($done:expr,)*] $k:literal : $v:tt , $($rest:tt)*) => {
        $crate::__json_object!([$($done,)* $crate::__private::member($k, $crate::__json_value!($v)),] $($rest)*)
    };
    // The last such member, without a trailing comma.
    ([$($done:expr,)*] $k:literal : $v:tt) => {
        $crate::__json_object!([$($done,)* $crate::__private::member($k, $crate::__json_value!($v)),])
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
    fn test_json_macro_mixes_fast_and_slow_items() {
        let n = 4;
        let v = json!([1, 2, 3, 4, 5, 6, 7, 8, -9, n, n + 1, "s", null, [true, false], { "a": -1 }, 10]);
        assert_eq!(
            v.to_string(),
            r#"[1,2,3,4,5,6,7,8,-9,4,5,"s",null,[true,false],{"a":-1},10]"#
        );
        let key = "k";
        let o = json!({ "a": 1, "b": [2], "c": { "d": null }, "e": n, (key): -n, "f": n * 2, "g": "x" });
        assert_eq!(
            o.to_string(),
            r#"{"a":1,"b":[2],"c":{"d":null},"e":4,"k":-4,"f":8,"g":"x"}"#
        );
    }

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
