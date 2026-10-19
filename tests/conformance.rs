//! Conformance cases written as data from RFC 8259, named after the
//! JSONTestSuite convention: `y_` must be accepted, `n_` must be refused, and
//! `i_` is left to the implementation by the RFC (the expected outcome records
//! this crate's choice). Every `n_` case also pins the error kind and the byte
//! offset, and every case is cross-checked against the independent reference
//! parser in `tests/common`.

mod common;

use common::{Fail, RefValue};
use jsonx_lib::{ErrorKind, Event, Limits, Parser, Reader};

/// Inputs that must parse.
const Y: &[(&str, &str)] = &[
    ("y_array_arraysWithSpaces", "[[]   ]"),
    ("y_array_empty", "[]"),
    ("y_array_empty-string", "[\"\"]"),
    ("y_array_false", "[false]"),
    ("y_array_heterogeneous", "[null, 1, \"1\", {}]"),
    ("y_array_null", "[null]"),
    ("y_array_with_1_and_newline", "[1\n]"),
    ("y_array_with_leading_space", " [1]"),
    ("y_array_with_several_null", "[1,null,null,null,2]"),
    ("y_array_with_trailing_space", "[2] "),
    ("y_number", "[123e65]"),
    ("y_number_0e+1", "[0e+1]"),
    ("y_number_0e1", "[0e1]"),
    ("y_number_after_space", "[ 4]"),
    (
        "y_number_double_close_to_zero",
        "[-0.000000000000000000000000000000000000000000000000000000000000000000000000000001]",
    ),
    ("y_number_int_with_exp", "[20e1]"),
    ("y_number_minus_zero", "[-0]"),
    ("y_number_negative_int", "[-123]"),
    ("y_number_negative_one", "[-1]"),
    ("y_number_real_capital_e", "[1E22]"),
    ("y_number_real_capital_e_neg_exp", "[1E-2]"),
    ("y_number_real_capital_e_pos_exp", "[1E+2]"),
    ("y_number_real_exponent", "[123e45]"),
    ("y_number_real_fraction_exponent", "[123.456e78]"),
    ("y_number_real_neg_exp", "[1e-2]"),
    ("y_number_real_pos_exponent", "[1e+2]"),
    ("y_number_simple_int", "[123]"),
    ("y_number_simple_real", "[123.456789]"),
    ("y_number_zero_fraction", "[0.0]"),
    ("y_object", "{\"asd\":\"sdf\", \"dfg\":\"fgh\"}"),
    ("y_object_basic", "{\"asd\":\"sdf\"}"),
    ("y_object_empty", "{}"),
    ("y_object_empty_key", "{\"\":0}"),
    ("y_object_escaped_null_in_key", "{\"foo\\u0000bar\": 42}"),
    (
        "y_object_extreme_numbers",
        "{ \"min\": -1.0e+28, \"max\": 1.0e+28 }",
    ),
    ("y_object_nested", "{\"a\":{\"b\":{\"c\":[{}]}}}"),
    ("y_object_simple", "{\"a\":[]}"),
    (
        "y_object_string_unicode",
        "{\"title\":\"\\u041f\\u043e\\u043b\\u0442\\u043e\\u0440\\u0430\" }",
    ),
    ("y_object_with_newlines", "{\n\"a\": \"b\"\n}"),
    (
        "y_string_1_2_3_bytes_UTF-8_sequences",
        "[\"\\u0060\\u012a\\u12AB\"]",
    ),
    ("y_string_accepted_surrogate_pair", "[\"\\uD801\\udc37\"]"),
    (
        "y_string_accepted_surrogate_pairs",
        "[\"\\ud83d\\ude39\\ud83d\\udc8d\"]",
    ),
    (
        "y_string_allowed_escapes",
        "[\"\\\"\\\\\\/\\b\\f\\n\\r\\t\"]",
    ),
    ("y_string_backslash_and_u_escaped_zero", "[\"\\\\u0000\"]"),
    ("y_string_backslash_doublequotes", "[\"\\\"\"]"),
    ("y_string_comments", "[\"a/*b*/c/*d//e\"]"),
    ("y_string_double_escape_a", "[\"\\\\a\"]"),
    ("y_string_double_escape_n", "[\"\\\\n\"]"),
    ("y_string_escaped_control_character", "[\"\\u0012\"]"),
    ("y_string_escaped_noncharacter", "[\"\\uFFFF\"]"),
    ("y_string_in_array", "[\"asd\"]"),
    ("y_string_last_surrogates_1_and_2", "[\"\\uDBFF\\uDFFF\"]"),
    ("y_string_nonCharacterInUTF-8_U+FFFF", "[\"\u{ffff}\"]"),
    ("y_string_null_escape", "[\"\\u0000\"]"),
    ("y_string_pi", "[\"\u{3c0}\"]"),
    ("y_string_simple_ascii", "[\"asd \"]"),
    ("y_string_space", "\" \""),
    ("y_string_uEscape", "[\"\\u0061\\u30af\\u30EA\\u30b9\"]"),
    ("y_string_unicode_U+2028_line_sep", "[\"\u{2028}\"]"),
    ("y_string_unicode_U+2029_par_sep", "[\"\u{2029}\"]"),
    ("y_string_utf8", "[\"\u{20ac}\u{1d11e}\"]"),
    ("y_string_with_del_character", "[\"a\u{7f}a\"]"),
    ("y_structure_lonely_false", "false"),
    ("y_structure_lonely_int", "42"),
    ("y_structure_lonely_negative_real", "-0.1"),
    ("y_structure_lonely_null", "null"),
    ("y_structure_lonely_string", "\"asd\""),
    ("y_structure_lonely_true", "true"),
    ("y_structure_string_empty", "\"\""),
    ("y_structure_trailing_newline", "[\"a\"]\n"),
    ("y_structure_true_in_array", "[true]"),
    ("y_structure_whitespace_array", " [] "),
    (
        "y_structure_whitespace_everywhere",
        "\t\r\n { \t\"a\"\r\n:\n[ 1 ,\t2 ]\r} \n",
    ),
];

