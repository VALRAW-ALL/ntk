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

// --- Template dedup (v0.2.27+) -------------------------------------------

#[test]
fn test_template_dedup_timestamps() {
    let input = "\
2024-09-12T08:00:01.151Z INFO  request handled in 12ms
2024-09-12T08:00:01.203Z INFO  request handled in 14ms
2024-09-12T08:00:01.317Z INFO  request handled in 11ms
2024-09-12T08:00:01.401Z INFO  request handled in 19ms";
    let result = filter(input);
    assert!(
        result.output.contains("[×4]"),
        "expected [×4] grouping, got:\n{}",
        result.output
    );
    // Exemplar must be a real first line (invariant #2).
    assert!(
        result.output.contains("2024-09-12T08:00:01.151Z"),
        "first line must be preserved as exemplar: {}",
        result.output
    );
}

#[test]
fn test_template_dedup_uuids_and_ints() {
    let input = "\
user 7a2f9e3d-11c8-4a6e-8b0a-1234567890ab completed step 42
user 9b1c4e2a-33d7-4f9e-a1b2-abcdef012345 completed step 99
user 4d8e2f1c-55aa-4bbc-90de-fedcba987654 completed step 128";
    let result = filter(input);
    assert!(
        result.output.contains("[×3]"),
        "3 templated lines should collapse: {}",
        result.output
    );
}

#[test]
fn test_template_dedup_short_hex_is_not_touched() {
    // Invariant: git short SHAs (7-hex) must NOT be normalized.
    let input = "\
commit abc1234 fix bug
commit def5678 fix other bug";
    let result = filter(input);
    // Both lines must survive without collapsing (they are legitimately distinct).
    assert!(result.output.contains("abc1234"));
    assert!(result.output.contains("def5678"));
}

// --- Stack-trace filter (multi-language) ---------------------------------

#[test]
fn test_stack_trace_filter_java() {
    let input = "\
Exception in thread \"main\" java.lang.NullPointerException: oops
    at com.example.MyService.doWork(MyService.java:42)
    at org.springframework.web.servlet.DispatcherServlet.doDispatch(DispatcherServlet.java:1071)
    at org.springframework.web.servlet.FrameworkServlet.service(FrameworkServlet.java:883)
    at org.springframework.web.servlet.FrameworkServlet.doGet(FrameworkServlet.java:778)
    at org.apache.catalina.core.ApplicationFilterChain.doFilter(ApplicationFilterChain.java:166)
    at org.apache.catalina.core.StandardWrapperValve.invoke(StandardWrapperValve.java:223)
    at org.apache.catalina.core.StandardContextValve.invoke(StandardContextValve.java:158)
    at com.example.MyController.handle(MyController.java:18)";
    let result = filter(input);
    // User frames (com.example.*) must survive (invariant #3).
    assert!(result.output.contains("MyService.doWork"));
    assert!(result.output.contains("MyController.handle"));
    // NullPointerException must always survive (invariant #1).
    assert!(result.output.contains("NullPointerException"));
}

#[test]
fn test_stack_trace_filter_python_django() {
    let input = "\
Traceback (most recent call last):
  File \"/app/views.py\", line 42, in index
    return render(request, 'index.html')
  File \"/usr/lib/python3.11/site-packages/django/shortcuts.py\", line 24, in render
    content = loader.render_to_string(template_name, context, request)
  File \"/usr/lib/python3.11/site-packages/django/template/loader.py\", line 62, in render_to_string
    return template.render(context, request)
  File \"/usr/lib/python3.11/site-packages/django/template/backends/django.py\", line 61, in render
    return self.template.render(context)
AttributeError: 'NoneType' object has no attribute 'user'";
    let result = filter(input);
    assert!(result.output.contains("Traceback"));
    assert!(result.output.contains("AttributeError"));
    assert!(result.output.contains("views.py"));
}

// --- Whitespace handling -------------------------------------------------

#[test]
fn test_whitespace_preserves_4col_leading_indent() {
    // Stack frames must keep their leading indent so structure is readable.
    let input = "error: bad\n    at foo.js:10\n    at bar.js:20";
    let result = filter(input);
    assert!(
        result.output.contains("    at foo.js"),
        "4-col leading indent must survive: {}",
        result.output
    );
}

// --- Invariants ----------------------------------------------------------

#[test]
fn test_idempotent_filter() {
    // Invariant #5: filter(filter(x)) == filter(x).
    let input = "2024-01-01T00:00:00Z INFO hit 1ms\n2024-01-01T00:00:01Z INFO hit 2ms\n\
                 ERROR: fatal\n";
    let once = filter(input).output;
    let twice = filter(&once).output;
    assert_eq!(once, twice, "filter must be idempotent");
}

#[test]
fn test_error_lines_never_dropped() {
    // Invariant #1: error/warning signals survive.
    let input = "\
info: started
info: started
info: started
info: started
ERROR: something blew up
warning: deprecated api
panic: assertion failed";
    let result = filter(input);
    assert!(result.output.contains("ERROR: something blew up"));
    assert!(result.output.contains("warning: deprecated api"));
    assert!(result.output.contains("panic: assertion failed"));
}
