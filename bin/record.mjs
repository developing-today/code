#!/usr/bin/env node
// record — append-only session-recording ingestion (#1853).
//
// Replaces the hand-append to ~/git/ai_recordings.jsonl that
// dezren39/nix instructions/recordings.md used to prescribe. An agent
// that has just explained something worth keeping to the user sends the
// same observation here instead:
//
//   record '<json-or-loose-text>'
//   echo '<json-or-loose-text>' | record
//
// The contract is NEVER REJECT (the issue's one hard rule after
// append-only): a malformed line costs one normalisation pass later, a
// rejected line costs the observation permanently. So:
//
// - Input that parses as a JSON object is appended as one line, with
//   stamps (ts, pwd, repo, origin, head, session, model) filled in
//   only for keys the agent did not supply. Agent-supplied fields are
//   never rewritten, flattened, or validated — `sentiment` is `any`,
//   unknown keys pass through untouched.
// - Input that parses as JSON but is not an object (a bare string, a
//   list) is wrapped as { note: <value> } and appended the same way.
// - Loosely structured text (`category=bad | finding: ... | ...`) is
//   normalised into an object when at least two keyed segments parse;
//   the verbatim text rides along in `raw`.
// - Anything else is quarantined VERBATIM to a sibling file
//   (<target>.quarantine) for a later normalisation pass.
// - An unwritable target (or quarantine) is a silent no-op.
// - Every path exits 0: recording must never fail a task.
//
// Appends are a single O_APPEND write of the whole line, so concurrent
// agent sessions cannot interleave partial lines. The target file (and
// its parent directory) is created on first append. Target override:
// AI_RECORDINGS_FILE (default ~/git/ai_recordings.jsonl).
//
// Define semantics for externally-spelled keys (#2953). A value keyed by a
// name a caller, document, feed, file, peer or user can spell is stored with
// define semantics, never assignment semantics: the store is built in a Map
// or a null-prototype object and converted at the boundary with
// Object.fromEntries / spread, or the entry is written with
// Object.defineProperty. The non-conforming forms for external keys are
// obj[key] = v, obj[key] ??= ..., dst[k] = src[k] copy loops, and
// Object.assign(dst, src) with an externally-keyed src — all go through
// [[Set]], and for the one spelling "__proto__" [[Set]] invokes the
// Object.prototype setter instead of creating an own property, silently
// dropping the entry or moving the container's prototype. Keys are data:
// preserve, don't reject. Code-constructed tables whose keys are all
// internal are exempt. Probe matrix for every covered site — keys
// "__proto__" (string value), "__proto__" (object value), "constructor",
// "toString", and an ordinary key: the entry exists as an own property with
// its value, a JSON round-trip preserves it, and neither the container's
// prototype nor Object.prototype moves or gains own properties.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const DEFAULT_TARGET = path.join(
  os.homedir(),
  'git',
  'ai_recordings.jsonl'
);

export const resolveTarget = (env = process.env) => {
  const raw = env.AI_RECORDINGS_FILE;
  return typeof raw === 'string' && raw.trim() !== '' ? raw : DEFAULT_TARGET;
};

export const quarantinePathFor = (target) => `${target}.quarantine`;

// Best-effort git discovery: every failure just omits the stamp.
const git = (cwd, args) => {
  try {
    const result = spawnSync('git', args, {
      cwd,
      encoding: 'utf8',
      timeout: 2000
    });
    if (result.status !== 0) return undefined;
    const value = result.stdout.trim();
    return value === '' ? undefined : value;
  } catch {
    return undefined;
  }
};

export const discoverStamps = ({ cwd, env, now }) => {
  const stamps = { ts: now.toISOString(), pwd: cwd };
  const commonDir = git(cwd, ['rev-parse', '--git-common-dir']);
  if (commonDir !== undefined) {
    const absolute = path.resolve(cwd, commonDir);
    stamps.git_common_dir = absolute;
    // The repo name is the directory holding the common dir's parent
    // (worktrees share one common dir under the primary checkout).
    stamps.repo = path.basename(path.dirname(absolute));
  }
  const origin = git(cwd, ['remote', 'get-url', 'origin']);
  if (origin !== undefined) stamps.origin = origin;
  const head = git(cwd, ['rev-parse', 'HEAD']);
  if (head !== undefined) stamps.head = head;
  const session = env.OPENCODE_SESSION_ID || env.SESSION_ID;
  if (session) stamps.session = session;
  const model = env.OPENCODE_MODEL || env.MODEL;
  if (model) stamps.model = model;
  return stamps;
};