/// Inputs that must be refused: the error kind and its byte offset.
const N: &[(&str, &[u8], ErrorKind, usize)] = &[
    (
        "n_array_1_true_without_comma",
        b"[1 true]",
        ErrorKind::ExpectedCommaOrBracket,
        3,
    ),
    (
        "n_array_colon_instead_of_comma",
        b"[\"\": 1]",
        ErrorKind::ExpectedCommaOrBracket,
        3,
    ),
    (
        "n_array_comma_and_number",
        b"[,1]",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_array_double_comma",
        b"[1,,2]",
        ErrorKind::ExpectedValue,
        3,
    ),
    (
        "n_array_extra_close",
        b"[\"x\"]]",
        ErrorKind::TrailingCharacters,
        5,
    ),
    (
        "n_array_extra_comma",
        b"[\"\",]",
        ErrorKind::TrailingComma,
        4,
    ),
    ("n_array_incomplete", b"[\"x\"", ErrorKind::UnexpectedEnd, 4),
    (
        "n_array_incomplete_invalid_value",
        b"[x",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_array_inner_array_no_comma",
        b"[3[4]]",
        ErrorKind::ExpectedCommaOrBracket,
        2,
    ),
    (
        "n_array_items_separated_by_semicolon",
        b"[1:2]",
        ErrorKind::ExpectedCommaOrBracket,
        2,
    ),
    ("n_array_just_comma", b"[,]", ErrorKind::ExpectedValue, 1),
    ("n_array_just_minus", b"[-]", ErrorKind::InvalidNumber, 2),
    (
        "n_array_missing_value",
        b"[   , \"\"]",
        ErrorKind::ExpectedValue,
        4,
    ),
    (
        "n_array_number_and_comma",
        b"[1,]",
        ErrorKind::TrailingComma,
        3,
    ),
    (
        "n_array_number_and_several_commas",
        b"[1,,]",
        ErrorKind::ExpectedValue,
        3,
    ),
    ("n_array_star_inside", b"[*]", ErrorKind::ExpectedValue, 1),
    ("n_array_unclosed", b"[\"\"", ErrorKind::UnexpectedEnd, 3),
    (
        "n_array_unclosed_trailing_comma",
        b"[1,",
        ErrorKind::UnexpectedEnd,
        3,
    ),
    (
        "n_array_unclosed_with_object_inside",
        b"[{}",
        ErrorKind::UnexpectedEnd,
        3,
    ),
    (
        "n_incomplete_false",
        b"[fals]",
        ErrorKind::InvalidLiteral,
        5,
    ),
    ("n_incomplete_null", b"[nul]", ErrorKind::InvalidLiteral, 4),
    ("n_incomplete_true", b"[tru]", ErrorKind::InvalidLiteral, 4),
    (
        "n_multidigit_number_then_00",
        b"123\x00",
        ErrorKind::TrailingCharacters,
        3,
    ),
    ("n_number_++", b"[++1234]", ErrorKind::ExpectedValue, 1),
    ("n_number_+1", b"[+1]", ErrorKind::ExpectedValue, 1),
    ("n_number_-01", b"[-01]", ErrorKind::InvalidNumber, 3),
    (
        "n_number_-1.0.",
        b"[-1.0.]",
        ErrorKind::ExpectedCommaOrBracket,
        5,
    ),
    ("n_number_.-1", b"[.-1]", ErrorKind::ExpectedValue, 1),
    ("n_number_0.e1", b"[0.e1]", ErrorKind::InvalidNumber, 3),
    ("n_number_0_capital_E", b"[0E]", ErrorKind::InvalidNumber, 3),
    (
        "n_number_0_capital_E+",
        b"[0E+]",
        ErrorKind::InvalidNumber,
        4,
    ),
    ("n_number_0e+", b"[0e+]", ErrorKind::InvalidNumber, 4),
    ("n_number_1.0e", b"[1.0e]", ErrorKind::InvalidNumber, 5),
    ("n_number_1.0e-", b"[1.0e-]", ErrorKind::InvalidNumber, 6),
    (
        "n_number_1_000",
        b"[1 000.0]",
        ErrorKind::ExpectedCommaOrBracket,
        3,
    ),
    ("n_number_1eE2", b"[1eE2]", ErrorKind::InvalidNumber, 3),
    ("n_number_2.e3", b"[2.e3]", ErrorKind::InvalidNumber, 3),
    ("n_number_9.e+", b"[9.e+]", ErrorKind::InvalidNumber, 3),
    ("n_number_Inf", b"[Inf]", ErrorKind::ExpectedValue, 1),
    ("n_number_NaN", b"[NaN]", ErrorKind::ExpectedValue, 1),
    (
        "n_number_U+FF11_fullwidth_digit_one",
        "[\u{ff11}]".as_bytes(),
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_number_hex_1_digit",
        b"[0x1]",
        ErrorKind::ExpectedCommaOrBracket,
        2,
    ),
    (
        "n_number_infinity",
        b"[Infinity]",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_number_minus_infinity",
        b"[-Infinity]",
        ErrorKind::InvalidNumber,
        2,
    ),
    (
        "n_number_minus_space_1",
        b"[- 1]",
        ErrorKind::InvalidNumber,
        2,
    ),
    (
        "n_number_neg_int_starting_with_zero",
        b"[-012]",
        ErrorKind::InvalidNumber,
        3,
    ),
    (
        "n_number_neg_real_without_int_part",
        b"[-.123]",
        ErrorKind::InvalidNumber,
        2,
    ),
    (
        "n_number_real_garbage_after_e",
        b"[1ea]",
        ErrorKind::InvalidNumber,
        3,
    ),
    (
        "n_number_real_without_fractional_part",
        b"[1.]",
        ErrorKind::InvalidNumber,
        3,
    ),
    (
        "n_number_starting_with_dot",
        b"[.123]",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_number_with_leading_zero",
        b"[012]",
        ErrorKind::InvalidNumber,
        2,
    ),
    (
        "n_object_bad_value",
        b"[\"x\", truth]",
        ErrorKind::InvalidLiteral,
        9,
    ),
    (
        "n_object_comma_instead_of_colon",
        b"{\"x\", null}",
        ErrorKind::ExpectedColon,
        4,
    ),
    (
        "n_object_double_colon",
        b"{\"x\"::\"b\"}",
        ErrorKind::ExpectedValue,
        5,
    ),
    (
        "n_object_emoji",
        "{\u{1f1e8}\u{1f1ed}}".as_bytes(),
        ErrorKind::ExpectedKey,
        1,
    ),
    (
        "n_object_garbage_at_end",
        b"{\"a\":\"a\" 123}",
        ErrorKind::ExpectedCommaOrBrace,
        9,
    ),
    (
        "n_object_key_with_single_quotes",
        b"{key: 'value'}",
        ErrorKind::ExpectedKey,
        1,
    ),
    (
        "n_object_missing_colon",
        b"{\"a\" b}",
        ErrorKind::ExpectedColon,
        5,
    ),
    (
        "n_object_missing_key",
        b"{:\"b\"}",
        ErrorKind::ExpectedKey,
        1,
    ),
    (
        "n_object_missing_semicolon",
        b"{\"a\" \"b\"}",
        ErrorKind::ExpectedColon,
        5,
    ),
    (
        "n_object_missing_value",
        b"{\"a\":",
        ErrorKind::UnexpectedEnd,
        5,
    ),
    ("n_object_no-colon", b"{\"a\"", ErrorKind::UnexpectedEnd, 4),
    (
        "n_object_non_string_key",
        b"{1:1}",
        ErrorKind::ExpectedKey,
        1,
    ),
    (
        "n_object_repeated_null_null",
        b"{null:null,null:null}",
        ErrorKind::ExpectedKey,
        1,
    ),
    (
        "n_object_single_quote",
        b"{'a':0}",
        ErrorKind::ExpectedKey,
        1,
    ),
    (
        "n_object_trailing_comma",
        b"{\"id\":0,}",
        ErrorKind::TrailingComma,
        8,
    ),
    (
        "n_object_trailing_comment",
        b"{\"a\":\"b\"}/**/",
        ErrorKind::TrailingCharacters,
        9,
    ),
    (
        "n_object_two_commas_in_a_row",
        b"{\"a\":\"b\",,\"c\":\"d\"}",
        ErrorKind::ExpectedKey,
        9,
    ),
    (
        "n_object_unquoted_key",
        b"{a: \"b\"}",
        ErrorKind::ExpectedKey,
        1,
    ),
    (
        "n_object_with_single_string",
        b"{ \"foo\" : \"bar\", \"a\" }",
        ErrorKind::ExpectedColon,
        21,
    ),
    (
        "n_object_with_trailing_garbage",
        b"{\"a\": true} \"x\"",
        ErrorKind::TrailingCharacters,
        12,
    ),
    ("n_single_space", b" ", ErrorKind::UnexpectedEnd, 1),
    (
        "n_string_1_surrogate_then_escape",
        b"[\"\\uD800\\\"]",
        ErrorKind::LoneSurrogate,
        2,
    ),
    (
        "n_string_escape_x",
        b"[\"\\x00\"]",
        ErrorKind::InvalidEscape,
        3,
    ),
    (
        "n_string_escaped_backslash_bad",
        b"[\"\\\\\\\"]",
        ErrorKind::UnexpectedEnd,
        7,
    ),
    (
        "n_string_escaped_ctrl_char_tab",
        b"[\"\\\t\"]",
        ErrorKind::InvalidEscape,
        3,
    ),
    (
        "n_string_escaped_emoji",
        "[\"\\\u{1f300}\"]".as_bytes(),
        ErrorKind::InvalidEscape,
        3,
    ),
    (
        "n_string_incomplete_escape",
        b"[\"\\\"]",
        ErrorKind::UnexpectedEnd,
        5,
    ),
    (
        "n_string_incomplete_escaped_character",
        b"[\"\\u00A\"]",
        ErrorKind::InvalidUnicodeEscape,
        7,
    ),
    (
        "n_string_incomplete_surrogate",
        b"[\"\\uD834\\uDd\"]",
        ErrorKind::InvalidUnicodeEscape,
        12,
    ),
    (
        "n_string_invalid_backslash_esc",
        b"[\"\\a\"]",
        ErrorKind::InvalidEscape,
        3,
    ),
    (
        "n_string_invalid_unicode_escape",
        b"[\"\\uqqqq\"]",
        ErrorKind::InvalidUnicodeEscape,
        4,
    ),
    (
        "n_string_invalid_utf8_after_escape",
        b"[\"\\u0041\xe5\"]",
        ErrorKind::InvalidUtf8,
        8,
    ),
    (
        "n_string_no_quotes_with_bad_escape",
        b"[\\n]",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_string_single_doublequote",
        b"\"",
        ErrorKind::UnexpectedEnd,
        1,
    ),
    (
        "n_string_single_quote",
        b"['single quote']",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_string_single_string_no_double_quotes",
        b"abc",
        ErrorKind::ExpectedValue,
        0,
    ),
    (
        "n_string_start_escape_unclosed",
        b"[\"\\",
        ErrorKind::UnexpectedEnd,
        3,
    ),
    (
        "n_string_unescaped_ctrl_char",
        b"[\"a\x00a\"]",
        ErrorKind::ControlCharacter,
        3,
    ),
    (
        "n_string_unescaped_newline",
        b"[\"new\nline\"]",
        ErrorKind::ControlCharacter,
        5,
    ),
    (
        "n_string_unescaped_tab",
        b"[\"\t\"]",
        ErrorKind::ControlCharacter,
        2,
    ),
    (
        "n_string_unicode_CapitalU",
        b"\"\\UA66D\"",
        ErrorKind::InvalidEscape,
        2,
    ),
    (
        "n_string_with_trailing_garbage",
        b"\"\"x",
        ErrorKind::TrailingCharacters,
        2,
    ),
    (
        "n_structure_U+2060_word_joined",
        "[\u{2060}]".as_bytes(),
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_structure_UTF8_BOM_no_data",
        b"\xEF\xBB\xBF",
        ErrorKind::ExpectedValue,
        0,
    ),
    (
        "n_structure_angle_bracket_.",
        b"<.>",
        ErrorKind::ExpectedValue,
        0,
    ),
    (
        "n_structure_angle_bracket_null",
        b"[<null>]",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_structure_array_trailing_garbage",
        b"[1]x",
        ErrorKind::TrailingCharacters,
        3,
    ),
    (
        "n_structure_array_with_extra_array_close",
        b"[1]]",
        ErrorKind::TrailingCharacters,
        3,
    ),
    (
        "n_structure_array_with_unclosed_string",
        b"[\"asd]",
        ErrorKind::UnexpectedEnd,
        6,
    ),
    (
        "n_structure_ascii-unicode-identifier",
        "a\u{e5}".as_bytes(),
        ErrorKind::ExpectedValue,
        0,
    ),
    (
        "n_structure_capitalized_True",
        b"[True]",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_structure_close_unopened_array",
        b"1]",
        ErrorKind::TrailingCharacters,
        1,
    ),
    (
        "n_structure_comma_instead_of_closing_brace",
        b"{\"x\": true,",
        ErrorKind::UnexpectedEnd,
        11,
    ),
    (
        "n_structure_double_array",
        b"[][]",
        ErrorKind::TrailingCharacters,
        2,
    ),
    ("n_structure_end_array", b"]", ErrorKind::ExpectedValue, 0),
    (
        "n_structure_lone-open-bracket",
        b"[",
        ErrorKind::UnexpectedEnd,
        1,
    ),
    ("n_structure_no_data", b"", ErrorKind::UnexpectedEnd, 0),
    (
        "n_structure_null-byte-outside-string",
        b"[\x00]",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_structure_number_with_trailing_garbage",
        b"2@",
        ErrorKind::TrailingCharacters,
        1,
    ),
    (
        "n_structure_object_followed_by_closing_object",
        b"{}}",
        ErrorKind::TrailingCharacters,
        2,
    ),
    (
        "n_structure_object_unclosed_no_value",
        b"{\"\":",
        ErrorKind::UnexpectedEnd,
        4,
    ),
    (
        "n_structure_object_with_comment",
        b"{\"a\":/*comment*/\"b\"}",
        ErrorKind::ExpectedValue,
        5,
    ),
    (
        "n_structure_object_with_trailing_garbage",
        b"{\"a\": true} \"x\"",
        ErrorKind::TrailingCharacters,
        12,
    ),
    (
        "n_structure_open_array_apostrophe",
        b"['",
        ErrorKind::ExpectedValue,
        1,
    ),
    ("n_structure_open_object", b"{", ErrorKind::UnexpectedEnd, 1),
    (
        "n_structure_open_object_close_array",
        b"{]",
        ErrorKind::ExpectedKey,
        1,
    ),
    (
        "n_structure_open_object_open_array",
        b"{[",
        ErrorKind::ExpectedKey,
        1,
    ),
    ("n_structure_single_star", b"*", ErrorKind::ExpectedValue, 0),
    (
        "n_structure_trailing_#",
        b"{\"a\":\"b\"}#{}",
        ErrorKind::TrailingCharacters,
        9,
    ),
    (
        "n_structure_uescaped_LF_before_string",
        b"[\\u000A\"\"]",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_structure_unclosed_array_partial_null",
        b"[ false, nul",
        ErrorKind::UnexpectedEnd,
        12,
    ),
    (
        "n_structure_unclosed_array_unfinished_true",
        b"[ false, tru",
        ErrorKind::UnexpectedEnd,
        12,
    ),
    (
        "n_structure_unclosed_object",
        b"{\"asd\":\"asd\"",
        ErrorKind::UnexpectedEnd,
        12,
    ),
    (
        "n_structure_whitespace_formfeed",
        b"[\x0c]",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_structure_whitespace_vertical_tab",
        b"[\x0b]",
        ErrorKind::ExpectedValue,
        1,
    ),
    (
        "n_structure_whitespace_nbsp",
        "[\u{a0}1]".as_bytes(),
        ErrorKind::ExpectedValue,
        1,
    ),
];

