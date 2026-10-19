//! Criterion benchmarks on realistic documents of 1 MB and 100 MB.
//!
//! The document is LSP-shaped: completion items with ranges, labels, escaped
//! detail strings, a non-ASCII escape, floats, booleans, and small integer
//! arrays, so every scanner path (plain strings, escapes, numbers, literals,
//! structure) carries its real share of the work.
//!
//! Run with `cargo bench`. The 100 MB group takes a few minutes.

use std::{fmt::Write as _, hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use jsonx_lib::{DuplicateKeys, Event, Limits, Number, Parser, Step, Value};

/// No budgets: the 100 MB document is over the default 64 MiB input budget.
const UNLIMITED: Parser = Parser::new().limits(Limits::unlimited());

/// Builds a document of at least `target` bytes.
fn document(target: usize) -> String {
    let mut out = String::with_capacity(target + 512);
    out.push('[');
    let mut i: u64 = 0;
    while out.len() < target {
        if i > 0 {
            out.push(',');
        }
        let line = i % 5000;
        let _infallible = write!(
            out,
            "{{\"id\":{i},\"label\":\"item_{i}\",\"detail\":\"fn item_{i}(x: i32) -> \\\"Result\\\"\\n\",\
             \"kind\":{kind},\"score\":{score},\"deprecated\":{dep},\
             \"range\":{{\"start\":{{\"line\":{line},\"character\":4}},\"end\":{{\"line\":{line},\"character\":{end}}}}},\
             \"tags\":[\"plain\",\"caf\\u00e9\",null],\"data\":[{i},{neg},0]}}",
            kind = i % 25,
            score = (i as f64) * 0.125 + 0.5,
            dep = i % 7 == 0,
            end = 4 + i % 40,
            neg = -((i % 1000) as i64),
        );
        i += 1;
    }
    out.push(']');
    out
}

fn bench_size(c: &mut Criterion, name: &str, bytes: usize, samples: usize, seconds: u64) {
    let text = document(bytes);
    let doc = UNLIMITED.parse(&text).unwrap_or_default();
    let mut group = c.benchmark_group(name);
    group
        .sample_size(samples)
        .measurement_time(Duration::from_secs(seconds));
    group.throughput(Throughput::Bytes(text.len() as u64));

    group.bench_function("parse", |b| b.iter(|| UNLIMITED.parse(black_box(&text))));
    group.bench_function("parse_duplicate_keys_kept", |b| {
        let p = UNLIMITED.duplicate_keys(DuplicateKeys::KeepAll);
        b.iter(|| p.parse(black_box(&text)));
    });
    group.bench_function("reader_events", |b| {
        b.iter(|| {
            let mut reader = UNLIMITED.reader(black_box(&text));
            let mut n = 0usize;
            while let Ok(Some(event)) = reader.next_event() {
                n += usize::from(!matches!(event, Event::Null));
            }
            n
        });
    });
    group.bench_function("read_raw_validate_only", |b| {
        b.iter(|| {
            UNLIMITED
                .reader(black_box(&text))
                .read_raw()
                .map(|r| r.map(str::len))
        });
    });
    group.bench_function("to_string", |b| {
        b.iter(|| jsonx_lib::to_string(black_box(&doc)))
    });
    group.bench_function("to_string_pretty", |b| {
        b.iter(|| jsonx_lib::to_string_pretty(black_box(&doc)))
    });
    group.bench_function("clone_tree", |b| b.iter(|| black_box(&doc).clone()));
    group.finish();
}

fn bench_1mb(c: &mut Criterion) {
    bench_size(c, "doc_1mb", 1 << 20, 50, 5);
}

fn bench_100mb(c: &mut Criterion) {
    bench_size(c, "doc_100mb", 100 << 20, 10, 20);
}

/// The 1 MB document pretty-printed: whitespace is about a third of it, so
/// skipping indentation is a real share of the work.
fn bench_pretty(c: &mut Criterion) {
    let text =
        jsonx_lib::to_string_pretty(&UNLIMITED.parse(&document(1 << 20)).unwrap_or_default());
    let mut group = c.benchmark_group("pretty_1mb");
    group
        .sample_size(50)
        .measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Bytes(text.len() as u64));
    group.bench_function("reader_events", |b| {
        b.iter(|| {
            let mut reader = UNLIMITED.reader(black_box(&text));
            let mut n = 0usize;
            while let Ok(Some(event)) = reader.next_event() {
                n += usize::from(!matches!(event, Event::Null));
            }
            n
        });
    });
    group.bench_function("read_raw_validate_only", |b| {
        b.iter(|| {
            UNLIMITED
                .reader(black_box(&text))
                .read_raw()
                .map(|r| r.map(str::len))
        });
    });
    group.finish();
}

