//! A strict reference JSON parser, written independently of the crate,
//! straight from the RFC 8259 grammar.
//!
//! It is deliberately simple (recursive descent, one function per grammar
//! rule) and slow, and it reports more than accept/reject: where the input
//! stops being the beginning of any valid JSON text. That lets the tests check
//! the crate's error offsets, not just its verdicts.
//!
//! Unpaired UTF-16 surrogate escapes are allowed by the grammar but have no
//! Unicode meaning. The reference parses them grammatically and records the
//! first one separately, at the point where it becomes certain that the
//! escape is unpaired.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

pub mod strategies;

/// A parsed value, with numbers as their exact text.
#[derive(Clone, Debug, PartialEq)]
pub enum RefValue {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<RefValue>),
    Obj(Vec<(String, RefValue)>),
}

/// Why a parse stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// The input ended while the text so far was still a valid beginning.
    Eof,
    /// The byte at this offset cannot continue any valid JSON text.
    Dead(usize),
}

/// The grammatical result, and the offset of the first unpaired surrogate
/// escape found before parsing stopped.
pub struct Outcome {
    pub result: Result<RefValue, Fail>,
    pub lone_surrogate: Option<usize>,
}

/// Parses `input` as one complete JSON text.
pub fn parse(input: &[u8]) -> Outcome {
    let mut p = P {
        b: input,
        i: 0,
        lone: None,
    };
    let result = p.text();
    Outcome {
        result,
        lone_surrogate: p.lone,
    }
}

struct P<'b> {
    b: &'b [u8],
    i: usize,
    lone: Option<usize>,
}

impl P<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn dead<T>(&self) -> Result<T, Fail> {
        Err(Fail::Dead(self.i))
    }

    fn need(&self) -> Result<u8, Fail> {
        self.peek().ok_or(Fail::Eof)
    }

    fn text(&mut self) -> Result<RefValue, Fail> {
        self.ws();
        let v = self.value()?;
        self.ws();
        if self.i < self.b.len() {
            self.dead()
        } else {
            Ok(v)
        }
    }

    fn value(&mut self) -> Result<RefValue, Fail> {
        match self.need()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(RefValue::Str),
            b'-' | b'0'..=b'9' => self.number(),
            b't' => self.literal(b"true", RefValue::Bool(true)),
            b'f' => self.literal(b"false", RefValue::Bool(false)),
            b'n' => self.literal(b"null", RefValue::Null),
            _ => self.dead(),
        }
    }

    fn literal(&mut self, word: &[u8], v: RefValue) -> Result<RefValue, Fail> {
        for &w in word {
            if self.need()? != w {
                return self.dead();
            }
            self.i += 1;
        }
        Ok(v)
    }

    fn digit(&mut self) -> Result<(), Fail> {
        if self.need()?.is_ascii_digit() {
            self.i += 1;
            Ok(())
        } else {
            self.dead()
        }
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.i += 1;
        }
    }

    // number = [ minus ] int [ frac ] [ exp ]
    // int = zero / ( digit1-9 *DIGIT )
    fn number(&mut self) -> Result<RefValue, Fail> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.need()? {
            b'0' => self.i += 1,
            b'1'..=b'9' => {
                self.i += 1;
                self.digits();
            }
            _ => return self.dead(),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            self.digit()?;
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            self.digit()?;
            self.digits();
        }
        Ok(RefValue::Num(
            String::from_utf8(self.b[start..self.i].to_vec()).unwrap(),
        ))
    }

    fn hex4(&mut self) -> Result<u32, Fail> {
        let mut v = 0;
        for _ in 0..4 {
            let b = self.need()?;
            let d = (b as char).to_digit(16).ok_or(Fail::Dead(self.i))?;
            v = v * 16 + d;
            self.i += 1;
        }
        Ok(v)
    }

    fn flag(&mut self, at: usize) {
        if self.lone.is_none() {
            self.lone = Some(at);
        }
    }

    // string = quotation-mark *char quotation-mark
    fn string(&mut self) -> Result<String, Fail> {
        self.i += 1; // the opening quote
        let mut units: Vec<u16> = Vec::new();
        let mut out = String::new();
        // The offset of a high-surrogate escape not yet followed by its pair.
        let mut pending: Option<usize> = None;
        loop {
            let b = self.need()?;
            if b != b'\\' {
                if let Some(h) = pending.take() {
                    self.flag(h);
                }
            }
            match b {
                b'"' => {
                    self.i += 1;
                    flush(&mut units, &mut out);
                    return Ok(out);
                }
                b'\\' => {
                    let at = self.i;
                    self.i += 1;
                    let e = self.need()?;
                    let short = match e {
                        b'"' => Some('"'),
                        b'\\' => Some('\\'),
                        b'/' => Some('/'),
                        b'b' => Some('\u{8}'),
                        b'f' => Some('\u{c}'),
                        b'n' => Some('\n'),
                        b'r' => Some('\r'),
                        b't' => Some('\t'),
                        b'u' => None,
                        _ => return self.dead(),
                    };
                    self.i += 1;
                    if let Some(c) = short {
                        if let Some(h) = pending.take() {
                            self.flag(h);
                        }
                        flush(&mut units, &mut out);
                        out.push(c);
                        continue;
                    }
                    let unit = self.hex4()?;
                    let is_high = (0xD800..0xDC00).contains(&unit);
                    let is_low = (0xDC00..0xE000).contains(&unit);
                    match pending.take() {
                        Some(_) if is_low => {}
                        Some(h) => {
                            self.flag(h);
                            if is_low {
                                self.flag(at);
                            }
                        }
                        None if is_low => self.flag(at),
                        None => {}
                    }
                    if is_high {
                        pending = Some(at);
                    }
                    units.push(unit as u16);
                }
                0x00..=0x1F => return self.dead(),
                _ => {
                    flush(&mut units, &mut out);
                    // Copy one whole UTF-8 character (the input is valid UTF-8).
                    let len = match b {
                        0x00..=0x7F => 1,
                        0xC0..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        _ => 4,
                    };
                    let end = (self.i + len).min(self.b.len());
                    out.push_str(std::str::from_utf8(&self.b[self.i..end]).unwrap_or("\u{fffd}"));
                    self.i = end;
                }
            }
        }
    }

    // array = begin-array [ value *( value-separator value ) ] end-array
    fn array(&mut self) -> Result<RefValue, Fail> {
        self.i += 1;
        self.ws();
        let mut items = Vec::new();
        if self.need()? == b']' {
            self.i += 1;
            return Ok(RefValue::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.need()? {
                b',' => {
                    self.i += 1;
                    self.ws();
                }
                b']' => {
                    self.i += 1;
                    return Ok(RefValue::Arr(items));
                }
                _ => return self.dead(),
            }
        }
    }

    // object = begin-object [ member *( value-separator member ) ] end-object
    fn object(&mut self) -> Result<RefValue, Fail> {
        self.i += 1;
        self.ws();
        let mut members = Vec::new();
        if self.need()? == b'}' {
            self.i += 1;
            return Ok(RefValue::Obj(members));
        }
        loop {
            if self.need()? != b'"' {
                return self.dead();
            }
            let key = self.string()?;
            self.ws();
            if self.need()? != b':' {
                return self.dead();
            }
            self.i += 1;
            self.ws();
            let v = self.value()?;
            members.push((key, v));
            self.ws();
            match self.need()? {
                b',' => {
                    self.i += 1;
                    self.ws();
                }
                b'}' => {
                    self.i += 1;
                    return Ok(RefValue::Obj(members));
                }
                _ => return self.dead(),
            }
        }
    }
}

