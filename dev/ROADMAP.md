# jsonx-lib - Roadmap

> Path from scaffold to a stable 1.0. Hard parts are front-loaded; each phase has hard exit criteria.
> Master plan: C:/Dev/libraries/Rust/lang-collection/_lexersketch/ROADMAP.md and NEW-LIBS.md
>
> **Anti-deferral rule:** no listed hard task moves to a later phase unless this file records the move and the reason.

## v0.1.0 - Scaffold (DONE)
Compiles, CI green, structure correct, no domain logic.
- [x] Manifest, README, CHANGELOG, REPS, dual license, CI, deny, clippy, rustfmt, DIRECTIVES, ROADMAP.

## v0.2.0 - Foundation (DONE, 2026-10-09)
- [x] Tokenizer, borrowing document model, writer with exact escaping, budgets, round-trip and conformance property tests (JSONTestSuite-style cases as data).

Delivered:
- Strict tokenizer (`src/lexer.rs`) with byte-offset errors: `Error` + 21
  `#[non_exhaustive]` `ErrorKind`s. The offset is the first byte at which the
  input stops being the beginning of any valid JSON text (property-tested
  against an independent reference parser); the documented exceptions are
  lone surrogates (the escape's backslash), duplicate keys (the second
  occurrence), and budgets.
- Pull reader (`Reader` + `Event`): the one state machine every parse goes
  through; iterative (one byte of state per level); `read_raw` returns a
  value's exact validated source text without decoding (the `RawValue` use
  case lsp-lang needs), `read_value` builds one part of a document.
- Borrowing document model: `Value`, `Array`, `Object` (ordered members),
  `Number` (exact text; lazy `as_i64`/`as_u64`/`as_f64`), `Kind`; strings
  borrowed unless escaped, escaped strings decoded once into an exactly-sized
  buffer; configurable `DuplicateKeys` (`Error` by default, `First`, `Last`,
  `KeepAll`), checked in `O(n log n)` per object.
- Budgets (`Limits`: depth, input bytes, string bytes, number bytes, element
  count) checked before the memory they guard is allocated (proven with a
  counting allocator).
- No recursion on depth: parse, write, clone, equality, and `into_owned`
  loop over explicit stacks; drop recurses at most a fixed 32 levels, then
  flattens (million-level nesting tested on a 256 KiB stack).