/// What this crate does with an input the RFC leaves to the implementation.
#[derive(Clone, Copy, Debug)]
enum Expect {
    Accept,
    Reject(ErrorKind, usize),
}

/// Implementation-defined inputs, and this crate's choice for each.
const I: &[(&str, &[u8], Expect)] = &[
    // Numbers beyond any machine type are accepted and kept as exact text
    // (RFC 8259 §6 allows limits; this crate imposes none on value).
    ("i_number_double_huge_neg_exp", b"[123.456e-789]", Expect::Accept),
    ("i_number_huge_exp", b"[0.4e00669999999999999999999999999999999999999999999999999999999999999999999999999999999999999]", Expect::Accept),
    ("i_number_neg_int_huge_exp", b"[-1e+9999]", Expect::Accept),
    ("i_number_pos_double_huge_exp", b"[1.5e+9999]", Expect::Accept),
    ("i_number_real_neg_overflow", b"[-123123e100000]", Expect::Accept),
    ("i_number_real_pos_overflow", b"[123123e100000]", Expect::Accept),
    ("i_number_real_underflow", b"[123e-10000000]", Expect::Accept),
    ("i_number_too_big_neg_int", b"[-123123123123123123123123123123]", Expect::Accept),
    ("i_number_too_big_pos_int", b"[100000000000000000000]", Expect::Accept),
    ("i_number_very_big_negative_int", b"[-237462374673276894279832749832423479823246327846]", Expect::Accept),
    // Unpaired surrogate escapes decode to no Unicode text: refused at the
    // backslash of the unpaired escape (§8.2 permits either choice).
    ("i_object_key_lone_2nd_surrogate", b"{\"\\uDFAA\":0}", Expect::Reject(ErrorKind::LoneSurrogate, 2)),
    ("i_string_1st_surrogate_but_2nd_missing", b"[\"\\uDADA\"]", Expect::Reject(ErrorKind::LoneSurrogate, 2)),
    ("i_string_1st_valid_surrogate_2nd_invalid", b"[\"\\uD888\\u1234\"]", Expect::Reject(ErrorKind::LoneSurrogate, 2)),
    ("i_string_incomplete_surrogate_and_escape_valid", b"[\"\\uD800\\n\"]", Expect::Reject(ErrorKind::LoneSurrogate, 2)),
    ("i_string_incomplete_surrogate_pair", b"[\"\\uDd1ea\"]", Expect::Reject(ErrorKind::LoneSurrogate, 2)),
    ("i_string_incomplete_surrogates_escape_valid", b"[\"\\uD800\\uD800\\n\"]", Expect::Reject(ErrorKind::LoneSurrogate, 2)),
    ("i_string_invalid_lonely_surrogate", b"[\"\\ud800\"]", Expect::Reject(ErrorKind::LoneSurrogate, 2)),
    ("i_string_invalid_surrogate", b"[\"\\ud800abc\"]", Expect::Reject(ErrorKind::LoneSurrogate, 2)),
    ("i_string_inverted_surrogates_U+1D11E", b"[\"\\uDd1e\\uD834\"]", Expect::Reject(ErrorKind::LoneSurrogate, 2)),
    ("i_string_lone_second_surrogate", b"[\"\\uDFAA\"]", Expect::Reject(ErrorKind::LoneSurrogate, 2)),
    // Input must be UTF-8 (§8.1): other encodings and invalid sequences are
    // refused at the first bad byte.
    ("i_string_UTF-16LE_with_BOM", b"\xff\xfe[\x00\"\x00\xe9\x00\"\x00]\x00", Expect::Reject(ErrorKind::InvalidUtf8, 0)),
    ("i_string_UTF-8_invalid_sequence", b"[\"\xe6\x97\xa5\xd1\x88\xfa\"]", Expect::Reject(ErrorKind::InvalidUtf8, 7)),
    ("i_string_UTF8_surrogate_U+D800", b"[\"\xed\xa0\x80\"]", Expect::Reject(ErrorKind::InvalidUtf8, 2)),
    ("i_string_invalid_utf-8", b"[\"\xff\"]", Expect::Reject(ErrorKind::InvalidUtf8, 2)),
    ("i_string_iso_latin_1", b"[\"\xe9\"]", Expect::Reject(ErrorKind::InvalidUtf8, 2)),
    ("i_string_lone_utf8_continuation_byte", b"[\"\x81\"]", Expect::Reject(ErrorKind::InvalidUtf8, 2)),
    ("i_string_not_in_unicode_range", b"[\"\xf4\xbf\xbf\xbf\"]", Expect::Reject(ErrorKind::InvalidUtf8, 2)),
    ("i_string_overlong_sequence_2_bytes", b"[\"\xc0\xaf\"]", Expect::Reject(ErrorKind::InvalidUtf8, 2)),
    ("i_string_truncated-utf-8", b"[\"\xe0\xff\"]", Expect::Reject(ErrorKind::InvalidUtf8, 2)),
    ("i_string_utf16BE_no_BOM", b"\x00[\x00\"\x00\xe9\x00\"\x00]", Expect::Reject(ErrorKind::InvalidUtf8, 5)),
    // A byte-order mark is not whitespace; RFC 8259 §8.1 lets a parser ignore
    // one, and this crate does not.
    ("i_structure_UTF-8_BOM_empty_object", b"\xEF\xBB\xBF{}", Expect::Reject(ErrorKind::ExpectedValue, 0)),
    // Nesting beyond the default depth budget (128) is refused at the
    // bracket that goes one level too deep.
    ("i_structure_500_nested_arrays", NESTED_500, Expect::Reject(ErrorKind::DepthLimitExceeded, 128)),
    // Duplicate names: §4 says names SHOULD be unique; refused by default.
    ("i_object_duplicated_key", b"{\"a\":\"b\",\"a\":\"c\"}", Expect::Reject(ErrorKind::DuplicateKey, 9)),
    ("i_object_duplicated_key_and_value", b"{\"a\":\"b\",\"a\":\"b\"}", Expect::Reject(ErrorKind::DuplicateKey, 9)),
];

