# jsonx-lib &mdash; Engineering Directives

> Engineering standards and the definition of done for this project. Read alongside `REPS.md` (root, authoritative) and `dev/ROADMAP.md` (current phase). If anything here conflicts with `REPS.md`, `REPS.md` wins.
> LexerSketch plan: `C:/Dev/libraries/Rust/lang-collection/_lexersketch/` (README rules of engagement, ARCHITECTURE, ISSUES, ROADMAP).

---

## 0. Philosophy

This library is built and maintained to a production standard and treated as a flagship piece of work. Plan the full path, then build one verified step at a time. "Good enough" is treated as a defect. It is a single-purpose `-lib` in the owner's lib-collection, used across the portfolio (LexerSketch's LSP, DAP, MCP, and emitters first).

---

## 1. What this is

jsonx-lib owns JSON text: a tokenizer, a borrowing document model, a streaming reader and writer, and exact escaping. It does not map JSON onto user types by reflection; typed access is explicit.

LexerSketch's LSP, DAP, MCP servers and its emitters all speak JSON, and the family's rule is no rented runtime dependencies (lsp-lang currently rents serde_json; ISSUES M76).

---

## 2. Engineering law (non-negotiable)

- **Ours, not rented.** No third-party runtime dependency. First-party `-lang` crates only, wired when first used. Dev-only test tooling is allowed.
- **Performance.** Peak is the baseline: allocation-free hot paths where feasible, dense ids instead of maps, no "faster" claim without `criterion` numbers at realistic scale (100k+ items).
- **Correctness.** Every invariant in section 4 is covered by property tests, cross-checked against a simple reference implementation wherever one can be written.
- **Hostile-input hardening.** Any input this crate accepts may be adversarial: explicit budgets (depth, size, steps), iterative algorithms, no quadratic paths, no reachable panics.
- **Architecture.** SOLID, KISS, YAGNI; one responsibility; a Tier-1 one-call entry point headlines the docs.
- **Cross-platform.** Linux, macOS, and Windows are first-class, verified by CI on stable and MSRV 1.85.
- **Error handling.** Every fallible path returns `Result`/`Option` per a documented contract; errors are `#[non_exhaustive]` and actionable.
- **Production-ready.** `#![forbid(unsafe_code)]` and `#![deny(missing_docs)]` from the first commit; no `unwrap`/`expect`/`todo!`/`dbg!`/printing in library code; every public item has rustdoc with a runnable example; every claim in the docs maps to a test.

---

## 3. Definition of done

1. Compiles clean on Linux/macOS/Windows, stable and MSRV 1.85.
2. `fmt`, `clippy -D warnings` (all targets, all/no-default features), `test --all-features`, `cargo doc -D warnings` clean.
3. `cargo audit` + `cargo deny check` pass.
4. No `unwrap`/`expect`/`todo!`/`dbg!` in shipping code.
5. A Tier-1 API exists and headlines the docs.
6. Property tests cover every section-4 invariant; a fuzz target exists for every decoder or parser.
7. Hot-path changes carry benchmarks; no regression over 5%.
8. `README.md`, `docs/API.md`, and `CHANGELOG.md` updated; the matching `docs/release/vX.Y.Z.md` written before the tag.
9. v1.0.0 only after a real consumer has exercised the API end to end (LexerSketch decision D18).

---

## 4. Project-specific invariants

- Accepts exactly RFC 8259 (property-tested against a reference grammar); rejects everything else with a byte offset.
- Writer output always parses back to the same value (round-trip property).
- Budgets on depth, total size, string length, and number length are enforced before allocation; hostile input never panics or overflows the stack.
- Escaping is complete: no input string can alter the structure of emitted JSON.

---

## 5. Recorded deviations from REPS

Where this crate departs from `REPS.md`, and why. Each is deliberate.

- **`Cargo.lock` is not committed** (REPS Reproducibility). The family
  convention for libraries: a dependent resolves its own versions, and CI
  generates a fresh lockfile for `cargo audit`.
- **No `rust-toolchain.toml`** (REPS Reproducibility). CI builds and tests
  every commit on stable and on the 1.85 MSRV explicitly; a pinned toolchain
  file would override that matrix on every runner.
- **No metrics or `tracing`** (REPS Observability). A pure parser and writer
  with no I/O, threads, or state between calls: every outcome is returned to
  the caller, who observes it. A runtime dependency would also break §2.
- **`dev/` is committed** (REPS Git Conventions). The family convention: the
  directives and roadmap are the project's engineering record.
