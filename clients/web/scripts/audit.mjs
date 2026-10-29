#!/usr/bin/env node
// Filtered npm-audit gate — CI dependency check (MPE-123).
//
// npm has no native audit-ignore mechanism (cargo has .cargo/audit.toml), so
// this script applies the checked-in allowlist in ../npm-audit-exceptions.json:
// each exception suppresses one advisory by GHSA id, including findings that
// only propagate it (e.g. @gorules/jdm-editor is flagged solely because its
// zod dep is flagged).
//
// Gate: production deps (--omit=dev), fails on un-excepted findings at
// severity >= GATE_SEVERITIES. Dev-only findings are reported, not gated —
// they are still recorded in the exceptions file for visibility.

import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const GATE_SEVERITIES = new Set(['high', 'critical']);
const GHSA_RE = /GHSA-[a-z0-9]{4}-[a-z0-9]{4}-[a-z0-9]{4}/i;

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const exceptions = JSON.parse(
  readFileSync(join(root, 'npm-audit-exceptions.json'), 'utf8'),
).exceptions;
const exceptedIds = new Map(exceptions.map((e) => [e.id.toUpperCase(), e]));

function auditJson(omitDev) {
  const args = ['audit', '--json'];
  if (omitDev) args.push('--omit=dev');
  // Prefer the npm-cli.js that invoked us (npm_execpath is set under `npm run`)
  // so we spawn plain `node` — avoids shell:true and .cmd quirks on Windows.
  const [cmd, cmdArgs] = process.env.npm_execpath
    ? [process.execPath, [process.env.npm_execpath, ...args]]
    : [process.platform === 'win32' ? 'npm.cmd' : 'npm', args];
  const res = spawnSync(cmd, cmdArgs, {
    cwd: root,
    encoding: 'utf8',
    shell: !process.env.npm_execpath && process.platform === 'win32',
  });
  if (res.error) throw res.error;
  const out = res.stdout || '';
  try {
    return JSON.parse(out);
  } catch {
    console.error('npm audit did not return JSON:\n' + out + res.stderr);
    process.exit(2);
  }
}

// A vulnerability entry is excepted iff every advisory object in `via` is
// allowlisted AND every string `via` names a vulnerability that is itself
// excepted (memoized DFS; advisory objects carry the GHSA url, strings name
// other entries in the vulnerabilities map).
function buildResolver(vulns) {
  const memo = new Map();
  return function isExcepted(name) {
    if (memo.has(name)) return memo.get(name);
    const v = vulns[name];
    if (!v) return false;
    memo.set(name, true); // cycle guard — chains are DAGs in practice
    const result = (v.via || []).every((item) => {
      if (typeof item === 'string') return isExcepted(item);
      const m = (item.url || '').match(GHSA_RE);
      return m != null && exceptedIds.has(m[0].toUpperCase());
    });
    memo.set(name, result);
    return result;
  };
}

function idsOf(v) {
  return (v.via || [])
    .map((item) => {
      if (typeof item !== 'string') {
        const m = (item.url || '').match(GHSA_RE);
        return m ? m[0] : null;
      }
      return null;
    })
    .filter(Boolean);
}

const prod = auditJson(true).vulnerabilities || {};
const full = auditJson(false).vulnerabilities || {};
const isExcepted = buildResolver(prod);

console.log('== npm audit gate (production dependencies) ==\n');
let failures = 0;
let warnings = 0;
for (const [name, v] of Object.entries(prod)) {
  const ids = idsOf(v).join(', ') || '(propagated)';
  if (isExcepted(name)) {
    console.log(`EXCEPTED   ${v.severity.padEnd(8)} ${name}  ${ids}`);
  } else if (GATE_SEVERITIES.has(v.severity)) {
    failures++;
    console.log(`FAIL       ${v.severity.padEnd(8)} ${name}  ${ids}`);
  } else {
    warnings++;
    console.log(`WARN       ${v.severity.padEnd(8)} ${name}  ${ids}`);
  }
}

const devOnly = Object.entries(full).filter(([name]) => !(name in prod));
if (devOnly.length) {
  console.log('\n== dev-only findings (report-only; recorded in exceptions file) ==\n');
  for (const [name, v] of devOnly) {
    console.log(`dev        ${v.severity.padEnd(8)} ${name}  ${idsOf(v).join(', ')}`);
  }
}

console.log('');
if (failures) {
  console.log(
    `GATE: FAIL — ${failures} un-excepted high/critical finding(s). ` +
      'Fix the dependency or add a justified entry to npm-audit-exceptions.json.',
  );
  process.exit(1);
}
console.log(
  `GATE: PASS — no un-excepted high/critical findings` +
    (warnings ? ` (${warnings} moderate/low warning(s) above)` : '') +
    `; ${devOnly.length} dev-only finding(s) reported.`,
);