const NESTED_500: &[u8] = &{
    let mut b = [b']'; 1000];
    let mut i = 0;
    while i < 500 {
        b[i] = b'[';
        i += 1;
    }
    b
};

/// Runs a document through the event reader to its end.
fn read_all(bytes: &[u8]) -> Result<usize, jsonx_lib::Error> {
    let mut reader = Parser::new().reader_bytes(bytes)?;
    let mut events = 0;
    while let Some(_event) = reader.next_event()? {
        events += 1;
    }
    Ok(events)
}

/// The reference parser's verdict must agree with an expected error.
fn check_reference(name: &str, bytes: &[u8], kind: ErrorKind, offset: usize) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        assert_eq!(kind, ErrorKind::InvalidUtf8, "{name}");
        return;
    };
    let outcome = common::parse(text.as_bytes());
    match kind {
        ErrorKind::LoneSurrogate => assert_eq!(outcome.lone_surrogate, Some(offset), "{name}"),
        ErrorKind::UnexpectedEnd => {
            assert_eq!(offset, bytes.len(), "{name}");
            assert_eq!(outcome.result, Err(Fail::Eof), "{name}");
        }
        ErrorKind::DuplicateKey | ErrorKind::DepthLimitExceeded => {
            assert!(
                outcome.result.is_ok(),
                "{name}: valid grammar, refused by policy"
            );
        }
        _ => assert_eq!(outcome.result, Err(Fail::Dead(offset)), "{name}"),
    }
}

