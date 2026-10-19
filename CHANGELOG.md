<h1 align="center">
    <img width="90px" height="auto" src="https://raw.githubusercontent.com/jamesgober/jamesgober/main/media/icons/hexagon-3.svg" alt="Triple Hexagon">
    <br><b>CHANGELOG</b>
</h1>
<p>
  All notable changes to <code>jsonx-lib</code> will be documented in this file. The format is based on <a href="https://keepachangelog.com/en/1.1.0/">Keep a Changelog</a>,
  and this project adheres to <a href="https://semver.org/spec/v2.0.0.html/">Semantic Versioning</a>.
</p>

---

## [Unreleased]

---

## [0.5.0] - 2026-10-19

The implementation release: typed reading and writing through hand-written
`FromJson` and `ToJson` impls, an incremental reader for input that arrives
in chunks, a checked push writer, output formats, and the fixes of the
baseline audit: duplicate keys refused on every reading path, a memory
budget, `json!` for large literals, fuzzing.

### Migration from 0.2

- **Write functions are generic.** `to_string`, `to_string_pretty`,
  `write_value`, `write_value_pretty`, `to_writer`, and `to_writer_pretty`
  take `&T` where `T: ToJson + ?Sized` instead of `&Value<'_>`. Calls compile
  unchanged (`Value` implements `ToJson`); only a use as a function pointer
  (`to_string as fn(&Value) -> String`) needs a closure.
