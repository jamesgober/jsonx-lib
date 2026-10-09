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

## v0.5.0 - Implementation
- [ ] Streaming reader/writer, SIMD-friendly scanning, number handling options (exact decimal text, i64/u64/f64).
- [ ] Incremental (chunk-fed) reader (moved from 0.2, see above).
- [ ] Push (event) writer for serializing without building a `Value`.
- Already delivered in 0.2: exact decimal text with lazy `i64`/`u64`/`f64`;
  SWAR (eight bytes at a time) string scanning in the reader and the writer.

## v0.9.0 - Hardening
- [ ] Fuzzing (cargo-fuzz targets for `parse_bytes` and the `Reader`); benchmarks against published numbers.

## v1.0.0 - Stable
- [ ] Frozen after lsp-lang and the LexerSketch API use it (D18).