// Loose structured text: `key=value` / `key: value` segments split on
// '|'. Returns an object only when at least two keyed segments parse —
// below that the text is not confidently structured and the caller
// quarantines it verbatim instead of guessing.
export const parseLoose = (raw) => {
  const fields = {};
  const unkeyed = [];
  let keyed = 0;
  for (const segment of raw.split('|')) {
    const match = segment.match(/^\s*([A-Za-z_][\w .-]*?)\s*[:=]\s*(.+)$/s);
    if (match) {
      const key = match[1].trim().replace(/\s+/g, '_');
      let value = match[2].trim();
      try {
        value = JSON.parse(value);
      } catch {
        // Keep the verbatim string; normalisation happens later.
      }
      // Define semantics, not assignment (#2828): assigning a
      // '__proto__' key would invoke the prototype setter — a string
      // value is silently dropped and an object value would replace
      // this object's prototype — while the JSON input path preserves
      // the field. Defining makes every accepted key an own data
      // property, so no supplied field is ever lost.
      Object.defineProperty(fields, key, {
        value,
        enumerable: true,
        writable: true,
        configurable: true
      });
      keyed += 1;
    } else if (segment.trim() !== '') {
      unkeyed.push(segment.trim());
    }
  }
  if (keyed < 2) return null;
  if (unkeyed.length > 0 && fields.note === undefined) {
    fields.note = unkeyed.join(' | ');
  }
  return fields;
};

// Returns { entry } to append, { quarantine: raw } to quarantine, or
// {} for empty input (nothing worth recording, still not an error).
export const normaliseInput = (raw, stamps) => {
  if (raw.trim() === '') return {};
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch {
    const loose = parseLoose(raw);
    if (loose === null) return { quarantine: raw };
    return { entry: { ...loose, raw, ...missingStamps(loose, stamps) } };
  }
  if (parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed)) {
    return { entry: { ...parsed, ...missingStamps(parsed, stamps) } };
  }
  const wrapped = { note: parsed };
  return { entry: { ...wrapped, ...missingStamps(wrapped, stamps) } };
};

const missingStamps = (entry, stamps) => {
  const missing = {};
  for (const [key, value] of Object.entries(stamps)) {
    if (entry[key] === undefined) missing[key] = value;
  }
  return missing;
};

// One O_APPEND write of the whole line: concurrent appenders cannot
// interleave partial lines. Creates the parent directory and the file
// on first append. Throws on failure; callers decide what that means
// (record() treats it as a no-op).
export const appendLine = (file, line) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const fd = fs.openSync(file, 'a');
  try {
    fs.writeSync(fd, line.endsWith('\n') ? line : `${line}\n`);
  } finally {
    fs.closeSync(fd);
  }
};

// Record one observation. Returns 'recorded', 'quarantined', or 'noop'
// (empty input, or any write failure). Never throws.
export const record = (
  raw,
  { target, cwd = process.cwd(), env = process.env, now = new Date() } = {}
) => {
  const file = target ?? resolveTarget(env);
  try {
    const stamps = discoverStamps({ cwd, env, now });
    const { entry, quarantine } = normaliseInput(raw, stamps);
    if (entry !== undefined) {
      appendLine(file, JSON.stringify(entry));
      return 'recorded';
    }
    if (quarantine !== undefined) {
      appendLine(quarantinePathFor(file), quarantine);
      return 'quarantined';
    }
    return 'noop';
  } catch {
    return 'noop';
  }
};

const readStdin = async () => {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  return Buffer.concat(chunks).toString('utf8');
};

const main = async () => {
  try {
    const args = process.argv.slice(2);
    const input =
      args.length > 0 ? args.join(' ') : process.stdin.isTTY ? '' : await readStdin();
    record(input);
  } catch {
    // Never reject: recording must not fail the caller's task.
  }
  process.exit(0);
};

// Entry-point check: compare realpaths, not the raw strings (#2235;
// same pattern as proxy-supervise.mjs / #2083). process.argv[1] keeps the
// path exactly as invoked (e.g. through a file or directory symlink)
// while import.meta.url is the resolved module URL, so a string compare
// silently never matches and main() never runs via a symlink.
const isMain = (() => {
  try {
    if (!process.argv[1]) return false;
    return fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();

if (isMain) {
  main();
}

