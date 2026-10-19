//! The grammar state machine behind both readers.
//!
//! [`Machine`] holds everything a reader knows about where it is in a
//! document (the grammar state, the open containers, the budgets spent) but
//! not the text: every step is handed the text to read. [`Reader`](crate::Reader)
//! hands it the whole document; [`StreamReader`](crate::StreamReader) hands
//! it whatever has arrived so far. One machine, so both readers accept
//! exactly the same documents with exactly the same errors and offsets.
//!
//! Over a complete document, running off the end is
//! [`ErrorKind::UnexpectedEnd`] at the text's length, as everywhere in the
//! crate. Over a partial one (`complete == false`) that same error means "the
//! text so far is fine, send more": the machine first records how far it got
//! inside the token it was reading ([`Pending`]), so that when more text
//! arrives the token continues where it stopped instead of being scanned
//! again from its first byte. A string or a run of whitespace fed one byte
//! at a time therefore costs linear time, not quadratic.

use alloc::{borrow::Cow, string::String, vec::Vec};

use crate::{
    DuplicateKeys, Kind, Limits, Number, dupes,
    error::{Error, ErrorKind},
    lexer::{self, NumScan, StrScan},
    reader::Event,
    scan,
    small::{BitStack, SmallStack},
};

/// What the machine expects next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    /// A value: at the start, or after a member's `:`.
    Value,
    /// A value after `,` in an array; `]` here is a trailing comma.
    Element,
    /// Just after `[`: a value or `]`.
    ArrayFirst,
    /// After an array item: `,` or `]`.
    ArrayNext,
    /// Just after `{`: a key or `}`.
    ObjectFirst,
    /// After `,` in an object: a key; `}` here is a trailing comma.
    Key,
    /// After a member's value: `,` or `}`.
    ObjectNext,
    /// The document's value is complete: only whitespace may follow.
    End,
    /// The end was reached and reported.
    Done,
}

/// A token cut off by the end of a partial text, to continue when more
/// arrives. Value tokens were counted against the element budget when they
/// started, and are not counted again.
#[derive(Clone, Copy, Debug)]
enum Pending {
    /// Nothing in progress.
    None,
    /// A string value whose closing quote has not arrived.
    Str { quote: usize, scan: StrScan },
    /// A key whose closing quote has not arrived.
    Key { quote: usize, scan: StrScan },
    /// A key read whole, waiting for its `:`; whitespace up to `from` is
    /// already skipped.
    Colon {
        quote: usize,
        end: usize,
        escaped: bool,
        from: usize,
    },
    /// A number that reached the end of the text.
    Num { start: usize, scan: NumScan },
    /// `true`, `false`, or `null`, cut short (at most four more bytes).
    Literal { start: usize },
}

/// An object still open whose keys are being kept.
#[derive(Clone, Copy, Debug)]
struct OpenObject {
    /// Its first key in `Machine::keys` (and, with `copy_keys`, in
    /// `Machine::copies`).
    keys: usize,
    /// One bit for each of its keys' fingerprints (a 64-bit Bloom filter):
    /// a new key whose bit is clear cannot match an earlier one, and is not
    /// compared with them.
    seen: u64,
    /// Two keys may be equal (their fingerprints are), or the object is too
    /// big to have compared them as they came: check when it closes.
    suspect: bool,
}

/// A key of an open object: its opening quote and the fingerprint of its
/// decoded text.
#[derive(Clone, Copy, Debug)]
struct KeyMark {
    print: u64,
    /// The opening quote's offset in the whole input, not in the current
    /// text: `rebase` drops the front of the text while the key's object is
    /// still open, and the key's offset must survive that
    /// (`Machine::dropped` converts).
    quote: usize,
}

/// Objects with more keys than this are checked by sorting when they close;
/// smaller ones compare each new key's fingerprint as it is read.
const COMPARE_AS_READ: usize = 16;

/// Open objects whose state is kept inline (deeper ones spill to the heap).
const INLINE_OBJECTS: usize = 8;

/// Keys of open objects kept inline (more spill to the heap).
const INLINE_KEYS: usize = 16;

