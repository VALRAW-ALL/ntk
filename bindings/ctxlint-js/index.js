// Reference JavaScript implementation of the RFC-0001 context-linter
// spec, matching the Rust impl at src/compressor/spec_loader.rs.
//
// This is the second-runtime binding (#26). The gate from RFC-0001
// §16 step 4 is that this implementation produces *byte-identical*
// output to the Rust reference on the same fixture + same rule file.
// When the two drift, the rule file format is the root cause, not
// the implementation — that's the point of having two bindings.
//
// Intentionally dependency-light (one external package: `yaml` for
// parsing) so it runs anywhere Node ≥ 18 runs, including inside a
// Continue plugin sandbox.

import { parse as parseYaml } from 'yaml';
import { readFileSync } from 'node:fs';

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/**
 * Parse a rule file from disk. Accepts both YAML (default) and JSON —
 * extension-based detection, no content sniffing in the POC.
 *
 * @param {string} path absolute path to the rule file
 * @returns {object} the parsed RuleFile
 */
export function loadRuleFile(path) {
  const text = readFileSync(path, 'utf8');
  const isJson = /\.json$/i.test(path);
  const file = isJson ? JSON.parse(text) : parseYaml(text);
  if (!/^0\./.test(String(file.spec_version))) {
    throw new Error(
      `unsupported spec_version '${file.spec_version}' — this loader handles 0.X`,
    );
  }
  return file;
}

// ---------------------------------------------------------------------------
// Apply rule file — same dispatch shape as the Rust impl
// ---------------------------------------------------------------------------

/**
 * Apply every rule in the file to `input` in order. Later rules see
 * earlier rules' output. Returns { output, applied, invariantRejected }.
 */
export function applyRuleFile(input, file) {
  let current = input;
  const applied = [];
  const rejected = [];

  for (const rule of file.rules || []) {
    let pair;
    try {
      pair = applyOne(current, rule.pattern, rule.transform);
    } catch (e) {
      rejected.push(rule.id);
      continue;
    }
    const [newText, fired] = pair;
    if (!fired) continue;

    // Invariant: preserve_errors — reject a transform that dropped
    // error/panic/Traceback markers even if the rule claims to
    // respect the invariant.
    if (
      (rule.invariants || []).includes('preserve_errors') &&
      !preservesErrorSignal(current, newText)
    ) {
      rejected.push(rule.id);
      continue;
    }

    current = newText;
    applied.push(rule.id);
  }

  return { output: current, applied, invariantRejected: rejected };
}

// ---------------------------------------------------------------------------
// Dispatcher — one (pattern, transform) pair → one primitive
// ---------------------------------------------------------------------------

function applyOne(input, pattern, transform) {
  const p = pattern || {};
  const t = transform || {};

  if (p.kind === 'frame-run' && t.kind === 'collapse-run') {
    return applyFrameRun(
      input,
      p.classifier,
      p.values || [],
      p.unit || 1,
      t.min_run || 3,
      t.replacement || '[{n} frames omitted]',
    );
  }
  if (p.kind === 'line-match' && t.kind === 'delete') {
    return applyLineMatchDelete(input, p.classifier, p.values || []);
  }
  if (p.kind === 'line-match' && t.kind === 'rewrite') {
    return applyLineMatchRewrite(
      input,
      p.classifier,
      p.values || [],
      t.replacement || '',
    );
  }
  if (p.kind === 'template-dedup' && t.kind === 'dedup') {
    return applyTemplateDedup(
      input,
      p.normalize || [],
      t.min_run || 2,
      t.format || '[×{n}] {exemplar}',
    );
  }
  if (p.kind === 'prefix-factor' && t.kind === 'factor-prefix') {
    return applyPrefixFactor(
      input,
      p.min_share ?? 0.8,
      p.min_lines || 4,
      t.replacement || '[prefix: {prefix}]',
    );
  }
  throw new Error(
    `unsupported (pattern=${p.kind}, transform=${t.kind}) combination`,
  );
}

// ---------------------------------------------------------------------------
// frame-run / collapse-run primitive
// ---------------------------------------------------------------------------

function applyFrameRun(input, classifier, values, unit, minRun, replacementTpl) {
  unit = Math.max(1, unit | 0);
  minRun = Math.max(2, minRun | 0);

  const lines = input.split(/\r?\n/);
  const out = [];
  let i = 0;
  let anyFired = false;

  while (i < lines.length) {
    if (!classifiesFrame(lines, i, unit, classifier, values)) {
      out.push(lines[i]);
      i += 1;
      continue;
    }

    let count = 0;
    while (classifiesFrame(lines, i + count * unit, unit, classifier, values)) {
      count += 1;
    }

    if (count >= minRun) {
      // Preserve first + last frames (invariant #3).
      for (let k = 0; k < unit && i + k < lines.length; k += 1) {
        out.push(lines[i + k]);
      }
      const omitted = Math.max(0, count - 2);
      out.push(replacementTpl.replaceAll('{n}', String(omitted)));
      const lastStart = i + (count - 1) * unit;
      for (let k = 0; k < unit && lastStart + k < lines.length; k += 1) {
        out.push(lines[lastStart + k]);
      }
      i += count * unit;
      anyFired = true;
    } else {
      const end = Math.min(lines.length, i + count * unit);
      for (let k = i; k < end; k += 1) out.push(lines[k]);
      i = end;
    }
  }

  return [out.join('\n'), anyFired];
}

function classifiesFrame(lines, idx, unit, classifier, values) {
  if (idx >= lines.length) return false;
  if (idx + unit - 1 >= lines.length) return false;
  return lineMatches(lines[idx], classifier, values);
}

