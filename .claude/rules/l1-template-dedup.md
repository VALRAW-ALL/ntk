# Rule: L1 Template Dedup — blank-line handling

Applies to: `group_by_template()` and any future line-grouping stage in
`src/compressor/layer1_filter.rs`.

## The bug this rule prevents

A proptest on input `"\n\nA\n"` caught this regression:

```
input : \n \n A
output: [×2]      ← BAD — marker with no exemplar body
        A
```

The two blank lines shared the same "template" (empty), so template
dedup emitted `[×2] ` with an empty exemplar. That violates invariant #2
from `l1-l2-invariants.md`: **every `[×N]` marker must carry a readable
exemplar line**. An empty marker is worse than useless — it wastes tokens
AND misleads the LLM about what was grouped.

## The rule

`group_by_template()` must **skip** blank / whitespace-only lines:

```rust
if current.trim().is_empty() {
    out.push(current.to_owned());
    idx = idx.saturating_add(1);
    continue;
}
```

And the forward-scan loop must **stop** at a blank line rather than
absorb it into a template group:

```rust
while idx.saturating_add(count) < lines.len() {
    let next = lines[idx.saturating_add(count)];
    if next.trim().is_empty() {
        break;
    }
    ...
}
```

Blank-line collapsing is the job of the dedicated `collapse_blank_lines`
stage later in the pipeline, not of template dedup.

## How to catch a regression

Run the proptest suite before committing any change to
`group_by_template` or `normalize_to_template`:

```bash
cargo test --test compression_invariants prop_dedup_keeps_real_exemplar
```

This property generates thousands of inputs including whitespace-heavy
ones and verifies the invariant holds.

## Related rules

- `l1-l2-invariants.md` — the five non-negotiable invariants
- `stack-trace-classifier.md` — analogous rule for the stack-trace stage
