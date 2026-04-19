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
use ntk::compressor::{layer1_filter, layer2_tokenizer, spec_loader};
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

// ---------------------------------------------------------------------------
// POC #24 — hardcoded vs YAML-spec overhead comparison
//
// Answers the kill-criterion from RFC-0001 §16 step 2: a YAML ruleset
// must stay within +20% of the hardcoded path for the abstraction to
// be worth shipping. Benchmarks the same Python-trace fixture through
// both engines on identical input.
// ---------------------------------------------------------------------------

fn bench_spec_vs_hardcoded_python(c: &mut Criterion) {
    let input = fixture_bench("python_django_trace.txt");
    let rule_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("rules")
        .join("stack_trace")
        .join("python.yaml");
    let rule_file = spec_loader::load_rule_file(&rule_path).expect("load python.yaml");

    let mut group = c.benchmark_group("poc_spec_vs_hardcoded");
    group.bench_function("hardcoded_l1_python", |b| {
        b.iter(|| layer1_filter::filter(black_box(&input)))
    });
    group.bench_function("spec_loader_python", |b| {
        b.iter(|| spec_loader::apply_rule_file(black_box(&input), black_box(&rule_file)))
    });
    group.finish();
}

fn fixture_bench(name: &str) -> String {
    // The spec-vs-hardcoded bench reuses bench/fixtures/ (richer Python
    // traces live there) rather than tests/fixtures/. Fall back cleanly
    // when the bench fixture is absent so the benchmark still runs.
    let bench_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("bench")
        .join("fixtures")
        .join(name);
    if bench_path.exists() {
        return std::fs::read_to_string(&bench_path).unwrap_or_default();
    }
    fixture(name)
}

criterion_group!(
    benches,
    bench_layer1_1kb,
    bench_layer1_100kb,
    bench_layer1_fixtures,
    bench_layer2_tokenizer,
    bench_layer2_count_tokens,
    bench_full_pipeline_no_inference,
    bench_full_pipeline_fixtures,
    bench_spec_vs_hardcoded_python,
);
criterion_main!(benches);
