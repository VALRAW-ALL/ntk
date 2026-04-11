// ---------------------------------------------------------------------------
// Etapa 22 — criterion.rs benchmarks
//
// Measures Layer 1, Layer 2, and the full L1+L2 pipeline latency against
// real fixture files.  Targets:
//   Layer 1 (1 KB)   < 1ms
//   Layer 1 (100 KB) < 5ms
//   Layer 2 tokenizer < 20ms
//   Full pipeline (no inference) < 50ms
// ---------------------------------------------------------------------------

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ntk::compressor::{layer1_filter, layer2_tokenizer};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture not found: {}", path.display()))
}

fn synthetic_input(size_bytes: usize) -> String {
    // Simulate cargo build / log output: mix of info lines, warnings, paths.
    let line = "warning: unused variable `x` --> src/lib.rs:42:9\n";
    let repeat = (size_bytes / line.len()).max(1);
    line.repeat(repeat)
}

// ---------------------------------------------------------------------------
// Layer 1 benchmarks
// ---------------------------------------------------------------------------

fn bench_layer1_1kb(c: &mut Criterion) {
    let input = synthetic_input(1_024);
    c.bench_function("layer1_1kb", |b| {
        b.iter(|| layer1_filter::filter(black_box(&input)))
    });
}

fn bench_layer1_100kb(c: &mut Criterion) {
    let input = synthetic_input(100_000);
    c.bench_function("layer1_100kb", |b| {
        b.iter(|| layer1_filter::filter(black_box(&input)))
    });
}

fn bench_layer1_fixtures(c: &mut Criterion) {
    let fixtures = [
        "cargo_test_output.txt",
        "tsc_output.txt",
        "vitest_output.txt",
        "docker_logs.txt",
    ];

    let mut group = c.benchmark_group("layer1_fixtures");
    group.measurement_time(Duration::from_secs(5));

    for name in &fixtures {
        if let Ok(content) = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        ) {
            group.bench_with_input(BenchmarkId::new("layer1", name), &content, |b, input| {
                b.iter(|| layer1_filter::filter(black_box(input)))
            });
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Layer 2 tokenizer benchmarks
// ---------------------------------------------------------------------------

fn bench_layer2_tokenizer(c: &mut Criterion) {
    let input = synthetic_input(50_000);
    // Pre-filter with L1 so Layer 2 sees realistic input.
    let l1 = layer1_filter::filter(&input);

    c.bench_function("layer2_tokenizer_50kb", |b| {
        b.iter(|| layer2_tokenizer::process(black_box(&l1.output)))
    });
}

fn bench_layer2_count_tokens(c: &mut Criterion) {
    let input = synthetic_input(10_000);
    c.bench_function("layer2_count_tokens_10kb", |b| {
        b.iter(|| layer2_tokenizer::count_tokens(black_box(&input)))
    });
}

// ---------------------------------------------------------------------------
// Full pipeline benchmark (L1 + L2, no L3 inference)
// ---------------------------------------------------------------------------

fn bench_full_pipeline_no_inference(c: &mut Criterion) {
    let inputs: &[(&str, &str)] = &[
        ("1kb", &synthetic_input(1_024)),
        ("10kb", &synthetic_input(10_000)),
        ("100kb", &synthetic_input(100_000)),
    ];

    let mut group = c.benchmark_group("full_pipeline");
    group.measurement_time(Duration::from_secs(8));

    for (label, input) in inputs {
        group.bench_with_input(BenchmarkId::new("l1+l2", label), input, |b, input| {
            b.iter(|| {
                let l1 = layer1_filter::filter(black_box(input));
                layer2_tokenizer::process(&l1.output)
            })
        });
    }

    group.finish();
}

fn bench_full_pipeline_fixtures(c: &mut Criterion) {
    let fixtures = [
        ("cargo_test", "cargo_test_output.txt"),
        ("tsc", "tsc_output.txt"),
        ("vitest", "vitest_output.txt"),
        ("docker_logs", "docker_logs.txt"),
        ("next_build", "next_build_output.txt"),
    ];

    let mut group = c.benchmark_group("full_pipeline_fixtures");
    group.measurement_time(Duration::from_secs(8));

    for (label, file) in &fixtures {
        if let Ok(content) = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(file),
        ) {
            group.bench_with_input(BenchmarkId::new("l1+l2", label), &content, |b, input| {
                b.iter(|| {
                    let l1 = layer1_filter::filter(black_box(input));
                    layer2_tokenizer::process(&l1.output)
                })
            });
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion groups
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_layer1_1kb,
    bench_layer1_100kb,
    bench_layer1_fixtures,
    bench_layer2_tokenizer,
    bench_layer2_count_tokens,
    bench_full_pipeline_no_inference,
    bench_full_pipeline_fixtures,
);
criterion_main!(benches);