/// The reader's position in the grammar, with its budgets. See the module
/// documentation.
///
/// Under [`DuplicateKeys::Error`] the machine also refuses a repeated key
/// when its object closes, at the repeat's opening quote: the same check, at
/// the same point, as the tree builder made in 0.2, now applied to every way
/// of reading. For each key of an open object it keeps the key's offset and
/// a one-word fingerprint, compared with the object's earlier keys as it
/// arrives; only an object where two fingerprints match (or with more than
/// [`COMPARE_AS_READ`] keys) has its keys read again and compared as text
/// when it closes. Over a text whose front is dropped as it is read
/// (`copy_keys`), each key's decoded text is copied as well.
///
/// The open containers are a stack of bits, and the first few open objects
/// and keys are kept inline ([`SmallStack`]): reading a message of ordinary
/// shape allocates nothing for the machine's own state.
#[derive(Clone, Debug)]
pub(crate) struct Machine {
    /// The offset just past the input consumed so far.
    pub(crate) pos: usize,
    /// Where the most recent event's token began.
    pub(crate) last: usize,
    state: State,
    /// One bit per open container: set for an object.
    stack: BitStack,
    elements: usize,
    pub(crate) limits: Limits,
    pub(crate) duplicates: DuplicateKeys,
    /// Set while a value is skipped: strings are not decoded.
    pub(crate) skipping: bool,
    /// Whether the end of the text is the end of the input.
    pub(crate) complete: bool,
    pending: Pending,
    /// Keep every key in the arena, not as a slice of the text (for a text
    /// whose front is dropped as it is read).
    pub(crate) copy_keys: bool,
    /// Read a sequence of documents: after one ends, another may follow.
    pub(crate) documents: bool,
    /// How typed reads take integers.
    pub(crate) numbers: crate::Numbers,
    objects: SmallStack<OpenObject, INLINE_OBJECTS>,
    /// Each key of the open objects.
    keys: SmallStack<KeyMark, INLINE_KEYS>,
    /// The bytes dropped from the front of the text so far by
    /// [`rebase`](Self::rebase): a text offset plus this is an offset in the
    /// whole input.
    dropped: usize,
    /// With `copy_keys`: each key's decoded text, as a range of `arena`
    /// (one for each entry of `keys`).
    copies: Vec<(usize, usize)>,
    arena: String,
    order: Vec<usize>,
}

impl Machine {
    /// A machine at the start of a document.
    pub(crate) const fn new(limits: Limits, duplicates: DuplicateKeys, complete: bool) -> Self {
        Self {
            pos: 0,
            last: 0,
            state: State::Value,
            stack: BitStack::new(),
            elements: 0,
            limits,
            duplicates,
            skipping: false,
            complete,
            pending: Pending::None,
            copy_keys: false,
            documents: false,
            numbers: crate::Numbers::IntegerText,
            objects: SmallStack::new(OpenObject {
                keys: 0,
                seen: 0,
                suspect: false,
            }),
            keys: SmallStack::new(KeyMark { print: 0, quote: 0 }),
            dropped: 0,
            copies: Vec::new(),
            arena: String::new(),
            order: Vec::new(),
        }
    }

    /// Whether keys are checked for duplicates.
    #[inline]
    fn checks_keys(&self) -> bool {
        self.duplicates == DuplicateKeys::Error
    }

    /// The number of arrays and objects open.
    #[inline]
    pub(crate) fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Whether the document has been read to its end.
    #[inline]
    pub(crate) fn is_done(&self) -> bool {
        self.state == State::Done
    }

    /// Marks the document as read to its end, so an iterator over the
    /// events stops (after it has yielded an error once).
    #[inline]
    pub(crate) fn finish_iteration(&mut self) {
        self.state = State::Done;
    }

    /// The first byte the machine may still need: the start of a token in
    /// progress, or the current position.
    pub(crate) fn needed_from(&self) -> usize {
        match self.pending {
            Pending::None => self.pos,
            Pending::Str { quote, .. }
            | Pending::Key { quote, .. }
            | Pending::Colon { quote, .. } => quote,
            Pending::Num { start, .. } | Pending::Literal { start } => start,
        }
    }

    /// Shifts every stored offset `by` bytes towards the start, after the
    /// first `by` bytes of the text were dropped. `by` is at most
    /// [`needed_from`](Self::needed_from). An event that began in the
    /// dropped part reads as beginning at 0; the stream reader keeps its own
    /// absolute copy of that offset.
    pub(crate) fn rebase(&mut self, by: usize) {
        // Key offsets are kept in whole-input terms: nothing to shift there.
        self.dropped += by;
        self.pos -= by;
        self.last = self.last.saturating_sub(by);
        self.pending = match self.pending {
            Pending::None => Pending::None,
            Pending::Str { quote, scan } => Pending::Str {
                quote: quote - by,
                scan: StrScan {
                    next: scan.next - by,
                    escaped: scan.escaped,
                },
            },
            Pending::Key { quote, scan } => Pending::Key {
                quote: quote - by,
                scan: StrScan {
                    next: scan.next - by,
                    escaped: scan.escaped,
                },
            },
            Pending::Colon {
                quote,
                end,
                escaped,
                from,
            } => Pending::Colon {
                quote: quote - by,
                end: end - by,
                escaped,
                from: from - by,
            },
            Pending::Num { start, mut scan } => {
                scan.rebase(by);
                Pending::Num {
                    start: start - by,
                    scan,
                }
            }
            Pending::Literal { start } => Pending::Literal { start: start - by },
        };
    }

    #[inline]
    fn end_error(src: &str) -> Error {
        Error::new(ErrorKind::UnexpectedEnd, src.len())
    }

