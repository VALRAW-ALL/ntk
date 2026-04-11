use ntk::compressor::layer1_filter::filter;

#[test]
fn test_remove_ansi_codes() {
    let input = "\x1b[32mhello\x1b[0m world";
    let result = filter(input);
    assert_eq!(result.output, "hello world");
    assert!(!result.rtk_pre_filtered);
}

#[test]
fn test_group_repeated_lines() {
    let input = "cargo:warning=foo\ncargo:warning=foo\ncargo:warning=foo\nother";
    let result = filter(input);
    assert!(result.output.contains("[×3] cargo:warning=foo"));
    assert!(result.output.contains("other"));
    assert_eq!(result.lines_removed, 2);
}

#[test]
fn test_keep_only_failures_cargo_test() {
    let input = "\
test module::test_a ... ok
test module::test_b ... FAILED
test module::test_c ... ok
test result: FAILED. 2 passed; 1 failed";

    let result = filter(input);
    assert!(!result.output.contains("test_a"));
    assert!(!result.output.contains("test_c"));
    assert!(result.output.contains("test_b"));
    assert!(result.output.contains("test result:"));
}

#[test]
fn test_remove_progress_bars() {
    let input = "Downloading crate foo\nnormal line\n[==== 70%]\nanother line";
    let result = filter(input);
    assert!(!result.output.contains("[===="));
    assert!(result.output.contains("normal line"));
    assert!(result.output.contains("another line"));
}

#[test]
fn test_collapse_blank_lines() {
    let input = "line1\n\n\n\nline2\n\nline3";
    let result = filter(input);
    assert!(!result.output.contains("\n\n\n"));
    assert!(result.output.contains("line1"));
    assert!(result.output.contains("line2"));
    assert!(result.output.contains("line3"));
}

#[test]
fn test_detect_rtk_filtered_output() {
    let input = "[×47] cargo:warning=unused import\n[×12] note: ...";
    let result = filter(input);
    assert!(result.rtk_pre_filtered);
}
