//! Generators shared by the property tests: arbitrary documents, strings rich
//! in what escaping must handle, and token soup that lands near every
//! grammar boundary.

use jsonx_lib::{Number, Value};
use proptest::{prelude::*, sample::Index};

/// Any JSON number text, including ones too large or precise for any machine type.
pub fn number_text() -> impl Strategy<Value = String> {
    "-?(0|[1-9][0-9]{0,30})(\\.[0-9]{1,20})?([eE][+-]?[0-9]{1,5})?"
}

/// Strings rich in the characters escaping must handle.
pub fn tricky_string() -> impl Strategy<Value = String> {
    let ch = prop_oneof![
        3 => any::<char>(),
        1 => prop::sample::select(vec![
            '"', '\\', '/', '\u{0}', '\u{1}', '\u{8}', '\u{c}', '\n', '\r', '\t', '\u{1f}',
            '\u{7f}', '\u{2028}', '\u{2029}', '\u{2027}', '\u{20ac}', '\u{1f600}', 'u', 'a',
        ]),
    ];
    prop_oneof![
        prop::collection::vec(ch, 0..24).prop_map(String::from_iter),
        any::<String>(),
    ]
}

pub fn leaf() -> impl Strategy<Value = Value<'static>> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>().prop_map(Value::from),
        number_text().prop_map(|t| {
            Number::from_text(&t).map_or(Value::Null, |n| Value::Number(n.into_owned()))
        }),
        tricky_string().prop_map(Value::from),
    ]
}

/// Arbitrary documents. Object keys may repeat.
pub fn value() -> impl Strategy<Value = Value<'static>> {
    leaf().prop_recursive(6, 96, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(Value::from),
            prop::collection::vec((tricky_string(), inner), 0..8)
                .prop_map(|members| Value::Object(members.into_iter().collect())),
        ]
    })
}

/// Pieces that, glued at random, land near every grammar boundary.
pub const FRAGMENTS: &[&str] = &[
    "{",
    "}",
    "[",
    "]",
    ",",
    ":",
    "\"",
    "\\",
    "\"a\"",
    "\"k\":",
    "\"\\u00e9\"",
    "\"\\ud83d\\ude00\"",
    "\\ud800",
    "\\udc00",
    "\\u",
    "u",
    "0",
    "1",
    "9",
    "-",
    ".",
    "e",
    "E",
    "+",
    "true",
    "tru",
    "false",
    "null",
    "nul",
    " ",
    "\n",
    "\t",
    "\r",
    "\u{1}",
    "\u{e9}",
    "\u{2028}",
    "\u{feff}",
    "x",
    "/",
    "b",
    "n",
    "12",
    "0.5",
    "1e3",
    "\\n",
    "\\\"",
    "D800",
    "DC00",
    "dc00",
    "00",
];

pub fn soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..24).prop_map(|parts| parts.concat())
}

/// A valid document with one random edit at a character boundary.
pub fn mutated() -> impl Strategy<Value = String> {
    (
        value(),
        any::<Index>(),
        prop::sample::select(FRAGMENTS),
        0..3u8,
    )
        .prop_map(|(v, at, fragment, edit)| {
            let text = jsonx_lib::to_string(&v);
            let bounds: Vec<usize> = (0..=text.len())
                .filter(|&i| text.is_char_boundary(i))
                .collect();
            let i = bounds.get(at.index(bounds.len())).copied().unwrap_or(0);
            let (head, tail) = text.split_at(i);
            match edit {
                0 => format!("{head}{fragment}{tail}"),
                1 => {
                    let mut chars = tail.chars();
                    let _removed = chars.next();
                    format!("{head}{}", chars.as_str())
                }
                _ => head.to_owned(),
            }
        })
}