    /// Whether `e` means only that a partial text ran out.
    #[inline]
    fn cut_short(&self, e: &Error, src: &str) -> bool {
        !self.complete && e.kind() == ErrorKind::UnexpectedEnd && e.offset() == src.len()
    }

    /// Records `pending` if `e` only means the text ran out, and passes `e` on.
    #[inline]
    fn suspend<T>(&mut self, e: Error, src: &str, pending: Pending) -> Result<T, Error> {
        if self.cut_short(&e, src) {
            self.pending = pending;
        }
        Err(e)
    }

    /// Moves past whitespace and a separating `,` and reports whether a value
    /// comes next, without consuming any token of the value. Over a partial
    /// text, an answer that depends on bytes not yet there is
    /// [`ErrorKind::UnexpectedEnd`].
    pub(crate) fn at_value(&mut self, src: &str) -> Result<bool, Error> {
        match self.pending {
            Pending::None => {}
            Pending::Key { .. } | Pending::Colon { .. } => return Ok(false),
            _ => return Ok(true),
        }
        let bytes = src.as_bytes();
        self.pos = scan::skip_whitespace(bytes, self.pos);
        let next = bytes.get(self.pos).copied();
        Ok(match self.state {
            State::Value | State::Element => true,
            State::End if self.documents => match next {
                Some(_) => {
                    self.next_document();
                    true
                }
                None if !self.complete => return Err(Self::end_error(src)),
                None => false,
            },
            State::ArrayFirst => match next {
                None if !self.complete => return Err(Self::end_error(src)),
                next => next != Some(b']'),
            },
            State::ArrayNext => match next {
                Some(b',') => {
                    // Leave the position at the value itself, for `peek_kind`.
                    self.pos = scan::skip_whitespace(bytes, self.pos + 1);
                    self.state = State::Element;
                    true
                }
                None if !self.complete => return Err(Self::end_error(src)),
                _ => false,
            },
            _ => false,
        })
    }

    /// [`at_value`](Self::at_value)'s answer for the common cases inside an
    /// array, small enough to inline: after an item, a `,` and the next
    /// item; after the `[`, an item. `None` for anything else, which
    /// `at_value` then decides.
    #[inline]
    pub(crate) fn at_item(&mut self, src: &str) -> Option<bool> {
        if !matches!(self.pending, Pending::None) {
            return None;
        }
        let bytes = src.as_bytes();
        let at = scan::skip_whitespace(bytes, self.pos);
        match (self.state, bytes.get(at)) {
            (State::ArrayNext, Some(b',')) => {
                let next = scan::skip_whitespace(bytes, at + 1);
                match bytes.get(next) {
                    Some(&b) if b != b']' => {
                        self.pos = next;
                        self.state = State::Element;
                        Some(true)
                    }
                    _ => None,
                }
            }
            (State::ArrayFirst, Some(&b)) if b != b']' => {
                self.pos = at;
                Some(true)
            }
            _ => None,
        }
    }

    /// The kind of the value that starts at the current position, if a
    /// value starts there. Call after [`at_value`](Self::at_value) said one
    /// comes next.
    pub(crate) fn peek_kind(&self, src: &str) -> Option<Kind> {
        let at = match self.pending {
            Pending::Str { .. } => return Some(Kind::String),
            Pending::Num { .. } => return Some(Kind::Number),
            Pending::Literal { start } => start,
            Pending::Key { .. } | Pending::Colon { .. } => return None,
            Pending::None => self.pos,
        };
        Some(match src.as_bytes().get(at)? {
            b'{' => Kind::Object,
            b'[' => Kind::Array,
            b'"' => Kind::String,
            b'-' | b'0'..=b'9' => Kind::Number,
            b't' | b'f' => Kind::Bool,
            b'n' => Kind::Null,
            _ => return None,
        })
    }

