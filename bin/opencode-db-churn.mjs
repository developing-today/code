#!/usr/bin/env node
// opencode-db-churn.mjs — bounded page-level churn sampler for the OpenCode
// session DB (research follow-through for developing-today/code#9094,
// workstream B of the Atlas program #8983).
//
// A daily restic snapshot of the live DB already exists
// (nixos/home/opencode-db-backup.sh, timer opencode-db-backup, local repo
// ~/backups/restic/opencode-db): each run VACUUMs the live DB into a
// staging file and backs the staging file up, so the restic snapshot set
// is a sequence of transaction-consistent full images. The cheapest
// bounded daily churn figure is therefore `restic diff` between two
// consecutive snapshots: "Added" (post-dedupe, compressed new blobs) is
// the restic-delta daily cost and "Removed" bounds the superseded bytes.
// No new capture schedule is needed and nothing ever reads the live DB.
//
// Subcommands (exactly one per invocation; anything else is a usage error):
//   diff <older-id> <newer-id>   churn between two snapshots of one repo
//                                (prints the diff summary + a JSONL record
//                                on stdout; with --append also appends the
//                                record to the ledger file).
//   ledger                       print every record in the ledger file
//                                (one JSON object per line).
//   info                         repo + tool versions and snapshot count
//                                (read-only; used to verify the setup).
//
// Options (value options take `--opt value` or `--opt=value`):
//   --repo <path>     restic repository (default
//                     $HOME/backups/restic/opencode-db).
//   --ledger <path>   JSONL ledger (default
//                     $HOME/backups/measure-9094/churn-ledger.jsonl).
//   --append          diff only: also append the record to the ledger.
//   --json            diff only: print only the JSON record (no summary).
//   --help            print usage and exit 0.
//
// Argument contract (#3352, strict pattern copied from the covered
// sibling bin/provider-catalog-sync.mjs): the whole command line is
// declared above and parsed strictly before any subprocess, file write,
// or network step. Unknown flags, stray positionals, typo'd flags,
// present-but-valueless options, and `--opt=value` vs `--opt value`
// mismatches are usage errors (exit 2) naming the offending token.
// `--opt=value` is accepted identically to `--opt value`. Reading the
// ledger with `ledger` never writes; only `diff --append` writes, and
// only to the ledger file.

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export const VALUE_OPTIONS = ['--repo', '--ledger'];
export const FLAG_OPTIONS = ['--append', '--json', '--help'];
export const SUBCOMMANDS = ['diff', 'ledger', 'info'];
export const USAGE =
  'usage: opencode-db-churn <diff <older-id> <newer-id> [--append] | ledger | info> ' +
  '[--repo <path>] [--ledger <path>] [--json]';

export class UsageError extends Error {}

function splitOpt(token) {
  const eq = token.indexOf('=');
  if (eq === -1) return [token, null];
  return [token.slice(0, eq), token.slice(eq + 1)];
}

export function parseArgs(argv) {
  const options = {};
  const flags = new Set();
  const positionals = [];
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (token.startsWith('--')) {
      const [name, inline] = splitOpt(token);
      if (FLAG_OPTIONS.includes(name)) {
        if (inline !== null) {
          throw new UsageError(`flag ${name} takes no value, got ${inline}\n${USAGE}`);
        }
        flags.add(name);
        continue;
      }
      if (VALUE_OPTIONS.includes(name)) {
        if (Object.hasOwn(options, name)) {
          throw new UsageError(`${name} was given more than once\n${USAGE}`);
        }
        let value = inline;
        if (value === null) {
          value = argv[i + 1];
          if (value === undefined || value.startsWith('--')) {
            throw new UsageError(
              `${name} requires a value, but ${
                value === undefined ? 'no value followed it' : `the next token is the flag ${value}`
              }\n${USAGE}`,
            );
          }
          i += 1;
        } else if (value === '') {
          throw new UsageError(`${name} requires a non-empty value\n${USAGE}`);
        }
        options[name] = value;
        continue;
      }
      throw new UsageError(`unrecognised argument: ${token}\n${USAGE}`);
    }
    positionals.push(token);
  }
  return { options, flags, positionals };
}

function defaultRepo(env) {
  const home = env.HOME;
  if (!home) throw new UsageError(`HOME is unset: cannot default --repo\n${USAGE}`);
  return path.join(home, 'backups', 'restic', 'opencode-db');
}

function defaultLedger(env) {
  const home = env.HOME;
  if (!home) throw new UsageError(`HOME is unset: cannot default --ledger\n${USAGE}`);
  return path.join(home, 'backups', 'measure-9094', 'churn-ledger.jsonl');
}

function run(cmd, args, env) {
  return execFileSync(cmd, args, { encoding: 'utf8', env, maxBuffer: 64 * 1024 * 1024 });
}

function resticEnv(env) {
  return { ...env, RESTIC_REPOSITORY: env.RESTIC_REPOSITORY };
}