function lineMatches(line, classifier, values) {
  switch (classifier) {
    case 'contains':
      return values.some((v) => line.includes(v));
    case 'starts_with':
      return values.some((v) => line.trimStart().startsWith(v));
    case 'equals':
      return values.some((v) => line === v);
    case 'regex':
      return values.some((v) => {
        try {
          return new RegExp(v).test(line);
        } catch {
          return false;
        }
      });
    default:
      return false;
  }
}

// ---------------------------------------------------------------------------
// line-match primitive
// ---------------------------------------------------------------------------

function applyLineMatchDelete(input, classifier, values) {
  let fired = false;
  const lines = input.split(/\r?\n/);
  const out = [];
  for (const line of lines) {
    if (lineMatches(line, classifier, values)) {
      fired = true;
    } else {
      out.push(line);
    }
  }
  return [out.join('\n'), fired];
}

function applyLineMatchRewrite(input, classifier, values, replacement) {
  let fired = false;
  const compiled =
    classifier === 'regex'
      ? values.map((v) => {
          try {
            return new RegExp(v, 'g');
          } catch {
            return null;
          }
        })
      : [];
  const lines = input.split(/\r?\n/);
  const out = [];
  for (const line of lines) {
    if (!lineMatches(line, classifier, values)) {
      out.push(line);
      continue;
    }
    fired = true;
    if (classifier === 'regex') {
      let rewritten = line;
      for (const re of compiled) {
        if (re) rewritten = rewritten.replace(re, replacement);
      }
      out.push(rewritten);
    } else {
      out.push(replacement);
    }
  }
  return [out.join('\n'), fired];
}

// ---------------------------------------------------------------------------
// template-dedup primitive
// ---------------------------------------------------------------------------

function applyTemplateDedup(input, normalize, minRun, format) {
  minRun = Math.max(2, minRun | 0);
  const compiled = (normalize || []).map((n) => ({
    re: new RegExp(n.regex, 'g'),
    rep: n.replacement ?? '§',
  }));
  const norm = (line) => {
    let s = line;
    for (const { re, rep } of compiled) s = s.replace(re, rep);
    return s;
  };

  const lines = input.split(/\r?\n/);
  const out = [];
  let i = 0;
  let fired = false;

  while (i < lines.length) {
    // Skip blanks — never emit '[×N] ' with empty exemplar.
    if (lines[i].trim() === '') {
      out.push(lines[i]);
      i += 1;
      continue;
    }
    const template = norm(lines[i]);
    let count = 1;
    while (i + count < lines.length) {
      const next = lines[i + count];
      if (next.trim() === '') break;
      if (norm(next) !== template) break;
      count += 1;
    }
    if (count >= minRun) {
      out.push(
        format.replaceAll('{n}', String(count)).replaceAll('{exemplar}', lines[i]),
      );
      fired = true;
      i += count;
    } else {
      out.push(lines[i]);
      i += 1;
    }
  }

  return [out.join('\n'), fired];
}

// ---------------------------------------------------------------------------
// prefix-factor primitive
// ---------------------------------------------------------------------------

function applyPrefixFactor(input, minShare, minLines, replacement) {
  const lines = input.split(/\r?\n/);
  if (lines.length < minLines) return [input, false];

  let prefixLen = [...lines[0]].length;
  for (let i = 1; i < lines.length; i += 1) {
    const a = [...lines[0]];
    const b = [...lines[i]];
    let common = 0;
    while (common < a.length && common < b.length && a[common] === b[common]) {
      common += 1;
    }
    prefixLen = Math.min(prefixLen, common);
    if (prefixLen === 0) break;
  }

  if (prefixLen < 2) return [input, false];

  // All lines share the full prefix by construction in v1, so share=1.0.
  if (1.0 < minShare) return [input, false];

  const prefix = [...lines[0]].slice(0, prefixLen).join('');
  const out = [replacement.replaceAll('{prefix}', prefix)];
  for (const line of lines) {
    const stripped = [...line].slice(prefixLen).join('');
    out.push(`  ${stripped}`);
  }
  return [out.join('\n'), true];
}

// ---------------------------------------------------------------------------
// Invariant check: error signal preserved
// ---------------------------------------------------------------------------

// Parity with src/compressor/spec_loader.rs::RE_ERROR_SIGNAL. The naive
// /error/i pattern false-matched path components like
// `/django/core/handlers/exception.py`, causing every framework-collapse
// rule on a Python fixture to be rejected. Each alternative anchors on
// either an uppercase class-prefix or a `:<whitespace>` contract.
const ERROR_RE = new RegExp(
  [
    // Typed class: ValueError:, RuntimeException: — uppercase start,
    // trailing colon + whitespace/EOL. Excludes lowercase path components.
    '[A-Z][A-Za-z0-9_]*(?:Error|Exception):(?:\\s|$)',
    // Bare Error: / Exception: at line-start or after whitespace.
    '(?:^|\\s)(?:Error|Exception):(?:\\s|$)',
    // Uppercase error tokens as whole words.
    '\\b(?:ERROR|FAILED|CRITICAL|PANIC)\\b',
    // Lowercase-colon forms: error: / panic: / fatal: / warning:.
    '(?:^|\\s)(?:error|panic|fatal|warning):\\s',
    // Python prefix that always opens a trace.
    '\\bTraceback\\b',
    // Rust compiler error codes (E0001 through E9999).
    '\\bE0\\d{3}:',
    // Java 'Caused by:' chain separator.
    '\\bCaused by:',
  ].join('|'),
  'gm',
);

function preservesErrorSignal(before, after) {
  const count = (s) => {
    // Reset lastIndex since ERROR_RE has the /g flag (stateful).
    ERROR_RE.lastIndex = 0;
    return (s.match(ERROR_RE) || []).length;
  };
  return count(after) >= count(before);
}