fn bench_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("strings_1mb");
    let sparse: String = "The quick brown fox jumps over the lazy dog. ".repeat(23_302);
    let dense: String = "line\n\t\"quoted\"\\\u{2028}\u{1}".repeat(52_428);
    for (label, text) in [("sparse_escapes", &sparse), ("dense_escapes", &dense)] {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::new("write_string", label), text, |b, t| {
            let mut out = String::with_capacity(t.len() * 2);
            b.iter(|| {
                out.clear();
                jsonx_lib::write_string(&mut out, black_box(t));
                out.len()
            });
        });
        let mut quoted = String::new();
        jsonx_lib::write_string(&mut quoted, text);
        group.bench_with_input(BenchmarkId::new("parse_string", label), &quoted, |b, q| {
            b.iter(|| {
                UNLIMITED
                    .parse(black_box(q))
                    .map(|v| v.as_str().map(str::len))
            });
        });
    }
    group.finish();
}

fn bench_numbers(c: &mut Criterion) {
    let mut group = c.benchmark_group("numbers");
    let floats: Vec<f64> = (0..10_000)
        .map(|i| f64::from(i) * 1.618_033_988_749_895 / 7.0)
        .collect();
    group.throughput(Throughput::Elements(floats.len() as u64));
    group.bench_function("from_f64_shortest", |b| {
        b.iter(|| {
            floats
                .iter()
                .filter_map(|&x| Number::from_f64(black_box(x)))
                .count()
        });
    });
    let texts: Vec<Number<'static>> = floats.iter().filter_map(|&x| Number::from_f64(x)).collect();
    group.bench_function("as_f64", |b| {
        b.iter(|| {
            texts
                .iter()
                .filter_map(|n| black_box(n).as_f64())
                .sum::<f64>()
        });
    });
    let ints: Vec<Value<'static>> = (0..10_000i64)
        .map(|i| Value::from(i * 7919 - 39_595_000))
        .collect();
    group.bench_function("as_i64", |b| {
        b.iter(|| {
            ints.iter()
                .filter_map(|v| black_box(v).as_i64())
                .sum::<i64>()
        });
    });
    group.finish();
}

/// The incremental reader over the 1 MB document, fed in network-sized
/// chunks, against the complete reader over the same text.
fn bench_stream(c: &mut Criterion) {
    let text = document(1 << 20);
    let mut group = c.benchmark_group("stream_1mb");
    group
        .sample_size(50)
        .measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Bytes(text.len() as u64));
    for chunk in [1500usize, 64 * 1024] {
        group.bench_with_input(BenchmarkId::new("events", chunk), &chunk, |b, &chunk| {
            b.iter(|| {
                let mut reader = UNLIMITED.stream_reader();
                let mut n = 0usize;
                for piece in black_box(&text).as_bytes().chunks(chunk) {
                    if reader.feed(piece).is_err() {
                        break;
                    }
                    while let Ok(Step::Ready(Some(event))) = reader.next_event() {
                        n += usize::from(!matches!(event, Event::Null));
                    }
                }
                reader.end_input();
                n
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_1mb,
    bench_pretty,
    bench_stream,
    bench_strings,
    bench_numbers,
    bench_100mb
);
criterion_main!(benches);
