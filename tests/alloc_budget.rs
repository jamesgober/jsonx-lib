//! Budgets are checked before allocation: a counting global allocator measures
//! the peak heap growth while hostile documents are refused.
//!
//! This test binary installs its own allocator, so it holds a single test
//! that runs every scenario in sequence, with no other test thread allocating
//! at the same time.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
};

use jsonx_lib::{ErrorKind, Limits, Parser};

/// Forwards to the system allocator and tracks live and peak bytes.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(bytes: usize) {
    let now = LIVE.fetch_add(bytes, Relaxed) + bytes;
    let _previous = PEAK.fetch_max(now, Relaxed);
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; the counters only observe sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `alloc`'s contract for `layout`.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            grew(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `alloc`/`realloc` above with this `layout`.
        unsafe { System.dealloc(ptr, layout) };
        let _previous = LIVE.fetch_sub(layout.size(), Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds `realloc`'s contract.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            let _previous = LIVE.fetch_sub(layout.size(), Relaxed);
            grew(new_size);
        }
        new
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Runs `f` and returns its result with the peak heap growth during the call.
fn peak_during<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    let result = f();
    (result, PEAK.load(Relaxed).saturating_sub(base))
}

fn kind<T>(r: &Result<T, jsonx_lib::Error>) -> Option<ErrorKind> {
    r.as_ref().err().map(jsonx_lib::Error::kind)
}

#[test]
fn test_budgets_hostile_input_refused_before_allocating() {
    let tight = Parser::new().limits(
        Limits::new()
            .with_max_string_bytes(1024)
            .with_max_depth(128)
            .with_max_elements(1000)
            .with_max_input_bytes(1 << 30),
    );

    // An 8 MiB string against a 1 KiB string budget: nothing is copied.
    let plain = format!("\"{}\"", "a".repeat(8 << 20));
    let (r, peak) = peak_during(|| tight.parse(&plain));
    assert_eq!(kind(&r), Some(ErrorKind::StringTooLong));
    assert!(peak < 1024, "plain string: {peak} bytes");

    // The same with escapes, which would otherwise need decoding.
    let escaped = format!("\"{}\"", "\\n".repeat(4 << 20));
    let (r, peak) = peak_during(|| tight.parse(&escaped));
    assert_eq!(kind(&r), Some(ErrorKind::StringTooLong));
    assert!(peak < 1024, "escaped string: {peak} bytes");

    // Four million open brackets against a depth budget of 128.
    let deep = "[".repeat(4 << 20);
    let (r, peak) = peak_during(|| tight.parse(&deep));
    assert_eq!(kind(&r), Some(ErrorKind::DepthLimitExceeded));
    assert!(peak < 16 << 10, "deep: {peak} bytes");

    // A million items against an element budget of 1000.
    let wide = format!("[{}0]", "0,".repeat(1 << 20));
    let (r, peak) = peak_during(|| tight.parse(&wide));
    assert_eq!(kind(&r), Some(ErrorKind::TooManyElements));
    assert!(peak < 128 << 10, "wide: {peak} bytes");

    // Too large overall: refused before reading a byte.
    let small_input = Parser::new().limits(Limits::new().with_max_input_bytes(1024));
    let (r, peak) = peak_during(|| small_input.parse(&plain));
    assert_eq!(kind(&r), Some(ErrorKind::InputTooLarge));
    assert_eq!(peak, 0, "input too large");

    // A string within budget is decoded into one buffer of its source size:
    // never more, and never grown by reallocation.
    let body = "\\n".repeat(500);
    let within = format!("\"{body}\"");
    let (r, peak) = peak_during(|| tight.parse(&within).map(|v| v.as_str().map(str::len)));
    assert_eq!(r.ok().flatten(), Some(500));
    assert!(
        peak <= body.len(),
        "decode: {peak} bytes for a {}-byte body",
        body.len()
    );

    // Deduplicating (`First`/`Last`) sizes the object exactly: one
    // allocation of the kept members, never grown.
    let mut dup = String::from("{");
    for i in 0..64 {
        dup.push_str(&format!("\"k{i}\":{i},"));
    }
    dup.push_str("\"k0\":0}");
    for policy in [
        jsonx_lib::DuplicateKeys::First,
        jsonx_lib::DuplicateKeys::Last,
    ] {
        let p = Parser::new().duplicate_keys(policy);
        let (r, peak) = peak_during(|| {
            p.parse(&dup)
                .map(|v| v.as_object().map(|o| (o.len(), o.as_slice().len())))
        });
        assert_eq!(r.ok().flatten(), Some((64, 64)));
        let member = std::mem::size_of::<(std::borrow::Cow<str>, jsonx_lib::Value)>();
        let value = std::mem::size_of::<jsonx_lib::Value>();
        // The stacks plus one exactly sized member vector; growing it would
        // have held a second, smaller one too.
        assert!(
            peak <= 128 * value + 128 * 24 + 64 * member + 4096,
            "{policy:?}: {peak}"
        );
    }

    // JX-SEC-01: under the default budgets no shape of document makes a
    // parse hold more than `max_memory` (64 MiB), counting the moment a
    // stack moves to a bigger buffer. 0.2 reached 256 MiB on 8 MiB of input.
    let defaults = Parser::new();
    let max = Limits::new().max_memory;
    let flat = format!("[{}0]", "0,".repeat(4 << 20));
    let nested = format!("{{\"a\":{flat}}}");
    let pairs = format!("[{}[0]]", "[0],".repeat(2 << 20));
    let members = format!("[{}{{}}]", "{\"\":0},".repeat(1 << 20));
    let escapes = format!("[{}\"\"]", "\"\\n\",".repeat(4 << 20));
    for (name, doc) in [
        ("flat", &flat),
        ("nested", &nested),
        ("pairs", &pairs),
        ("members", &members),
        ("escapes", &escapes),
    ] {
        let (r, peak) = peak_during(|| defaults.parse(doc).map(|v| v.is_null()));
        assert!(r.is_err(), "{name} should be over a budget");
        assert!(peak <= max + (1 << 20), "{name}: {} MiB", peak >> 20);
    }
    // The memory budget itself: a tight one refuses before it is passed.
    let tight = Parser::new().limits(Limits::unlimited().with_max_memory(1 << 20));
    for doc in [&flat, &pairs, &members] {
        let (r, peak) = peak_during(|| tight.parse(doc).map(|v| v.is_null()));
        assert_eq!(kind(&r), Some(ErrorKind::MemoryLimitExceeded));
        assert!(peak <= (1 << 20) + 4096, "tight: {peak}");
    }
    // An array at the top takes the stack in place: one buffer, not two.
    let ok = format!("[{}0]", "0,".repeat(100_000));
    let (r, peak) = peak_during(|| defaults.parse(&ok).map(|v| v.as_array().map(|a| a.len())));
    assert_eq!(r.ok().flatten(), Some(100_001));
    let value = std::mem::size_of::<jsonx_lib::Value>();
    assert!(peak <= 2 * 131_072 * value, "root array: {peak}");

    // A string without escapes is borrowed: parsing it allocates nothing.
    let borrowed = format!("\"{}\"", "b".repeat(1000));
    let (r, peak) = peak_during(|| tight.parse(&borrowed).map(|v| v.as_str().map(str::len)));
    assert_eq!(r.ok().flatten(), Some(1000));
    assert_eq!(peak, 0, "borrowed string");
}
