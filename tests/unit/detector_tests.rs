use ntk::detector::{detect, OutputType};

#[test]
fn test_detects_cargo_test_output() {
    let input = "\
test foo::test_a ... ok
test foo::test_b ... FAILED
test result: FAILED. 1 passed; 1 failed; 0 ignored";
    assert_eq!(detect(input), OutputType::Test);
}

#[test]
fn test_detects_tsc_output() {
    let input = "src/index.ts(10,5): error TS2345: Argument of type 'string' is not assignable.";
    assert_eq!(detect(input), OutputType::Build);
}

#[test]
fn test_detects_vitest_output() {
    let input = "\
 PASS  src/foo.test.ts
 FAIL  src/bar.test.ts

Tests:  1 failed, 5 passed, 6 total";
    assert_eq!(detect(input), OutputType::Test);
}

#[test]
fn test_detects_docker_logs() {
    let input = "\
2024-03-15T10:00:00.000Z INFO server started on port 3000
2024-03-15T10:00:01.123Z WARN deprecated endpoint called
2024-03-15T10:00:02.456Z ERROR connection refused
2024-03-15T10:00:03.789Z INFO request processed";
    assert_eq!(detect(input), OutputType::Log);
}

#[test]
fn test_detects_git_diff() {
    let input = "\
diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
+    println!(\"hello\");
 }";
    assert_eq!(detect(input), OutputType::Diff);
}

#[test]
fn test_unknown_falls_back_to_generic() {
    let input = "some random output that matches no known pattern\nfoo bar baz";
    assert_eq!(detect(input), OutputType::Generic);
}