#[test]
fn test_y_cases_are_accepted_and_match_the_reference() {
    for &(name, text) in Y {
        let value = jsonx_lib::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let reference = common::parse(text.as_bytes());
        assert_eq!(reference.lone_surrogate, None, "{name}");
        assert_eq!(
            reference.result.as_ref().ok(),
            Some(&common::to_ref(&value)),
            "{name}"
        );
        assert!(read_all(text.as_bytes()).is_ok(), "{name}");
        // And it writes back to text that parses to the same value.
        let again = jsonx_lib::to_string(&value);
        assert_eq!(jsonx_lib::parse(&again).ok(), Some(value.clone()), "{name}");
    }
}

#[test]
fn test_n_cases_are_refused_at_the_exact_offset() {
    for &(name, bytes, kind, offset) in N {
        let err = Parser::new().parse_bytes(bytes).err();
        let got = err.map(|e| (e.kind(), e.offset()));
        assert_eq!(got, Some((kind, offset)), "{name}");
        // The event reader reports the same error.
        let streamed = read_all(bytes).err().map(|e| (e.kind(), e.offset()));
        assert_eq!(streamed, Some((kind, offset)), "{name} (reader)");
        check_reference(name, bytes, kind, offset);
    }
}

#[test]
fn test_i_cases_follow_the_documented_choice() {
    for &(name, bytes, expect) in I {
        let got = Parser::new().parse_bytes(bytes);
        match expect {
            Expect::Accept => {
                assert!(got.is_ok(), "{name}: {got:?}");
                let text = std::str::from_utf8(bytes).unwrap_or_default();
                let reference = common::parse(text.as_bytes()).result;
                assert_eq!(
                    reference.ok(),
                    got.as_ref().ok().map(common::to_ref),
                    "{name}"
                );
            }
            Expect::Reject(kind, offset) => {
                let got = got.err().map(|e| (e.kind(), e.offset()));
                assert_eq!(got, Some((kind, offset)), "{name}");
                check_reference(name, bytes, kind, offset);
            }
        }
    }
}

