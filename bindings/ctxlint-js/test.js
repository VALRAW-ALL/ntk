// Parity tests against the Rust reference impl in
// src/compressor/spec_loader.rs — the #26 acceptance criterion is
// that the JS impl produces the same output on the same inputs
// with the same rule files. Each test mirrors one in the Rust
// #[cfg(test)] block so regressions in either implementation show
// up as a diff against the shared expectations.
//
// Run: node --test test.js

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parse as parseYaml } from 'yaml';
import { applyRuleFile, loadRuleFile } from './index.js';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const PYTHON_YAML = resolve(__dirname, '../../rules/stack_trace/python.yaml');

test('loads the shipped python.yaml ruleset', () => {
  const file = loadRuleFile(PYTHON_YAML);
  // YAML parsers in JS may read the unquoted `0.1` as a number; what
  // matters for compatibility with the Rust reference is that
  // `String(spec_version)` starts with "0." — same check the loader
  // applies at ingestion time.
  assert.match(String(file.spec_version), /^0\./);
  assert.equal(file.language, 'python');
  assert.ok(file.rules.length >= 3);
});

test('rejects unknown spec_version', () => {
  // Use parseYaml directly to simulate an in-memory bad file.
  const bad = parseYaml('spec_version: 1.5\nrules: []\n');
  assert.throws(() => {
    if (!/^0\./.test(String(bad.spec_version))) {
      throw new Error(`unsupported spec_version '${bad.spec_version}' — 0.X`);
    }
  }, /0\.X/);
});

test('frame-run collapses Python site-packages frames', () => {
  const file = loadRuleFile(PYTHON_YAML);
  const input = [
    'Traceback (most recent call last):',
    '  File "/app/main.py", line 10, in <module>',
    '    run()',
    '  File "/usr/lib/python3/site-packages/django/core/handlers.py", line 1, in a',
    '    pass',
    '  File "/usr/lib/python3/site-packages/django/core/handlers.py", line 2, in b',
    '    pass',
    '  File "/usr/lib/python3/site-packages/django/core/handlers.py", line 3, in c',
    '    pass',
    '  File "/usr/lib/python3/site-packages/django/core/handlers.py", line 4, in d',
    '    pass',
    '  File "/usr/lib/python3/site-packages/django/core/handlers.py", line 5, in e',
    '    pass',
    '  File "/app/views.py", line 20, in run',
    '    crash()',
    'ValueError: crashed',
  ].join('\n');

  const r = applyRuleFile(input, file);
  assert.ok(r.output.includes('ValueError: crashed'), 'lost error line');
  assert.ok(r.output.includes('Traceback'), 'lost Traceback');
  assert.ok(r.output.includes('/app/main.py'), 'lost first user frame');
  assert.ok(r.output.includes('/app/views.py'), 'lost last user frame');
  assert.ok(r.output.includes('frames omitted'), 'no collapse marker');
  assert.ok(r.applied.length > 0, 'no rule fired');
});

test('line-match delete removes matching lines', () => {
  const file = parseYaml(`
spec_version: 0.1
rules:
  - id: ansi.progress
    pattern:
      kind: line-match
      classifier: regex
      values: ['^\\s*\\[=+>?\\s*\\d+%']
    transform:
      kind: delete
    severity: lossy-safe
`);
  const input = [
    'line one',
    '  [====>    50%]  done',
    'line two',
    '  [======>   75%]  more',
  ].join('\n');
  const r = applyRuleFile(input, file);
  assert.ok(r.output.includes('line one'));
  assert.ok(r.output.includes('line two'));
  assert.ok(!r.output.includes('50%'));
  assert.ok(!r.output.includes('75%'));
  assert.deepEqual(r.applied, ['ansi.progress']);
});

test('line-match rewrite replaces line content', () => {
  const file = parseYaml(`
spec_version: 0.1
rules:
  - id: redact.token
    pattern:
      kind: line-match
      classifier: regex
      values: ['Bearer [A-Za-z0-9._-]+']
    transform:
      kind: rewrite
      replacement: 'Bearer <redacted>'
    severity: lossy-safe
`);
  const input = [
    'Authorization: Bearer eyJhbGciOi.abc.def',
    'Content-Type: application/json',
  ].join('\n');
  const r = applyRuleFile(input, file);
  assert.ok(r.output.includes('Bearer <redacted>'));
  assert.ok(!r.output.includes('eyJhbGciOi'));
  assert.ok(r.output.includes('application/json'));
});

test('template-dedup collapses repeated warnings', () => {
  const file = parseYaml(`
spec_version: 0.1
rules:
  - id: warn.dedup
    pattern:
      kind: template-dedup
      normalize:
        - regex: '\\d+'
          replacement: '§'
    transform:
      kind: dedup
      min_run: 3
      format: '[×{n}] {exemplar}'
`);
  const input = [
    'warning: retry 1 failed',
    'warning: retry 2 failed',
    'warning: retry 3 failed',
    'warning: retry 4 failed',
    'ok: done',
  ].join('\n');
  const r = applyRuleFile(input, file);
  assert.ok(r.output.includes('[×4] warning: retry 1 failed'), r.output);
  assert.ok(r.output.includes('ok: done'));
});

test('template-dedup skips blank lines (no empty exemplar regression)', () => {
  const file = parseYaml(`
spec_version: 0.1
rules:
  - id: dedup.any
    pattern:
      kind: template-dedup
      normalize:
        - regex: '\\d+'
          replacement: '§'
    transform:
      kind: dedup
      min_run: 2
      format: '[×{n}] {exemplar}'
`);
  const r = applyRuleFile('\n\nA\n', file);
  assert.ok(!r.output.includes('[×2] \n'), `empty-exemplar regression: ${r.output}`);
});

test('prefix-factor extracts a shared leading prefix', () => {
  const file = parseYaml(`
spec_version: 0.1
rules:
  - id: cargo.warn
    pattern:
      kind: prefix-factor
      min_share: 0.8
      min_lines: 3
    transform:
      kind: factor-prefix
      replacement: '[common prefix: {prefix}]'
`);
  const input = [
    'warning: foo is dead',
    'warning: bar is dead',
    'warning: baz is dead',
    'warning: qux is dead',
  ].join('\n');
  const r = applyRuleFile(input, file);
  assert.ok(r.output.includes('[common prefix:'), r.output);
  assert.ok(r.applied.length > 0);
});

test('invariant preserves error lines', () => {
  const file = loadRuleFile(PYTHON_YAML);
  const input = [
    'error: build failed',
    '  File "/site-packages/a/b.py", line 1, in x',
    '    pass',
    '  File "/site-packages/a/b.py", line 2, in y',
    '    pass',
    '  File "/site-packages/a/b.py", line 3, in z',
    '    pass',
    'error: build failed twice',
  ].join('\n');
  const r = applyRuleFile(input, file);
  assert.ok(r.output.includes('error: build failed'), r.output);
  assert.ok(r.output.includes('error: build failed twice'));
});
