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

[Unreleased]: https://github.com/jamesgober/jsonx-lib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/jamesgober/jsonx-lib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jamesgober/jsonx-lib/releases/tag/v0.1.0