    /// Reads the next event from `src`, or `Ok(None)` at the end of the
    /// document.
    pub(crate) fn step<'s>(&mut self, src: &'s str) -> Result<Option<Event<'s>>, Error> {
        if !matches!(self.pending, Pending::None) {
            return self.resume(src).map(Some);
        }
        let bytes = src.as_bytes();
        loop {
            self.pos = scan::skip_whitespace(bytes, self.pos);
            let at = self.pos;
            let next = bytes.get(at).copied();
            let err = |kind| Err(Error::new(kind, at));
            match self.state {
                State::Value | State::Element | State::ArrayFirst => match next {
                    None => return Err(Self::end_error(src)),
                    Some(b']') if self.state == State::ArrayFirst => {
                        return Ok(Some(self.close(at, Event::EndArray)));
                    }
                    Some(b']') if self.state == State::Element => {
                        return err(ErrorKind::TrailingComma);
                    }
                    Some(b) => return self.value(src, b, at).map(Some),
                },
                State::ArrayNext => match next {
                    None => return Err(Self::end_error(src)),
                    Some(b',') => {
                        self.pos += 1;
                        self.state = State::Element;
                    }
                    Some(b']') => return Ok(Some(self.close(at, Event::EndArray))),
                    Some(_) => return err(ErrorKind::ExpectedCommaOrBracket),
                },
                State::ObjectFirst | State::Key => match next {
                    None => return Err(Self::end_error(src)),
                    Some(b'"') => return self.key(src, at, StrScan::start(at)).map(Some),
                    Some(b'}') if self.state == State::ObjectFirst => {
                        return self.close_object(src, at).map(Some);
                    }
                    Some(b'}') => return err(ErrorKind::TrailingComma),
                    Some(_) => return err(ErrorKind::ExpectedKey),
                },
                State::ObjectNext => match next {
                    None => return Err(Self::end_error(src)),
                    Some(b',') => {
                        self.pos += 1;
                        self.state = State::Key;
                    }
                    Some(b'}') => return self.close_object(src, at).map(Some),
                    Some(_) => return err(ErrorKind::ExpectedCommaOrBrace),
                },
                State::End => {
                    return match next {
                        // Over a partial text, whitespace so far proves
                        // nothing: more input may still follow.
                        None if !self.complete => Err(Self::end_error(src)),
                        None => {
                            self.state = State::Done;
                            Ok(None)
                        }
                        Some(_) if self.documents => {
                            self.next_document();
                            continue;
                        }
                        Some(_) => err(ErrorKind::TrailingCharacters),
                    };
                }
                State::Done => return Ok(None),
            }
        }
    }

    /// Enters the object (`object`) or array that starts at the current
    /// position, as [`step`](Self::step) would: the fast path of a typed
    /// read, which knows what it wants. `Ok(false)`, with nothing consumed,
    /// if no such container starts here (or a token is in progress); the
    /// caller then takes the general path.
    #[inline]
    pub(crate) fn enter(&mut self, src: &str, object: bool) -> Result<bool, Error> {
        if !matches!(self.pending, Pending::None)
            || !matches!(
                self.state,
                State::Value | State::Element | State::ArrayFirst
            )
        {
            return Ok(false);
        }
        let bytes = src.as_bytes();
        let at = scan::skip_whitespace(bytes, self.pos);
        let want = if object { b'{' } else { b'[' };
        if bytes.get(at) != Some(&want) {
            return Ok(false);
        }
        // The step opens it (one copy of that code, inlined in the step).
        self.step(src).map(|_| true)
    }

    /// Continues the token recorded in `pending`.
    fn resume<'s>(&mut self, src: &'s str) -> Result<Event<'s>, Error> {
        match core::mem::replace(&mut self.pending, Pending::None) {
            Pending::None => Err(Self::end_error(src)),
            Pending::Str { quote, scan } => self.string(src, quote, scan),
            Pending::Key { quote, scan } => self.key(src, quote, scan),
            Pending::Colon {
                quote,
                end,
                escaped,
                from,
            } => self.colon(src, quote, end, escaped, from),
            Pending::Num { start, scan } => self.number(src, start, scan),
            Pending::Literal { start } => self.literal(src, start),
        }
    }

    /// Starts the next document of a sequence: its own element budget.
    fn next_document(&mut self) {
        self.state = State::Value;
        self.elements = 0;
    }

    /// The state after a value completes in the current container.
    #[inline]
    fn after_value(&mut self) {
        self.state = match self.stack.last() {
            None => State::End,
            Some(false) => State::ArrayNext,
            Some(true) => State::ObjectNext,
        };
    }

    /// Closes the innermost container, whose closing byte is at `at`.
    #[inline]
    fn close<'s>(&mut self, at: usize, event: Event<'s>) -> Event<'s> {
        self.pos = at + 1;
        self.last = at;
        self.stack.pop();
        self.after_value();
        event
    }

    /// Closes the innermost object, whose `}` is at `at`, refusing it if it
    /// repeats a key.
    fn close_object<'s>(&mut self, src: &'s str, at: usize) -> Result<Event<'s>, Error> {
        if self.checks_keys() {
            if let Some(open) = self.objects.pop() {
                if open.suspect {
                    self.check_object(src, open)?;
                }
                self.keys.truncate(open.keys);
                if self.copy_keys {
                    if let Some(&(from, _)) = self.copies.get(open.keys) {
                        self.arena.truncate(from);
                    }
                    self.copies.truncate(open.keys);
                }
            }
        }
        Ok(self.close(at, Event::EndObject))
    }

    /// Compares the keys of the closing object `open` as text, refusing a
    /// repeat.
    #[cold]
    fn check_object(&mut self, src: &str, open: OpenObject) -> Result<(), Error> {
        // Back to text offsets. A key whose quote was dropped from the text
        // (only with `copy_keys`, whose texts come from the arena) wraps
        // below zero here; the stream reader's error conversion adds the
        // dropped length back with the same wrapping, so the reported
        // offset is exact either way.
        let quotes: Vec<usize> = self
            .keys
            .iter_from(open.keys)
            .map(|k| k.quote.wrapping_sub(self.dropped))
            .collect();
        let repeat = if self.copy_keys {
            let copies = self.copies.get(open.keys..).unwrap_or(&[]);
            let arena = self.arena.as_str();
            let text = |i: usize| {
                copies
                    .get(i)
                    .and_then(|&(from, to)| arena.get(from..to))
                    .unwrap_or("")
            };
            dupes::has_duplicates(quotes.len(), &text, &mut self.order)
                .then(|| dupes::first_repeat(&text, &self.order))
        } else {
            // Read each key again from the text: the keys only, not the
            // values between them.
            let texts: Vec<Cow<'_, str>> = quotes.iter().map(|&q| key_text(src, q)).collect();
            let text = |i: usize| texts.get(i).map_or("", |t| t);
            dupes::has_duplicates(texts.len(), &text, &mut self.order)
                .then(|| dupes::first_repeat(&text, &self.order))
        };
        match repeat.and_then(|i| quotes.get(i)) {
            Some(&quote) => Err(Error::new(ErrorKind::DuplicateKey, quote)),
            None => Ok(()),
        }
    }

    /// Notes the key at `quote..end` of the object being read, and whether
    /// it may repeat an earlier one.
    #[inline]
    fn keep_key(&mut self, src: &str, quote: usize, end: usize, escaped: bool) {
        let print = if self.copy_keys || escaped {
            self.decoded_print(src, quote, end, escaped)
        } else {
            dupes::fingerprint_at(src.as_bytes(), quote + 1, end - 1)
        };
        let count = self.keys.len();
        if let Some(open) = self.objects.last_mut() {
            let bit = 1u64 << (print.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 58);
            if !open.suspect && open.seen & bit != 0 {
                let (inline, spill) = self.keys.from(open.keys);
                open.suspect = inline.iter().chain(spill).any(|k| k.print == print);
            }
            open.seen |= bit;
            if count - open.keys >= COMPARE_AS_READ {
                open.suspect = true;
            }
        }
        self.keys.push(KeyMark {
            print,
            quote: quote + self.dropped,
        });
    }

    /// The fingerprint of the key at `quote..end` when its decoded text is
    /// needed: it has escapes, or (with `copy_keys`) is copied to the arena.
    /// Out of line, to keep [`keep_key`](Self::keep_key) small.
    #[cold]
    #[inline(never)]
    fn decoded_print(&mut self, src: &str, quote: usize, end: usize, escaped: bool) -> u64 {
        let body = src.get(quote + 1..end - 1).unwrap_or("");
        if self.copy_keys {
            let from = self.arena.len();
            if escaped {
                self.arena.push_str(&lexer::decode(body));
            } else {
                self.arena.push_str(body);
            }
            let to = self.arena.len();
            self.copies.push((from, to));
            dupes::fingerprint(self.arena.as_bytes().get(from..to).unwrap_or_default())
        } else {
            dupes::fingerprint(lexer::decode(body).as_bytes())
        }
    }

    /// Counts one more value, refusing it if the budget is spent.
    #[inline]
    fn count_value(&mut self, at: usize) -> Result<(), Error> {
        if self.elements >= self.limits.max_elements {
            return Err(Error::new(ErrorKind::TooManyElements, at));
        }
        self.elements += 1;
        Ok(())
    }

    /// Reads the value starting with byte `b` at `at`.
    #[inline]
    fn value<'s>(&mut self, src: &'s str, b: u8, at: usize) -> Result<Event<'s>, Error> {
        match b {
            b'{' => self.open(at, true),
            b'[' => self.open(at, false),
            b'"' => {
                self.count_value(at)?;
                self.string(src, at, StrScan::start(at))
            }
            b'-' | b'0'..=b'9' => {
                self.count_value(at)?;
                match lexer::quick_integer(src.as_bytes(), at) {
                    Some(end) if end - at <= self.limits.max_number_bytes => {
                        self.pos = end;
                        self.last = at;
                        self.after_value();
                        let text = src.get(at..end).unwrap_or("0");
                        Ok(Event::Number(Number::from_valid(text)))
                    }
                    _ => self.number(src, at, NumScan::start(at)),
                }
            }
            b't' | b'f' | b'n' => {
                self.count_value(at)?;
                self.literal(src, at)
            }
            _ => Err(Error::new(ErrorKind::ExpectedValue, at)),
        }
    }

    /// Opens the object (`object`) or array whose first byte is at `at`.
    #[inline]
    fn open<'s>(&mut self, at: usize, object: bool) -> Result<Event<'s>, Error> {
        self.count_value(at)?;
        if self.stack.len() >= self.limits.max_depth {
            return Err(Error::new(ErrorKind::DepthLimitExceeded, at));
        }
        self.pos = at + 1;
        self.last = at;
        self.stack.push(object);
        Ok(if object {
            self.state = State::ObjectFirst;
            if self.checks_keys() {
                self.objects.push(OpenObject {
                    keys: self.keys.len(),
                    seen: 0,
                    suspect: false,
                });
            }
            Event::StartObject
        } else {
            self.state = State::ArrayFirst;
            Event::StartArray
        })
    }

    /// Reads (or continues) the string value whose quote is at `quote`.
    #[inline(always)]
    fn string<'s>(
        &mut self,
        src: &'s str,
        quote: usize,
        scan: StrScan,
    ) -> Result<Event<'s>, Error> {
        let mut scan = scan;
        match lexer::resume_string(
            src.as_bytes(),
            quote,
            &mut scan,
            self.limits.max_string_bytes,
        ) {
            Ok(token) => {
                self.pos = token.end;
                self.last = quote;
                let text = self.body(src, quote, token.end, token.escaped);
                self.after_value();
                Ok(Event::String(text))
            }
            Err(e) => self.suspend(e, src, Pending::Str { quote, scan }),
        }
    }

    /// Reads (or continues) the key whose quote is at `quote`, and the `:`
    /// after it.
    fn key<'s>(&mut self, src: &'s str, quote: usize, scan: StrScan) -> Result<Event<'s>, Error> {
        let mut scan = scan;
        match lexer::resume_string(
            src.as_bytes(),
            quote,
            &mut scan,
            self.limits.max_string_bytes,
        ) {
            Ok(token) => self.colon(src, quote, token.end, token.escaped, token.end),
            Err(e) => self.suspend(e, src, Pending::Key { quote, scan }),
        }
    }

    /// Finds the `:` after the key at `quote..end`, looking from `from`.
    fn colon<'s>(
        &mut self,
        src: &'s str,
        quote: usize,
        end: usize,
        escaped: bool,
        from: usize,
    ) -> Result<Event<'s>, Error> {
        let at = scan::skip_whitespace(src.as_bytes(), from);
        match src.as_bytes().get(at) {
            Some(b':') => {
                self.pos = at + 1;
                self.last = quote;
                self.state = State::Value;
                if self.checks_keys() {
                    self.keep_key(src, quote, end, escaped);
                }
                Ok(Event::Key(self.body(src, quote, end, escaped)))
            }
            Some(_) => Err(Error::new(ErrorKind::ExpectedColon, at)),
            None => {
                let pending = Pending::Colon {
                    quote,
                    end,
                    escaped,
                    from: at,
                };
                self.suspend(Self::end_error(src), src, pending)
            }
        }
    }

    /// Reads (or continues) the number starting at `start`.
    #[inline]
    fn number<'s>(
        &mut self,
        src: &'s str,
        start: usize,
        scan: NumScan,
    ) -> Result<Event<'s>, Error> {
        let mut scan = scan;
        let max = self.limits.max_number_bytes;
        match lexer::resume_number(src.as_bytes(), start, &mut scan, self.complete, max) {
            Ok(end) => {
                self.pos = end;
                self.last = start;
                self.after_value();
                let text = src.get(start..end).unwrap_or("0");
                Ok(Event::Number(Number::from_valid(text)))
            }
            Err(e) => self.suspend(e, src, Pending::Num { start, scan }),
        }
    }

    /// Reads the literal starting at `start` (its first byte is `t`, `f`, or
    /// `n`).
    #[inline]
    fn literal<'s>(&mut self, src: &'s str, start: usize) -> Result<Event<'s>, Error> {
        let (word, event): (&[u8], _) = match src.as_bytes().get(start) {
            Some(b't') => (b"true", Event::Bool(true)),
            Some(b'f') => (b"false", Event::Bool(false)),
            _ => (b"null", Event::Null),
        };
        match lexer::scan_literal(src.as_bytes(), start, word) {
            Ok(end) => {
                self.pos = end;
                self.last = start;
                self.after_value();
                Ok(event)
            }
            Err(e) => self.suspend(e, src, Pending::Literal { start }),
        }
    }

    /// The contents of the string at `quote..end`: borrowed when it has no
    /// escapes (or while skipping), decoded otherwise.
    #[inline]
    fn body<'s>(&self, src: &'s str, quote: usize, end: usize, escaped: bool) -> Cow<'s, str> {
        let body = src.get(quote + 1..end - 1).unwrap_or("");
        if escaped && !self.skipping {
            Cow::Owned(lexer::decode(body))
        } else {
            Cow::Borrowed(body)
        }
    }
}

