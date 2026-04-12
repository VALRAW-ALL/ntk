# Generates a payload that exercises Layer 1 (fast dedup + line grouping).
# Pattern: repeated cargo warning blocks -- L1 deduplicates identical messages.

$out = [System.Text.StringBuilder]::new()

# Block 1: repeated "unused variable" warnings (60 occurrences)
for ($i = 0; $i -lt 60; $i++) {
    $ln  = Get-Random -Min 10 -Max 200
    $col = Get-Random -Min 1  -Max 40
    [void]$out.AppendLine("warning: unused variable")
    [void]$out.AppendLine("   --> src/compressor/layer1_filter.rs:${ln}:${col}")
    [void]$out.AppendLine("    note: [warn(unused_variables)] on by default")
    [void]$out.AppendLine("    help: if intentional, prefix with an underscore")
    [void]$out.AppendLine("")
}

# Block 2: repeated "dead code" warnings (40 occurrences)
for ($i = 0; $i -lt 40; $i++) {
    $ln = Get-Random -Min 1 -Max 120
    [void]$out.AppendLine("warning: function is never used: process_chunk")
    [void]$out.AppendLine("   --> src/compressor/layer2_tokenizer.rs:${ln}:1")
    [void]$out.AppendLine("    note: [warn(dead_code)] on by default")
    [void]$out.AppendLine("")
}

# Block 3: repeated "unused import" warnings (30 occurrences)
for ($i = 0; $i -lt 30; $i++) {
    $ln = Get-Random -Min 1 -Max 50
    [void]$out.AppendLine("warning: unused import: std::collections::HashMap")
    [void]$out.AppendLine("   --> src/metrics.rs:${ln}:5")
    [void]$out.AppendLine("    help: remove unused imports")
    [void]$out.AppendLine("")
}

[void]$out.AppendLine("warning: 130 warnings emitted")
[void]$out.AppendLine("")

$out.ToString()