- **Duplicate keys are refused by the `Reader` too.** Under the default
  `DuplicateKeys::Error`, a `Reader`'s events and `read_raw` refuse an object
  that repeats a key (`ErrorKind::DuplicateKey` at the repeat's quote, in
  place of the object's `EndObject`), as `parse` always did. Code that relied
  on a raw event loop seeing every member must ask for it:
  `Parser::new().duplicate_keys(DuplicateKeys::KeepAll).reader(text)`.
- **Smaller default budgets.** `Limits::new()` allows 1 Mi values (was 4 Mi)
  and adds `max_memory` (64 MiB) on what a parse holds for its tree. A
  document that parsed under the 0.2 defaults but holds more than 1 Mi values
  or 64 MiB of tree now needs `with_max_elements` / `with_max_memory`.
- **New `ErrorKind` variants** `MemoryLimitExceeded` and `InputAfterEnd`
  (`ErrorKind` is `#[non_exhaustive]`; matches already have a wildcard).
- **`Limits` has a new public field**, `max_memory`. `Limits` is
  `#[non_exhaustive]`, so it was already built with `Limits::new()` and the
  `with_*` methods.

### Added

- Typed reading: `FromJson<'de>` (`from_json(&mut Reader<'de>)`, `absent`),
  read straight from the reader with no tree, borrowing strings from the
  input. `from_str`, `from_bytes`, `Parser::decode`, `Parser::decode_bytes`,
  `Reader::decode`, `StreamReader::decode`, `from_value`.
  `Reader::read_object` gives an `ObjectReader` (`next_key`, `field`,
  `value`, `value_with`, `skip`, `raw`, `unknown_field`, `required`, `offset`)
  and `Reader::read_array` an `ArrayReader` (`next_item`, `next_with`, `skip`,
  `end`, `index`, `offset`). `FromJsonKey` for map keys. Impls for every
  primitive, `&str`/`String`/`Cow<str>`/`Box<str>`/`char`, `Option`, `Box`,
  `Rc`, `Arc`, `Vec`, `VecDeque`, `Box<[T]>`, arrays, tuples up to eight,
  `BTreeMap`/`HashMap` and `BTreeSet`/`HashSet`, and the crate's `Value`,
  `Number`, `Array`, `Object`.
- `DecodeError` (one pointer wide): `DecodeErrorKind` (`Syntax`,
  `WrongType`, `OutOfRange`, `StringNotBorrowable`, `WrongLength`,
  `MissingField`, `UnknownField`, `DuplicateField`, `InvalidValue`, `Custom`),
  the byte offset, and the path to the value (`PathSegment`, `pointer()` as
  RFC 6901), built only on failure.
- Typed writing: `ToJson` (`to_json(&self, ValueWriter<'_>) -> Written`,
  `is_absent`), written straight to the output with no tree. `ValueWriter`
  (`null`, `bool`, `i64`, `u64`, `i128`, `u128`, `f64`, `f32`, `number`,
  `str`, `raw`, `value`, `json`, `object`, `array`), `ObjectWriter`
  (`member`, `optional`, `member_with`), `ArrayWriter` (`item`, `item_with`,
  `items`), `Written`, `ToJsonKey`. The writer types make one well-formed
  value per `to_json` a compile-time check. Impls mirror `FromJson`'s.
- `json_struct!`: a `macro_rules!` helper that writes a plain struct's
  `FromJson` and `ToJson` (keys explicit or the field name; `Option` fields
  optional).
- `Nullable<T>` (`Absent`, `Null`, `Value`) to keep LSP's `key?: T | null`
  apart. `RawValue<'a>`: one validated value kept as text (`new`,
  `from_string`, `encode`, `as_str`, `into_owned`, `parse`, `decode`), taken
  from a reader with no second scan (`Reader::read_raw_value`) and written
  verbatim.
- `StreamReader`, the incremental reader: `feed` bytes as they arrive (any
  split, inside tokens and UTF-8 characters), events out as they complete,
  with the same grammar, budgets, errors, and offsets as `Reader`; tokens cut
  by a chunk boundary continue where they stopped (linear time fed a byte at a
  time); consumed input is dropped as more is fed. `Step<T>`, `end_input`,
  `read_from` (`std`), `next_event`, `read_raw`, `read_value`, `peek_kind`,
  `buffered`, `Parser::stream_reader`.
- Document sequences (NDJSON, concatenated JSON): `Reader::documents`,
  `StreamReader::documents`.
- `Writer`, a push writer: `begin_object`, `end_object`, `begin_array`,
  `end_array`, `key`, `value` (any `ToJson`), `event`, `finish`, to a
  `String`, an `io::Write` (`std`, in 8 KiB chunks), or a `fmt::Write`, every
  call checked (`WriteError`).
- `Format` (`compact`, `pretty`, `with_indent`, `html_safe`, `ascii`) and
  `to_string_with`, `write_value_with`, `to_writer_with`.
- `Numbers` (`IntegerText`, `WholeValues`) and `Parser::numbers`: typed
  integer reads from integer text only (the default) or from any spelling of
  a whole number. `Number::whole_value` (exact), `as_i128`, `as_u128`,
  `as_f32`.
- `Reader::peek_kind`, `Reader::find_pointer` (a JSON Pointer lookup that
  builds no tree).
- `Error::line_col` and `Error::utf16_position` (an LSP position), and the
  same on `DecodeError`.
- `Value::is_bool`, `is_number`, `is_string`, `is_array`, `is_object`,
  `as_i128`, `as_u128`, `pointer_mut`; `Hash` for `Value` and `Object`
  (iterative, consistent with `==`); `PartialEq<i128>`, `PartialEq<u128>`,
  `PartialEq<f32>`; `Object::retain`, `sort_keys`, `entry` (`Entry`,
  `OccupiedEntry`, `VacantEntry`).
- `Limits::max_memory` and `with_max_memory`; `ErrorKind::MemoryLimitExceeded`
  and `ErrorKind::InputAfterEnd`.
- cargo-fuzz targets for every reading entry point (`parse`, `reader`,
  `stream`, `decode`, `write`) under `fuzz/`, sharing their checks with
  `tests/fuzz_harness.rs`, which runs them on arbitrary bytes in plain
  `cargo test`. CI jobs for `no_std` (`thumbv7em-none-eabihf`), 32-bit
  (`i686-unknown-linux-gnu`), a fuzz smoke run, line coverage (at least 95%),
  `cargo-semver-checks`, and a weekly `cargo audit`.
- `examples/typed_lsp.rs`.

### Changed

- The reader's grammar state machine is shared by `Reader` and
  `StreamReader` and is resumable: a token cut off by the end of the input so
  far continues where it stopped.
- Whitespace is skipped eight bytes at a time and strings scanned sixteen at
  a time once past their first word; the short-token paths are inlined; an
  integer followed by a delimiter is read without the number grammar's state
  machine, and `Number::as_i64`/`as_u64` convert it in one pass.
- The reader keeps its own state inline: the open containers as a stack of
  bits, and under `DuplicateKeys::Error` the first 8 open objects and 16 of
  their keys, each key as a one-word fingerprint behind a per-object 64-bit
  Bloom filter (compared with earlier keys only on a filter hit). Reading a
  message of ordinary shape allocates nothing for the reader's state, and
  the tree builder's container stack is inline too: parsing an LSP hover's
  params makes 5 allocations (0.2: 7), with duplicate keys refused on every
  path. Measured against 0.2.0 on the same machine, `parse` takes 0.80-0.93
  of 0.2's time on a 110-byte message and 0.80-1.02 on a 1 MiB document, and
  the default-policy reader 0.77-0.81 on the small message (see the release
  notes).
- The value writer has one copy of its loop per layout (compact or pretty,
  plain or strict escaping), picked once per call: compact output of the
  1 MiB benchmark document takes 0.81-1.01 of 0.2's time.
- The outermost array of a parse becomes its value in place, without a copy
  of its items.
- `json!` takes single-token items up to 32 at a time and literal-keyed
  members up to 16 at a time: literals of thousands of items fit the default
  recursion limit (it failed at 63 array items and 42 members).
- The crate root also denies `warnings` and `clippy::unreachable`.

### Fixed

- The `Reader` let a repeated key through (the last value won) where `parse`
  refused it, so the documented JSON-RPC envelope loop could be handed a
  smuggled second `method` (audit JX-SEC-02).
- The default budgets allowed a parse to hold 32 times its input: 256 MiB of
  heap from 8 MiB of `[0,0,...]` (audit JX-SEC-01). The memory budget, the
  in-place outermost array, and the smaller element budget bound it.
- A deduplicated object (`First`/`Last`) grew its member vector by
  reallocation; it is now sized exactly.
- `tests/api.rs` assumed a 64-bit `usize`; the suite now passes on 32-bit
  targets.
- Doctests did not build from a `git archive` tarball (`docs/` was
  export-ignored); the nightly-only rustfmt options are gone, so `cargo fmt
  --check` is the same on stable and nightly.
- Documentation claims the audit found false: the duplicate-key guarantee
  (now true of every reading path), "exactly RFC 8259" (repeated keys and
  lone surrogates are refused by choice), `read_raw` "allocating nothing"
  (the reader's own state grows with depth), the default budgets "cannot
  exhaust memory" (now bounded by `max_memory`), `Value` comparing with
  "every integer type" (now true up to 128 bits), the decode buffer being
  "exactly" sized (it is sized from the escaped source), and `json!`'s
  "very long literals".

---

## [0.2.0] - 2026-10-09

The foundation: a strict RFC 8259 tokenizer, a borrowing document model, a
pull reader, and a deterministic writer, with budgets checked before
allocation and nothing that recurses on nesting depth.

### Added

- Tier 1: `parse`, `parse_bytes`, `to_string`, `to_string_pretty`.
- Tier 2: `Parser` (a `Copy` configuration usable in a `const`: `new`,
  `limits`, `duplicate_keys`, `get_limits`, `get_duplicate_keys`, `parse`,
  `parse_bytes`, `reader`, `reader_bytes`), `Limits` (depth, input bytes,
  string bytes, number bytes, element count; `new`, `unlimited`, `with_*`),
  and `DuplicateKeys` (`Error` by default, `First`, `Last`, `KeepAll`).
- Tier 3: `Reader`, a pull parser yielding `Event`s, with `read_raw` (a
  value's exact source text, validated, nothing decoded or allocated: the
  `RawValue` use case), `read_value` (build just one part of a document),
  `finish`, `depth`, `offset`, `event_offset`, and `Iterator`.
- The document model: `Value` (six variants; typed accessors `as_bool`,
  `as_number`, `as_i64`, `as_u64`, `as_f64`, `as_str`, `as_array`,
  `as_object` and their `_mut` forms; `get`, `get_mut`, `get_index`, RFC 6901
  `pointer`, `take`, `into_owned`, `kind`; panic-free `Index` by key and
  position; `From` conversions; comparisons with Rust strings, booleans, and
  numbers), `Kind`, `Number`, `Array`, `Object` (ordered members, `get`,
  `insert`, `push`, `remove`, `iter`, `keys`, `values`, and more), `Iter`,
  `IterMut`.
- Strings without escapes borrow from the input; strings with escapes are
  decoded once into a buffer of exactly their source size.
- `Number` keeps the exact text; `as_i64`/`as_u64` succeed only for integer
  text in range, `as_f64` is correctly rounded and `None` when not finite;
  `from_text` and `from_f64` (shortest round-trip text).
- The writer: compact and pretty layouts, `write_value`, `write_value_pretty`,
  `write_string` (complete escaping, including U+2028 and U+2029), and with
  `std`, `to_writer` and `to_writer_pretty` (buffered, chunked). `Display`
  writes compact JSON, `{:#}` pretty.
- The `json!` macro.
- `Error` (byte offset + `ErrorKind`, 21 `#[non_exhaustive]` kinds, with
  `is_budget`).
- Tests: JSONTestSuite-style y/n/i cases as data with exact error kinds and
  offsets; an independent strict reference parser; property tests for
  conformance (verdict, value, and exact error offset against the reference),
  round trip, escaping no-injection, truncation, `read_raw` slices, and no
  panics on arbitrary bytes; million-level nesting on a 256 KiB stack; every
  budget at its boundaries; a counting allocator proving budgets refuse
  before allocating.
- Criterion benchmarks on 1 MB and 100 MB documents; `examples/quick_start.rs`
  and `examples/rpc_envelope.rs`.

### Changed

- The crate description now says what the crate is.

---

## [0.1.0] - 2026-10-08

Initial scaffold and repository bootstrap. No domain logic yet &mdash; this release establishes the structure, tooling, and quality gates the implementation will be built on.

### Added

- `Cargo.toml` with crate metadata, Rust 2024 edition, MSRV 1.85.
- Dual `Apache-2.0 OR MIT` license files.
- `README.md`, `CHANGELOG.md`, and a documentation skeleton.
- `REPS.md` compliance baseline.
- `.github/workflows/ci.yml` CI matrix; `deny.toml`, `clippy.toml`, `rustfmt.toml`.
- `dev/DIRECTIVES.md` and `dev/ROADMAP.md` (committed engineering standards + plan).

[Unreleased]: https://github.com/jamesgober/jsonx-lib/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/jamesgober/jsonx-lib/compare/v0.2.0...v0.5.0
[0.2.0]: https://github.com/jamesgober/jsonx-lib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jamesgober/jsonx-lib/releases/tag/v0.1.0
