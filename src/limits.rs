//! Budgets on what a document may cost to read.

/// Budgets on the work and memory a document may cost.
///
/// Every budget is checked before the memory it guards is allocated (with
/// the one exception noted under "What a tree costs"), so a hostile document
/// is refused while it is still cheap to refuse. None of them
/// is there to protect the stack: the parser, the writer, and every operation
/// on [`Value`](crate::Value) use a bounded amount of stack whatever the
/// nesting depth (explicit stacks on the heap; drop recurses at most a fixed
/// 32 levels). The depth budget exists for the code that consumes the value,
/// which may well be recursive.
///
/// | Budget | Default | Counts |
/// |---|---:|---|
/// | [`max_depth`](Self::max_depth) | 128 | nested arrays and objects; a scalar at the top is depth 0, `[]` is 1 |
/// | [`max_input_bytes`](Self::max_input_bytes) | 64 MiB | bytes of input |
/// | [`max_string_bytes`](Self::max_string_bytes) | 16 MiB | source bytes between a string's quotes, keys included |
/// | [`max_number_bytes`](Self::max_number_bytes) | 1 KiB | bytes of a number's text |
/// | [`max_elements`](Self::max_elements) | 1 Mi | values in the document: every scalar, array, and object counts one |
/// | [`max_memory`](Self::max_memory) | 64 MiB | heap a read of one document holds for what it builds: a tree, or typed values (see below) |
///
/// The depth default matches `serde_json`'s recursion limit, so code moving
/// from it sees the same behaviour. A string's budget is measured in source
/// bytes, which is never less than its decoded length, so the check can run
/// before the string is decoded.
///
/// **What a tree costs.** A parsed value holds, besides the borrowed input,
/// `size_of::<Value>()` bytes for every value (32 on a 64-bit target), the
/// same again for every object member's key (24 bytes for the key, so 56
/// per member), and the decoded text of every string or key that had
/// escapes. A compact document spends a few bytes of input per value, so the
/// tree can be several times the input: under the defaults a hostile
/// document can make one parse hold at most about [`max_memory`](Self::max_memory)
/// (64 MiB), whatever its shape. That is the number to multiply by the
/// parses that may run at once; a server taking many requests should lower
/// it (and `max_input_bytes`) to what its messages need.
///
/// `max_memory` counts the buffers a parse asks for (the tree and the
/// builder's working stacks, at their capacity), checked before a stack
/// grows and before each array or object is allocated; a decoded string is
/// counted once made, so a parse may hold one string budget beyond it. The
/// allocator's own overhead is not counted. It applies wherever a tree is
/// built ([`Parser::parse`](crate::Parser::parse), [`Reader::read_value`](crate::Reader::read_value),
/// [`StreamReader::read_value`](crate::StreamReader::read_value)), and to
/// typed reads ([`FromJson`](crate::FromJson) through `decode`, `from_str`,
/// and the stream reader's `decode`): one count per document, shared by
/// every tree and value the read builds. The crate's own impls charge what
/// they hold: owned strings at their capacity, vectors at their buffer's
/// size as it doubles, boxes at their contents' size, and maps and sets at
/// an estimate of about twice their entries' size (a B-tree's or hash
/// table's spare room is not measured exactly); your own `FromJson` impls
/// are charged for the crate types they read.
///
/// Separately, the reader's own state is bounded by the same number: under
/// [`DuplicateKeys::Error`](crate::DuplicateKeys::Error) the duplicate check
/// keeps about 56 bytes per key of the objects still open (and a stream
/// reader a copy of each key), and a key past `max_memory` of that is
/// refused. Reading events with no check keeps only a bit per open
/// container. So one read of a document holds at most about `max_memory`
/// for what it builds, plus as much again for the duplicate check's state
/// in the worst case.
///
/// The struct is `#[non_exhaustive]`: start from [`Limits::default`] or
/// [`Limits::unlimited`] and change the fields you care about, or chain the
/// `with_*` methods.
///
/// # Examples
///
/// ```
/// use jsonx_lib::{Limits, Parser, ErrorKind};
///
/// let strict = Parser::new().limits(Limits::default().with_max_depth(2));
/// assert!(strict.parse("[[1]]").is_ok());
/// let err = strict.parse("[[[1]]]").unwrap_err();
/// assert_eq!(err.kind(), ErrorKind::DepthLimitExceeded);
/// assert_eq!(err.offset(), 2);
///
/// let mut limits = Limits::default();
/// limits.max_string_bytes = 4;
/// assert!(Parser::new().limits(limits).parse(r#""abcde""#).is_err());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Limits {
    /// The deepest nesting of arrays and objects allowed. `0` allows only a
    /// scalar document.
    pub max_depth: usize,
    /// The longest input accepted, in bytes.
    pub max_input_bytes: usize,
    /// The longest string or key accepted, in source bytes between the quotes
    /// (escape sequences count at their written length).
    pub max_string_bytes: usize,
    /// The longest number accepted, in bytes of its text.
    pub max_number_bytes: usize,
    /// The most values a document may hold, counting every scalar, array, and
    /// object (keys are not counted separately).
    pub max_elements: usize,
    /// The most heap, in bytes, a read of one document may hold for the tree
    /// or the typed values it builds; see "What a tree costs" above.
    pub max_memory: usize,
}