- Writer: compact and pretty, deterministic, complete escaping (control
  characters, `"`, `\`, U+2028, U+2029), shortest round-trip `f64` text,
  `std::io` output through a chunked buffer, `Display`/`{:#}`; `json!` macro.
- Tier-1 `parse`/`parse_bytes`/`to_string`/`to_string_pretty`; Tier-2
  `Parser` (a `Copy` config usable in a `const`).
- Proof: y/n/i conformance data with exact kinds and offsets, reference
  cross-check, round-trip, no-injection, truncation, and no-panic properties;
  criterion benches at 1 MB and 100 MB.

Dependency wiring (decided here, recorded per the anti-deferral rule):
- **None wired.** jsonx-lib is a leaf: it needs nothing from the `-lang`
  family (byte offsets are plain `usize`, not `span-lang` spans, so that
  rpc-io, lsp-lang, and non-language consumers can all use it without pulling
  in the language stack). Dev-dependencies: `criterion`, `proptest` only.

Moved to a later phase (anti-deferral rule):
- **Incremental (chunk-fed) reader → v0.5.0.** The 0.2 `Reader` streams
  events without building a tree, but over a complete in-memory document. A
  reader that accepts bytes as they arrive needs owned token buffers across
  chunk boundaries and a resumable lexer; it belongs with the streaming writer
  below, and no 0.2 consumer needs it (LSP/DAP/MCP framing delivers whole
  messages). `ErrorKind::UnexpectedEnd` at the input's length already tells a
  caller "the message is incomplete".
- **Push (event) writer → v0.5.0**, as already planned there. 0.2 writes from
  a `Value`; lsp-lang's `Reply::new` currently serializes structs straight to
  text, which needs either the push writer or a `Value` built first.
- **Fuzz target → v0.9.0**, as already planned there (DIRECTIVES §3.6). 0.2
  covers the same ground with proptest: arbitrary bytes, JSON-like token soup,
  and mutated valid documents through every entry point, never panicking.

Open for the coordinator (a design question, not deferred work):
- lsp-lang uses `serde` derives (`Deserialize` into borrowed structs,
  `Serialize` for replies) and re-exports `serde_json::{Value, json}` in its
  public API. DIRECTIVES §1 rules out reflection-based mapping, so dropping
  serde_json (ISSUES M76) means lsp-lang hand-reads messages with `Reader` /
  `Value` accessors and builds replies with `json!`/`Object`, which changes
  lsp-lang's public API. If that is too much churn, an explicit,
  hand-implemented `ToJson`/`FromJson` trait pair (no derive) is the
  candidate for v0.5.0.
  **Decided (2026-10):** the owner chose the trait pair; delivered in v0.5.0
  (below), with `json_struct!` for plain structs.

## v0.5.0 - Implementation (DONE, 2026-10-19)
- [x] Streaming reader/writer, SIMD-friendly scanning, number handling options (exact decimal text, i64/u64/f64).
- [x] Incremental (chunk-fed) reader (moved from 0.2, see above).
- [x] Push (event) writer for serializing without building a `Value`.
- [x] Typed reading and writing: an explicit, hand-implemented `ToJson`/`FromJson` pair (decided by the owner for this release; no derive, no reflection, DIRECTIVES §1).
- [x] Fuzzing (moved forward from 0.9, see below).
- [x] The baseline audit's findings (`_audits/2026-10-14/jsonx-lib.md`).

Delivered:
- **Typed values.** `FromJson<'de>` reads a type straight from a `Reader`
  (no tree; `&'de str` and `Cow<'de, str>` borrow from the input), through
  `ObjectReader`/`ArrayReader`, which refuse a repeated field, skip unknown
  members (LSP and MCP require receivers to ignore them), and give
  `MissingField` unless the type allows absence. `DecodeError` carries the
  kind, the byte offset, and a JSON Pointer path built on the way out.
  `ToJson` writes through a typed `ValueWriter` whose methods consume it, so
  one well-formed value per impl is a compile-time check; it is object-safe.
  Impls for the primitives, strings, `Option`, pointers, sequences, arrays,
  tuples, string-keyed maps, sets, and the crate's own types. `Nullable<T>`
  keeps absent and `null` apart; `RawValue` keeps a validated value as text.
  `json_struct!` (a `macro_rules!` macro over the field list you write)
  generates both impls for a plain struct; a proc-macro derive stays out.
- **Streaming reader.** `StreamReader`: bytes fed in chunks, the same state
  machine as `Reader` (one implementation, made resumable: a token cut by a
  chunk boundary continues where it stopped), UTF-8 checked as it arrives,
  consumed input dropped. Both readers read document sequences (NDJSON).
- **Streaming writer.** The push `Writer` (events in, checked; `String`,
  `io::Write`, `fmt::Write`), and `to_writer` for any `ToJson`, in 8 KiB
  chunks.
- **Scanning.** SWAR (no `unsafe`, so no Miri case): whitespace runs eight
  bytes at a time, strings sixteen at a time past their first word, the
  short-token paths inlined. `std::arch` was not used: its loads need
  `unsafe`, against `#![forbid(unsafe_code)]`, and portable SIMD
  (`core::simd`) is not stable.
- **Numbers.** `Numbers::{IntegerText, WholeValues}` for typed integer reads;
  `Number::whole_value` (exact decimal arithmetic), `as_i128`, `as_u128`,
  `as_f32`; shortest `f32` and `f64` output.
- **Output formats.** `Format`: indent width, HTML-safe and ASCII-only
  escaping.
- **Audit fixes.** Duplicate keys refused on every reading path (JX-SEC-02);
  a `max_memory` budget, the outermost array built in place, 1 Mi elements by
  default (JX-SEC-01); `json!` in batches (JX-COR-02); error line/column and
  LSP positions; `Value`/`Object` accessor gaps; `Reader::find_pointer`;
  32-bit, `no_std`, fuzz, coverage, semver, and scheduled-audit CI jobs;
  the crate-root lints; nightly-proof rustfmt config; docs in source
  archives; eight false documentation claims corrected.
- **Fuzzing.** Five cargo-fuzz targets (`parse`, `reader`, `stream`,
  `decode`, `write`) covering every reading entry point, with the audit's
  oracles plus stream-vs-reader and transcoding agreement; the same checks run
  on arbitrary bytes in `cargo test` (`tests/fuzz_harness.rs`), since
  libFuzzer does not build on Windows MSVC.

Moved to a later phase, or decided (anti-deferral rule):
- **Fuzz target: 0.9 → 0.5**, as rpc-io did: a parser for untrusted input
  should not wait (audit JX-SEC-03). 0.9 keeps long campaigns and a corpus.
- **A dedicated fast validator for `read_raw` → v0.9.0** (audit JX-PERF-01,
  a Should). `read_raw` is now faster than 0.2 under the default policy
  (which 0.2's reader did not check), but still well behind
  `serde_json`'s `IgnoredAny`. Closing that gap needs a second
  implementation of the grammar, which must agree with the first to the byte;
  measured here, driving the shared state machine without events gained
  nothing, so the step itself is the cost. It belongs with 0.9's benchmarks
  against published numbers, where it can be measured against the
  alternatives rather than guessed at.
- **Cross-crate benchmarks → v0.9.0**, as planned (they need `serde_json`
  and friends as dev-dependencies).
- **RFC 8785 canonical writer (JCS) → after 1.0** (audit nice-to-have): its
  number form is ECMAScript's `Number.prototype.toString`, a second float
  formatter to get exactly right; no consumer needs it yet.
- **Arena or inline-storage tree → v0.9.0 profiling** (audit JX-PERF-02, a
  nice-to-have): the tree build is allocator-bound and the change is large.
  Measured here, `parse` builds the 1 MiB benchmark document about 1.4 times
  as fast as `serde_json::Value` (204-232 against 136-170 MiB/s), but a
  110-byte message about 10% slower (409-472 against 364-420 ns), where the
  per-object allocations dominate: the case an arena would help.
- **`Array: Deref<Target = Vec<Value>>` and `Object::as_slice` (JX-API-01):
  decided, kept for 1.0.** Vectors are the representation the crate is
  designed around (ordered members, linear lookup for small objects); a hash
  index for large objects can be added beside them without changing these.
- **Dev-dependency upgrades (JX-HLT-04): not taken.** criterion 0.8 and
  proptest 1.12 need Rust 1.86 and 1.88, above the 1.85 MSRV that CI builds
  benches and tests with.

## v0.9.0 - Hardening
- [x] Fuzz targets (delivered in 0.5, see above).
- [ ] Long fuzz campaigns with a kept corpus.
- [ ] Benchmarks against published numbers and other JSON crates; a dedicated `read_raw` validator if they show it pays (moved from 0.5, see above).
- [ ] Allocation profile of the tree build (an arena or inline storage, moved from 0.5).

## v1.0.0 - Stable
- [ ] Frozen after lsp-lang and the LexerSketch API use it (D18).