/// The decoded text of the (valid) key whose opening quote is at `quote`.
pub(crate) fn key_text(src: &str, quote: usize) -> Cow<'_, str> {
    match lexer::scan_string(src.as_bytes(), quote, usize::MAX) {
        Ok(token) => {
            let body = src.get(quote + 1..token.end - 1).unwrap_or("");
            if token.escaped {
                Cow::Owned(lexer::decode(body))
            } else {
                Cow::Borrowed(body)
            }
        }
        Err(_) => Cow::Borrowed(""),
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    /// Feeds `text` to a partial machine `chunk` bytes more at a time and
    /// collects the events, as the stream reader does (without compaction).
    fn chunked(text: &str, chunk: usize) -> Result<Vec<Event<'static>>, Error> {
        let mut m = Machine::new(Limits::unlimited(), DuplicateKeys::KeepAll, false);
        let mut out = Vec::new();
        let mut have = 0;
        loop {
            let src = text.get(..have).unwrap_or(text);
            match m.step(src) {
                Ok(Some(e)) => out.push(owned(e)),
                Ok(None) => return Ok(out),
                Err(e) if m.cut_short(&e, src) => {
                    if have == text.len() {
                        m.complete = true;
                    }
                    have = (have + chunk).min(text.len());
                    while !text.is_char_boundary(have) {
                        have += 1;
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn owned(e: Event<'_>) -> Event<'static> {
        match e {
            Event::Key(k) => Event::Key(Cow::Owned(k.into_owned())),
            Event::String(s) => Event::String(Cow::Owned(s.into_owned())),
            Event::Number(n) => Event::Number(n.into_owned()),
            Event::StartObject => Event::StartObject,
            Event::EndObject => Event::EndObject,
            Event::StartArray => Event::StartArray,
            Event::EndArray => Event::EndArray,
            Event::Bool(b) => Event::Bool(b),
            Event::Null => Event::Null,
        }
    }

    fn whole(text: &str) -> Result<Vec<Event<'static>>, Error> {
        let mut m = Machine::new(Limits::unlimited(), DuplicateKeys::KeepAll, true);
        let mut out = Vec::new();
        while let Some(e) = m.step(text)? {
            out.push(owned(e));
        }
        Ok(out)
    }

    #[test]
    fn test_partial_text_gives_the_same_events_at_every_chunk_size() {
        let docs = [
            r#"{"a" : [1, -2.5e3, true, false, null, "x\ny\u00e9\ud83d\ude00"], "b": {}}"#,
            "  12345  ",
            "[[], {}, [[]], \"\"]",
            r#"{"key with spaces"   :   "v"}"#,
            "[1,]",
            "{\"a\" 1}",
            "[tru]",
            "[1 2]",
            "-",
            "\"abc",
            "[0e]",
            "nul",
        ];
        for doc in docs {
            let want = whole(doc);
            for chunk in 1..=doc.len() {
                assert_eq!(chunked(doc, chunk), want, "{doc:?} in chunks of {chunk}");
            }
        }
    }

    /// Every event of `text` under `DuplicateKeys::Error`, or the error.
    fn checked(text: &str) -> Result<usize, Error> {
        let mut m = Machine::new(Limits::new(), DuplicateKeys::Error, true);
        let mut n = 0;
        while m.step(text)?.is_some() {
            n += 1;
        }
        Ok(n)
    }

    #[test]
    fn test_repeats_are_found_past_the_inline_state_and_collisions_are_not_repeats() {
        // Nested past the inline objects and keys: each level has three keys
        // before its child, so the deepest keys live in the spill.
        let nest = |depth: usize, repeat_at: Option<usize>| {
            let mut text = alloc::string::String::new();
            for level in 0..depth {
                text.push_str(r#"{"a": 1, "b": 2, "c": 3, "#);
                if repeat_at == Some(level) {
                    text.push_str(r#""b": 4, "#);
                }
                text.push_str(r#""child": "#);
            }
            text.push_str("null");
            text.push_str(&"}".repeat(depth));
            text
        };
        assert!(checked(&nest(20, None)).is_ok());
        for level in [0, 5, 7, 8, 12, 19] {
            let text = nest(20, Some(level));
            let quote = text.match_indices(r#""b": 4"#).next().map(|(at, _)| at);
            let err = checked(&text).err();
            assert_eq!(
                err.map(|e| (e.kind(), Some(e.offset()))),
                Some((ErrorKind::DuplicateKey, quote)),
                "{level}"
            );
        }
        // Keys that share their length and first and last eight bytes have
        // equal fingerprints: compared as text, they differ.
        let same_print = r#"{"abcdefgh_1_abcdefgh": 1, "abcdefgh_2_abcdefgh": 2}"#;
        assert_eq!(
            dupes::fingerprint(b"abcdefgh_1_abcdefgh"),
            dupes::fingerprint(b"abcdefgh_2_abcdefgh")
        );
        assert!(checked(same_print).is_ok());
        assert!(
            checked(
                r#"{"abcdefgh_1_abcdefgh": 1, "abcdefgh_2_abcdefgh": 2, "abcdefgh_1_abcdefgh": 3}"#
            )
            .is_err()
        );
        // An escaped key equals its unescaped spelling, short or long.
        assert!(checked(r#"{"id": 1, "\u0069d": 2}"#).is_err());
        assert!(checked(r#"{"textDocument": 1, "textDocumen\u0074": 2}"#).is_err());
        // Many keys: checked by sorting when the object closes.
        let mut wide = alloc::string::String::new();
        for i in 0..40 {
            wide.push_str(&alloc::format!(r#""k{i}": {i}, "#));
        }
        assert!(checked(&alloc::format!("{{{wide}\"end\": 0}}")).is_ok());
        assert!(checked(&alloc::format!("{{{wide}\"k3\": 0}}")).is_err());
    }

    #[test]
    fn test_documents_follow_one_another() {
        let text = " 1 [2]{\"a\":3}\"s\"
null ";
        let mut m = Machine::new(Limits::new(), DuplicateKeys::Error, true);
        m.documents = true;
        let mut events = Vec::new();
        while let Ok(Some(e)) = m.step(text) {
            events.push(owned(e));
        }
        assert_eq!(events.len(), 10);
        assert!(m.is_done());
        // At the end of one document, another value comes next.
        let mut m = Machine::new(Limits::new(), DuplicateKeys::Error, true);
        m.documents = true;
        assert!(m.step("1 2").is_ok());
        assert_eq!(m.at_value("1 2"), Ok(true));
        assert_eq!(m.peek_kind("1 2"), Some(Kind::Number));
        // Over a partial text, the end of a document is not the end.
        let mut m = Machine::new(Limits::new(), DuplicateKeys::Error, false);
        m.documents = true;
        assert!(m.step("[] ").is_ok() && m.step("[] ").is_ok());
        assert!(m.at_value("[] ").is_err());
        m.complete = true;
        assert_eq!(m.at_value("[] "), Ok(false));
        assert_eq!(m.step("[] "), Ok(None));
    }

    #[test]
    fn test_peek_kind_and_at_value() {
        let src = r#"[{"a": 1}, "s", 2, true, null, []]"#;
        let mut m = Machine::new(Limits::new(), DuplicateKeys::Error, true);
        assert_eq!(m.at_value(src), Ok(true));
        assert_eq!(m.peek_kind(src), Some(Kind::Array));
        assert!(m.step(src).is_ok());
        let mut kinds = Vec::new();
        while m.at_value(src) == Ok(true) {
            kinds.push(m.peek_kind(src));
            // Skip the value: a container takes two events here.
            let depth = m.depth();
            let _first = m.step(src);
            while m.depth() > depth {
                let _inner = m.step(src);
            }
        }
        assert_eq!(
            kinds,
            [
                Some(Kind::Object),
                Some(Kind::String),
                Some(Kind::Number),
                Some(Kind::Bool),
                Some(Kind::Null),
                Some(Kind::Array)
            ]
        );
        assert_eq!(m.peek_kind("x"), None);
    }

    #[test]
    fn test_pending_tokens_report_their_kind_and_rebase() {
        let mut m = Machine::new(Limits::new(), DuplicateKeys::Error, false);
        let src = "   [\"abc";
        assert!(m.step(src).is_ok()); // [
        assert!(m.step(src).is_err()); // the string is cut short
        assert_eq!(m.needed_from(), 4);
        assert_eq!(m.peek_kind(src), Some(Kind::String));
        assert_eq!(m.at_value(src), Ok(true));
        m.rebase(3);
        assert_eq!(m.needed_from(), 1);
        let rest = "[\"abcdef\"]";
        assert_eq!(
            m.step(rest).ok().flatten(),
            Some(Event::String("abcdef".into()))
        );
        assert_eq!(m.last, 1);

        let mut m = Machine::new(Limits::new(), DuplicateKeys::Error, false);
        assert!(m.step("{\"k\"  ").is_ok()); // {
        assert!(m.step("{\"k\"  ").is_err()); // waiting for the colon
        assert_eq!(m.at_value("{\"k\"  "), Ok(false));
        assert_eq!(m.peek_kind("{\"k\"  "), None);
        m.rebase(1);
        assert_eq!(
            m.step("\"k\"  :1}").ok().flatten(),
            Some(Event::Key("k".into()))
        );

        let mut m = Machine::new(Limits::new(), DuplicateKeys::Error, false);
        assert!(m.step("[12").is_ok());
        assert!(m.step("[12").is_err());
        assert_eq!(m.peek_kind("[12"), Some(Kind::Number));
        m.rebase(1);
        assert_eq!(m.needed_from(), 0);
        assert!(
            m.step("12]")
                .is_ok_and(|e| e == Some(Event::Number(12.into())))
        );

        let mut m = Machine::new(Limits::new(), DuplicateKeys::Error, false);
        assert!(m.step("[tr").is_ok());
        assert!(m.step("[tr").is_err());
        assert_eq!(m.peek_kind("[tr"), Some(Kind::Bool));
        m.rebase(1);
        assert_eq!(m.step("true]").ok().flatten(), Some(Event::Bool(true)));
    }
}
