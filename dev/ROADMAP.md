# jsonx-lib - Roadmap

> Path from scaffold to a stable 1.0. Hard parts are front-loaded; each phase has hard exit criteria.
> Master plan: C:/Dev/libraries/Rust/lang-collection/_lexersketch/ROADMAP.md and NEW-LIBS.md
>
> **Anti-deferral rule:** no listed hard task moves to a later phase unless this file records the move and the reason.

## v0.1.0 - Scaffold (DONE)
Compiles, CI green, structure correct, no domain logic.
- [x] Manifest, README, CHANGELOG, REPS, dual license, CI, deny, clippy, rustfmt, DIRECTIVES, ROADMAP.

## v0.2.0 - Foundation
- [ ] Tokenizer, borrowing document model, writer with exact escaping, budgets, round-trip and conformance property tests (JSONTestSuite-style cases as data).

## v0.5.0 - Implementation
- [ ] Streaming reader/writer, SIMD-friendly scanning, number handling options (exact decimal text, i64/u64/f64).

## v0.9.0 - Hardening
- [ ] Fuzzing; benchmarks against published numbers.

## v1.0.0 - Stable
- [ ] Frozen after lsp-lang and the LexerSketch API use it (D18).