#[test]
fn test_i_numbers_keep_their_text_and_convert_exactly() {
    let v = jsonx_lib::parse("[1.5e+9999, 123e-10000000, -123123123123123123123123123123]")
        .unwrap_or_default();
    assert_eq!(v[0].as_number().map(|n| n.as_str()), Some("1.5e+9999"));
    assert_eq!(v[0].as_f64(), None, "too large to be finite");
    assert_eq!(v[1].as_f64(), Some(0.0), "underflows to zero");
    assert_eq!(v[2].as_i64(), None);
    assert_eq!(v[2].as_f64(), Some(-1.2312312312312312e29));
    assert_eq!(
        jsonx_lib::to_string(&v),
        "[1.5e+9999,123e-10000000,-123123123123123123123123123123]"
    );
}

#[test]
fn test_i_deep_nesting_larger_budget_is_accepted() {
    let p = Parser::new().limits(Limits::new().with_max_depth(500));
    assert!(p.parse_bytes(NESTED_500).is_ok());
    let p = Parser::new().limits(Limits::new().with_max_depth(499));
    let err = p
        .parse_bytes(NESTED_500)
        .err()
        .map(|e| (e.kind(), e.offset()));
    assert_eq!(err, Some((ErrorKind::DepthLimitExceeded, 499)));
}

