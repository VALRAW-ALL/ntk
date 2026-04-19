#!/usr/bin/env node
// Standalone CLI shim around @ctxlint/core — reads stdin, applies every
// rule file under <rules-path> (file OR directory), writes the
// compressed output to stdout. Non-zero exit code if any rule is
// rejected by the built-in preserve_errors invariant.
//
// This is the integration point for RFC-0001 §16 step 4 (#26): any
// agent with a shell-pipe or subprocess hook can consume the same
// YAML rules consumed by the Rust reference, with zero changes to
// either codebase. Usage patterns in docs/spec-second-binding.md.
//
// Usage:
//   echo "$OUTPUT" | ctxlint rules/stack_trace/python.yaml
//   cat trace.txt  | ctxlint rules/stack_trace/          # compose dir
//   ctxlint --help

import { readdirSync, statSync } from 'node:fs';
import { join, extname } from 'node:path';
import { loadRuleFile, applyRuleFile } from '../index.js';

function usage() {
  process.stderr.write(
    [
      'ctxlint — RFC-0001 context-linter reference CLI (JavaScript runtime)',
      '',
      'Usage:',
      '  ctxlint <rules-path>          # read stdin, write compressed stdout',
      '  ctxlint --help',
      '',
      '<rules-path> is a .yaml/.json file OR a directory of rule files',
      '(composed in filename order, same as `ntk test-compress --spec`).',
      '',
      'Exit codes:',
      '  0 — success',
      '  1 — arg error / unreadable rule file',
      '  2 — one or more rules rejected by preserve_errors invariant',
      '',
    ].join('\n'),
  );
}

function resolveRuleFiles(rulesPath) {
  const st = statSync(rulesPath);
  if (!st.isDirectory()) return [rulesPath];
  return readdirSync(rulesPath)
    .filter((n) => {
      const ext = extname(n).toLowerCase();
      return ext === '.yaml' || ext === '.yml' || ext === '.json';
    })
    .sort()
    .map((n) => join(rulesPath, n));
}

async function readStdin() {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  return Buffer.concat(chunks).toString('utf8');
}

async function main() {
  const argv = process.argv.slice(2);
  if (argv.length === 0 || argv.includes('--help') || argv.includes('-h')) {
    usage();
    process.exit(argv.length === 0 ? 1 : 0);
  }
  const rulesPath = argv[0];

  let ruleFiles;
  try {
    ruleFiles = resolveRuleFiles(rulesPath);
  } catch (e) {
    process.stderr.write(`ctxlint: cannot read ${rulesPath}: ${e.message}\n`);
    process.exit(1);
  }
  if (ruleFiles.length === 0) {
    process.stderr.write(`ctxlint: no rule files under ${rulesPath}\n`);
    process.exit(1);
  }

  const input = await readStdin();
  let current = input;
  const rejected = [];
  for (const rf of ruleFiles) {
    const loaded = loadRuleFile(rf);
    const result = applyRuleFile(current, loaded);
    current = result.output;
    if (result.invariantRejected && result.invariantRejected.length) {
      rejected.push(...result.invariantRejected.map((r) => `${rf}:${r}`));
    }
  }

  process.stdout.write(current);
  if (rejected.length) {
    process.stderr.write(
      `ctxlint: ${rejected.length} rule(s) rejected by invariants:\n` +
        rejected.map((r) => `  - ${r}`).join('\n') +
        '\n',
    );
    process.exit(2);
  }
}

main().catch((e) => {
  process.stderr.write(`ctxlint: ${e.stack || e.message}\n`);
  process.exit(1);
});