/// Decodes collected UTF-16 units into `out` (lone surrogates as U+FFFD).
fn flush(units: &mut Vec<u16>, out: &mut String) {
    out.extend(char::decode_utf16(units.drain(..)).map(|r| r.unwrap_or('\u{fffd}')));
}

/// Converts the crate's value into the reference representation.
pub fn to_ref(v: &jsonx_lib::Value<'_>) -> RefValue {
    use jsonx_lib::Value;
    match v {
        Value::Null => RefValue::Null,
        Value::Bool(b) => RefValue::Bool(*b),
        Value::Number(n) => RefValue::Num(n.as_str().to_owned()),
        Value::String(s) => RefValue::Str(s.to_string()),
        Value::Array(a) => RefValue::Arr(a.iter().map(to_ref).collect()),
        Value::Object(o) => {
            RefValue::Obj(o.iter().map(|(k, v)| (k.to_owned(), to_ref(v))).collect())
        }
    }
}

#[test]
fn reference_self_check() {
    let ok = |s: &str| parse(s.as_bytes()).result.is_ok();
    assert!(ok(r#"{"a":[1,-2.5e+3,true,false,null,"x"]}"#));
    assert!(!ok("[1,]"));
    assert_eq!(parse(b"[1,").result, Err(Fail::Eof));
    assert_eq!(parse(b"[1 2]").result, Err(Fail::Dead(3)));
    assert_eq!(parse("\"\\ud800\"".as_bytes()).lone_surrogate, Some(1));
    assert_eq!(parse("\"\\ud83d\\ude00\"".as_bytes()).lone_surrogate, None);
    assert_eq!(
        parse("\"\\ud83d\\ude00\"".as_bytes()).result,
        Ok(RefValue::Str("\u{1f600}".into()))
    );
}