#[test]
fn test_decoded_strings_escapes_decode_exactly() {
    let cases: &[(&str, &str)] = &[
        ("[\"\\uD801\\udc37\"]", "\u{10437}"),
        ("[\"\\\"\\\\\\/\\b\\f\\n\\r\\t\"]", "\"\\/\u{8}\u{c}\n\r\t"),
        ("[\"\\\\u0000\"]", "\\u0000"),
        ("[\"\\u0000\"]", "\u{0}"),
        (
            "[\"\\u0061\\u30af\\u30EA\\u30b9\"]",
            "a\u{30af}\u{30ea}\u{30b9}",
        ),
        ("[\"\\uDBFF\\uDFFF\"]", "\u{10ffff}"),
    ];
    for &(text, want) in cases {
        let v = jsonx_lib::parse(text).unwrap_or_default();
        assert_eq!(v[0].as_str(), Some(want), "{text}");
    }
}

#[test]
fn test_reader_events_mixed_document_come_in_order() {
    let events: Result<Vec<Event<'_>>, _> =
        Reader::new("{\"a\":[1,\"x\",null],\"b\":{}}").collect();
    let events = events.unwrap_or_default();
    assert_eq!(events.len(), 11);
    assert_eq!(events.first(), Some(&Event::StartObject));
    assert_eq!(events.get(4), Some(&Event::String("x".into())));
    assert_eq!(events.last(), Some(&Event::EndObject));
}

#[test]
fn test_reference_y_values_have_the_same_structure() {
    // A spot check that the reference really decodes, not just accepts.
    let r = common::parse(b"{\"k\":[true,\"v\"]}").result;
    assert_eq!(
        r,
        Ok(RefValue::Obj(vec![(
            "k".into(),
            RefValue::Arr(vec![RefValue::Bool(true), RefValue::Str("v".into())])
        )]))
    );
}