impl Limits {
    /// The default budgets (see the table on [`Limits`]): generous for real
    /// documents, and bounding what a hostile one can make a read hold (about
    /// `max_memory`, see above) whatever its size or shape.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Limits;
    ///
    /// assert_eq!(Limits::new().max_depth, 128);
    /// assert_eq!(Limits::new(), Limits::default());
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_depth: 128,
            max_input_bytes: 64 * 1024 * 1024,
            max_string_bytes: 16 * 1024 * 1024,
            max_number_bytes: 1024,
            max_elements: 1024 * 1024,
            max_memory: 64 * 1024 * 1024,
        }
    }

    /// No budgets at all: every limit is `usize::MAX`. For input you trust;
    /// even then the parser cannot overflow the stack, because nothing in this
    /// crate recurses on nesting depth.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{Limits, Parser};
    ///
    /// let deep = format!("{}{}", "[".repeat(1000), "]".repeat(1000));
    /// assert!(Parser::new().limits(Limits::unlimited()).parse(&deep).is_ok());
    /// ```
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            max_depth: usize::MAX,
            max_input_bytes: usize::MAX,
            max_string_bytes: usize::MAX,
            max_number_bytes: usize::MAX,
            max_elements: usize::MAX,
            max_memory: usize::MAX,
        }
    }

    /// Sets [`max_depth`](Self::max_depth).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::Limits;
    ///
    /// assert_eq!(Limits::new().with_max_depth(8).max_depth, 8);
    /// ```
    #[must_use]
    pub const fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    /// Sets [`max_input_bytes`](Self::max_input_bytes).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Limits, Parser};
    ///
    /// let p = Parser::new().limits(Limits::new().with_max_input_bytes(4));
    /// assert!(p.parse("1234").is_ok());
    /// assert_eq!(p.parse("12345").unwrap_err().kind(), ErrorKind::InputTooLarge);
    /// ```
    #[must_use]
    pub const fn with_max_input_bytes(mut self, bytes: usize) -> Self {
        self.max_input_bytes = bytes;
        self
    }

    /// Sets [`max_string_bytes`](Self::max_string_bytes).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Limits, Parser};
    ///
    /// let p = Parser::new().limits(Limits::new().with_max_string_bytes(2));
    /// assert!(p.parse(r#"{"ab": "cd"}"#).is_ok());
    /// assert_eq!(p.parse(r#"{"abc": 1}"#).unwrap_err().kind(), ErrorKind::StringTooLong);
    /// ```
    #[must_use]
    pub const fn with_max_string_bytes(mut self, bytes: usize) -> Self {
        self.max_string_bytes = bytes;
        self
    }

    /// Sets [`max_number_bytes`](Self::max_number_bytes).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Limits, Parser};
    ///
    /// let p = Parser::new().limits(Limits::new().with_max_number_bytes(3));
    /// assert!(p.parse("-12").is_ok());
    /// assert_eq!(p.parse("1e10").unwrap_err().kind(), ErrorKind::NumberTooLong);
    /// ```
    #[must_use]
    pub const fn with_max_number_bytes(mut self, bytes: usize) -> Self {
        self.max_number_bytes = bytes;
        self
    }

    /// Sets [`max_elements`](Self::max_elements).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Limits, Parser};
    ///
    /// // The array itself is one value, so `[1, 2]` holds three.
    /// let p = Parser::new().limits(Limits::new().with_max_elements(3));
    /// assert!(p.parse("[1, 2]").is_ok());
    /// assert_eq!(p.parse("[1, 2, 3]").unwrap_err().kind(), ErrorKind::TooManyElements);
    /// ```
    #[must_use]
    pub const fn with_max_elements(mut self, count: usize) -> Self {
        self.max_elements = count;
        self
    }

    /// Sets [`max_memory`](Self::max_memory).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonx_lib::{ErrorKind, Limits, Parser};
    ///
    /// let p = Parser::new().limits(Limits::new().with_max_memory(4096));
    /// assert!(p.parse("[1, 2, 3]").is_ok());
    /// let big = format!("[{}0]", "0,".repeat(1000));
    /// assert_eq!(p.parse(&big).unwrap_err().kind(), ErrorKind::MemoryLimitExceeded);
    /// ```
    #[must_use]
    pub const fn with_max_memory(mut self, bytes: usize) -> Self {
        self.max_memory = bytes;
        self
    }
}

impl Default for Limits {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limits_default_matches_documented_table() {
        let l = Limits::default();
        assert_eq!(l.max_depth, 128);
        assert_eq!(l.max_input_bytes, 64 << 20);
        assert_eq!(l.max_string_bytes, 16 << 20);
        assert_eq!(l.max_number_bytes, 1024);
        assert_eq!(l.max_elements, 1 << 20);
        assert_eq!(l.max_memory, 64 << 20);
    }

    #[test]
    fn test_limits_setters_chain() {
        let l = Limits::unlimited()
            .with_max_depth(1)
            .with_max_input_bytes(2)
            .with_max_string_bytes(3)
            .with_max_number_bytes(4)
            .with_max_elements(5)
            .with_max_memory(6);
        assert_eq!(
            (
                l.max_depth,
                l.max_input_bytes,
                l.max_string_bytes,
                l.max_number_bytes,
                l.max_elements,
                l.max_memory
            ),
            (1, 2, 3, 4, 5, 6)
        );
    }
}