// Parse the tail summary of `restic diff` output. Returns
// { dataBlobsNew, dataBlobsRemoved, addedBytes, removedBytes }.
// Throws UsageError when the output does not match the expected shape,
// so a restic format change fails loudly instead of recording zeros.
export function parseDiffSummary(text) {
  const blobs = text.match(/Data Blobs:\s+(\d+)\s+new,\s+(\d+)\s+removed/);
  const added = text.match(/Added:\s+([\d.]+)\s+([KMGT]?i?B)/);
  const removed = text.match(/Removed:\s+([\d.]+)\s+([KMGT]?i?B)/);
  if (!blobs || !added || !removed) {
    throw new UsageError(`could not parse restic diff summary from output:\n${text}`);
  }
  return {
    dataBlobsNew: Number(blobs[1]),
    dataBlobsRemoved: Number(blobs[2]),
    addedBytes: toBytes(Number(added[1]), added[2]),
    removedBytes: toBytes(Number(removed[1]), removed[2]),
  };
}

const UNIT_MULT = { B: 1, KB: 1000, MB: 1000 ** 2, GB: 1000 ** 3, TB: 1000 ** 4, KiB: 1024, MiB: 1024 ** 2, GiB: 1024 ** 3, TiB: 1024 ** 4 };

export function toBytes(n, unit) {
  const mult = UNIT_MULT[unit];
  if (mult === undefined) throw new UsageError(`unknown restic size unit: ${unit}`);
  return Math.round(n * mult);
}

function snapshotTime(repo, id, env) {
  const out = run('restic', ['-r', repo, 'snapshots', id, '--json'], env);
  const parsed = JSON.parse(out);
  const snap = Array.isArray(parsed) ? parsed[0] : parsed;
  if (!snap || !snap.time) throw new UsageError(`no snapshot time for id ${id}`);
  return snap.time;
}

async function main(argv, env = process.env) {
  const { options, flags, positionals } = parseArgs(argv);
  if (flags.has('--help')) {
    console.log(USAGE);
    return 0;
  }
  const [sub, ...rest] = positionals;
  if (!SUBCOMMANDS.includes(sub)) {
    throw new UsageError(
      `${sub === undefined ? 'no subcommand given' : `unrecognised subcommand: ${sub}`}\n${USAGE}`,
    );
  }
  const repo = options['--repo'] ?? defaultRepo(env);
  const ledgerPath = options['--ledger'] ?? defaultLedger(env);
  const renv = { ...env, RESTIC_REPOSITORY: repo };
  if (!env.RESTIC_PASSWORD) {
    throw new UsageError('RESTIC_PASSWORD is unset in the environment (fetch restic.repo-archive-password by name at runtime, never log it)');
  }

  if (sub === 'info') {
    if (rest.length > 0) throw new UsageError(`info takes no snapshot ids\n${USAGE}`);
    if (flags.has('--append') || flags.has('--json')) {
      throw new UsageError(`--append/--json only apply to diff\n${USAGE}`);
    }
    const version = run('restic', ['version'], renv).trim();
    const snaps = JSON.parse(run('restic', ['-r', repo, 'snapshots', '--json'], renv));
    console.log(JSON.stringify({ repo, restic: version, snapshots: snaps.length }, null, 1));
    return 0;
  }

  if (sub === 'ledger') {
    if (rest.length > 0) throw new UsageError(`ledger takes no snapshot ids\n${USAGE}`);
    if (flags.has('--append') || flags.has('--json')) {
      throw new UsageError(`--append/--json only apply to diff\n${USAGE}`);
    }
    if (!fs.existsSync(ledgerPath)) {
      console.log(JSON.stringify({ ledger: ledgerPath, records: 0 }));
      return 0;
    }
    process.stdout.write(fs.readFileSync(ledgerPath, 'utf8'));
    return 0;
  }

  // diff
  if (rest.length !== 2) {
    throw new UsageError(`diff takes exactly two snapshot ids, got ${rest.length}\n${USAGE}`);
  }
  if (flags.has('--help')) {
    console.log(USAGE);
    return 0;
  }
  const [older, newer] = rest;
  const raw = run('restic', ['-r', repo, 'diff', older, newer], renv);
  const summary = parseDiffSummary(raw);
  const record = {
    measured_at: new Date().toISOString(),
    repo,
    older,
    newer,
    older_time: snapshotTime(repo, older, renv),
    newer_time: snapshotTime(repo, newer, renv),
    ...summary,
  };
  if (flags.has('--append')) {
    fs.mkdirSync(path.dirname(ledgerPath), { recursive: true });
    fs.appendFileSync(ledgerPath, `${JSON.stringify(record)}\n`, 'utf8');
    record.ledger = ledgerPath;
  }
  if (flags.has('--json')) {
    console.log(JSON.stringify(record));
  } else {
    console.log(raw.trimEnd());
    console.log(JSON.stringify(record));
  }
  return 0;
}

const isMain = process.argv[1] !== undefined && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname);
if (isMain) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code ?? 0),
    (err) => {
      if (err instanceof UsageError) {
        process.stderr.write(`opencode-db-churn: usage error: ${err.message}\n`);
        process.exit(2);
      }
      process.stderr.write(`opencode-db-churn: FAILED ${err?.message ?? err}\n`);
      process.exit(1);
    },
  );
}
