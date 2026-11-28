#!/usr/bin/env node
// check-opencode-go-seating — registration/seating guard for `opencode-go.*`
// instance leaves (developing-today/code#5611).
//
// The failure this pins (2026-11-03/04): provisioning the popesdrew Go
// instance stored its console-login OAuth credential under the bare
// integration id `opencode` instead of `opencode-go.popesdrew`. The route
// selector matches credentials by the EXACT leaf provider id
// (`WHERE integration_id = <leaf id> AND active = 1`), so every attempt on
// that leaf failed with "no credential for provider opencode-go.popesdrew":
// 23 failed attempts, the account served 0% for ~16h, and nothing noticed —
// provisioning ended at "rows created" and the weighted pool config
// (drewrypope 2 : popesdrew 1) was correct as deployed.
//
// The guard asserts, for every `opencode-go.*` leaf the config seats in a
// pool (a provider-route group's `providers` list or a model-route target):
//
//   1. credential  A credential row exists under the EXACT leaf integration
//                  id (never the bare `opencode` console-login key) with
//                  active = 1. Missing → registration fails loudly.
//   2. attempt     A SUCCESSFUL route attempt for that leaf is recorded
//                  after the active config's latest modification (a
//                  conservative seating-time proxy), within a bounded window.
//                  Absent → the leaf is not seated; raise an alert.
//
// Surface substitution (#5611 scope note): the standalone route proxy whose
// ledger the issue named was retired under opencode#1153; the successor
// surface is the `route_attempt` table in the OpenCode history store
// (`~/.local/share/opencode/opencode.db`), which the in-fork route selector
// writes for every attempt and the usage aggregator reads. The attempt
// assertion therefore reads `route_attempt` by `provider_id = <leaf id>`
// and `outcome = 'success'`. Recorded on issue #5611.
//
// Read-only. Prints provider ids, credential ids/labels and attempt counts —
// never token values.
//
// Usage:
//   check-opencode-go-seating.mjs [--config <jsonc>] [--db <opencode.db>]
//                                 [--leaf opencode-go.<id>] [--window-hours 24]
//                                 [--stage <s>]
//   --stage pre-seat   credential assertion only (run before seating a new
//                      leaf: no attempt can exist yet)
//   --stage post-seat  credential + a successful attempt within the window
//   --stage periodic   same as post-seat; what the timer/service runs
//                      (default)
//   --leaf             add an explicit registration target. Use with
//                      --stage pre-seat BEFORE adding the leaf to a pool;
//                      after seating, --stage post-seat requires a recent
//                      success as well.
//   --skip-attempts    shorthand for --stage pre-seat
//   --json             machine-readable report on stdout
//
// Exit: 0 healthy, 1 violation (not seated), 2 check error (config or store
// unreadable — loud, never a silent pass).

import { existsSync, realpathSync, statSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { DatabaseSync } from 'node:sqlite';

import { parseJsonc } from './provider-catalog-sync.mjs';

export const DEFAULT_CONFIG = '.opencode/opencode.jsonc';
export const DEFAULT_STORE = '~/.local/share/opencode/opencode.db';
export const DEFAULT_WINDOW_HOURS = 24;
export const LEAF_PREFIX = 'opencode-go.';

const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);

function expandHome(path) {
  return path.startsWith('~') ? resolve(homedir(), `.${path.slice(1)}`) : path;
}

/** Collect `provider/model` strings from a route definition's selections.
 * Mirrors the shape the metered-provider guard walks: model targets may be
 * plain strings or `{ model }` objects. */
function collectSelections(node, out) {
  if (typeof node === 'string') {
    out.push(node);
    return;
  }
  if (Array.isArray(node)) {
    for (const item of node) collectSelections(item, out);
    return;
  }
  if (isObject(node)) {
    if (typeof node.model === 'string') out.push(node.model);
    for (const key of ['targets', 'fallback', 'fallbacks']) {
      if (key in node) collectSelections(node[key], out);
    }
  }
}

/** Every `opencode-go.*` provider id the config seats in a pool.
 *
 * Seating is membership, not mention: a leaf is seated when it appears in a
 * provider-route group's `providers` list or as the provider of a model-route
 * target. A catalog/`provider` block alone does not seat it.
 *
 * `requestedLeaves` are explicit registration targets. They matter for
 * pre-seat checks, when a new instance is intentionally not in any pool yet.
 * Returns a Map leafId -> Set of seat descriptions.
 */
export function seatedGoLeaves(config, requestedLeaves = []) {
  const seats = new Map();
  const seat = (leaf, where) => {
    if (!seats.has(leaf)) seats.set(leaf, new Set());
    seats.get(leaf).add(where);
  };

  // The experimental map wins when present; otherwise the top-level map is
  // the active runtime input (same fallback as the route proxy and the
  // metered-provider guard).
  const experimental = isObject(config?.experimental) ? config.experimental : {};
  const providerRoutes = isObject(experimental.provider_routes)
    ? experimental.provider_routes
    : isObject(config?.provider_routes)
      ? config.provider_routes
      : {};
  const modelRoutes = isObject(experimental.model_routes)
    ? experimental.model_routes
    : isObject(config?.model_routes)
      ? config.model_routes
      : {};

  const walkGroups = (node, path) => {
    if (Array.isArray(node)) {
      node.forEach((item, index) => walkGroups(item, `${path}[${index}]`));
      return;
    }
    if (!isObject(node)) return;
    // Group maps key by group NAME, so a group literally named "providers" is
    // still a group: iterate entries as names, then walk each value as a
    // group definition.
    for (const [name, value] of Object.entries(node)) {
      walkGroupDefinition(value, `${path}.${name}`);
    }
  };
  const walkGroupDefinition = (node, path) => {
    if (Array.isArray(node)) {
      node.forEach((item, index) => walkGroupDefinition(item, `${path}[${index}]`));
      return;
    }
    if (!isObject(node)) return;
    for (const [key, value] of Object.entries(node)) {
      if (key === 'providers') {
        for (const provider of Array.isArray(value) ? value : [value]) {
          if (typeof provider === 'string' && provider.startsWith(LEAF_PREFIX)) {
            seat(provider, `${path}.providers`);
          }
        }
      } else {
        walkGroupDefinition(value, `${path}.${key}`);
      }
    }
  };
  walkGroups(providerRoutes, '$' + (isObject(experimental.provider_routes) ? '.experimental' : '') + '.provider_routes');

  for (const [name, definition] of Object.entries(modelRoutes)) {
    const selections = [];
    collectSelections(definition, selections);
    selections.forEach((selection) => {
      const provider = selection.split('/', 1)[0];
      if (provider.startsWith(LEAF_PREFIX)) {
        seat(provider, `$.model_routes.${name}`);
      }
    });
  }

  for (const leaf of requestedLeaves) {
    if (typeof leaf !== 'string' || !leaf.startsWith(LEAF_PREFIX) || leaf.length === LEAF_PREFIX.length) {
      throw new Error(`invalid --leaf ${JSON.stringify(leaf)}; expected an opencode-go.<instance-id> leaf`);
    }
    seat(leaf, 'requested registration target (not yet seated)');
  }

  return seats;
}

/** Rows shaped { integrationId, credentialId, label, active }. */
export function credentialViolations(leaf, rows) {
  const violations = [];
  const exact = rows.filter((row) => row.integrationId === leaf);
  const active = exact.filter((row) => row.active === true || row.active === 1);
  if (exact.length === 0) {
    const bare = rows
      .filter((row) => row.integrationId === 'opencode')
      .map((row) => `id=${row.credentialId} active=${row.active === true || row.active === 1}`);
    violations.push(
      `${leaf}: no credential row exists under the exact instance id` +
        (bare.length
          ? ` (a bare "opencode" console-login row exists: ${bare.join(', ')} — the selector matches the leaf id, not the console-login key)`
          : ''),
    );
  } else if (active.length === 0) {
    violations.push(
      `${leaf}: credential row exists under the exact id but none is active (rows: ` +
        exact.map((row) => `id=${row.credentialId} active=${row.active}`).join(', ') +
        ')',
    );
  }
  return violations;
}

/** attempts shaped { outcome, timeStarted (ms), errorTag }. */
export function attemptViolations(leaf, attempts, { now, windowHours, configUpdatedAt = 0 }) {
  const cutoff = now - windowHours * 3600 * 1000;
  const afterSeat = Math.max(cutoff, configUpdatedAt);
  const recent = attempts.filter((attempt) => attempt.timeStarted >= afterSeat);
  if (recent.some((attempt) => attempt.outcome === 'success')) return [];
  const last = attempts.reduce(
    (latest, attempt) => (latest === null || attempt.timeStarted > latest.timeStarted ? attempt : latest),
    null,
  );
  const detail = last
    ? `most recent attempt ${new Date(last.timeStarted).toISOString()} outcome=${last.outcome}` +
      (last.errorTag ? ` error_tag=${last.errorTag}` : '')
    : 'no attempt recorded at all';
  return [
    `${leaf}: no successful route attempt within ${windowHours}h and after the active config's latest modification (${detail}) — the leaf is seated in a pool but has not served`,
  ];
}

/** Full report over a config + store snapshot.
 *
 * `store` is { credentialRows, attemptsByProvider } and may be omitted with
 * stage='pre-seat' (credential assertion only).
 */
export function inspectSeating({ config, store, stage = 'periodic', now = Date.now(), windowHours = DEFAULT_WINDOW_HOURS, leafIds = [], configUpdatedAt = 0 }) {
  const seats = seatedGoLeaves(config, leafIds);
  const checkAttempts = stage !== 'pre-seat';
  const violations = [];
  const leaves = [...seats.keys()].sort();
  for (const leaf of leaves) {
    const rows = store?.credentialRows ?? [];
    violations.push(...credentialViolations(leaf, rows));
    if (checkAttempts && store) {
      const attempts = store.attemptsByProvider.get?.(leaf) ?? store.attemptsByProvider[leaf] ?? [];
      violations.push(...attemptViolations(leaf, attempts, { now, windowHours, configUpdatedAt }));
    }
  }
  return {
    leaves: leaves.map((leaf) => ({ leaf, seats: [...seats.get(leaf)].sort() })),
    stage,
    windowHours,
    violations,
  };
}

function readStore(dbPath, leaves) {
  const db = new DatabaseSync(dbPath, { readOnly: true });
  try {
    const credentialRows = db
      .prepare('SELECT id, integration_id, label, active FROM credential')
      .all()
      .map((row) => ({
        credentialId: row.id,
        integrationId: row.integration_id,
        label: row.label,
        active: row.active,
      }));
    const attemptsByProvider = new Map();
    const stmt = db.prepare(
      'SELECT provider_id, outcome, time_started, error_tag FROM route_attempt WHERE provider_id = ?',
    );
    for (const leaf of leaves) {
      const rows = stmt.all(leaf).map((row) => ({
        outcome: row.outcome,
        timeStarted: Number(row.time_started) < 1e12 ? Number(row.time_started) * 1000 : Number(row.time_started),
        errorTag: row.error_tag,
      }));
      attemptsByProvider.set(leaf, rows);
    }
    return { credentialRows, attemptsByProvider };
  } finally {
    db.close();
  }
}

export function parseArgs(argv) {
  const options = {
    config: DEFAULT_CONFIG,
    db: DEFAULT_STORE,
    windowHours: DEFAULT_WINDOW_HOURS,
    stage: 'periodic',
    json: false,
    leafIds: [],
  };
  // A bare positional is the config path, matching the metered-provider guard's
  // `check-opencode-metered-providers.mjs <config>` invocation.
  if (argv.length && !argv[0].startsWith('-')) {
    options.config = argv[0];
    argv = argv.slice(1);
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      const value = argv[i + 1];
      if (value === undefined) throw new Error(`${arg} needs a value`);
      i += 1;
      return value;
    };
    switch (arg) {
      case '--config':
        options.config = next();
        break;
      case '--db':
        options.db = next();
        break;
      case '--leaf':
        options.leafIds.push(next());
        break;
      case '--window-hours': {
        const value = Number(next());
        if (!Number.isFinite(value) || value <= 0) throw new Error('--window-hours must be a positive number');
        options.windowHours = value;
        break;
      }
      case '--stage': {
        const value = next();
        if (!['pre-seat', 'post-seat', 'periodic'].includes(value)) {
          throw new Error('--stage must be pre-seat, post-seat or periodic');
        }
        options.stage = value;
        break;
      }
      case '--skip-attempts':
        options.stage = 'pre-seat';
        break;
      case '--json':
        options.json = true;
        break;
      case '--help':
      case '-h':
        options.help = true;
        break;
      default:
        throw new Error(`unknown argument: ${arg}`);
    }
  }
  if (options.stage === 'pre-seat' && options.leafIds.length === 0) {
    throw new Error('--stage pre-seat requires at least one --leaf registration target');
  }
  return options;
}

async function main(argv) {
  const options = parseArgs(argv);
  if (options.help) {
    console.log(
      'usage: check-opencode-go-seating.mjs [--config <jsonc>] [--db <opencode.db>] ' +
        '[--leaf opencode-go.<id>] [--window-hours 24] [--stage pre-seat|post-seat|periodic] [--json]',
    );
    return 0;
  }
  const configPath = resolve(options.config);
  const dbPath = expandHome(options.db);
  const config = parseJsonc(await readFile(configPath, 'utf8'));
  // A success must postdate the active config's latest write. This is a
  // conservative proxy for pool seating time: any later config edit resets
  // the proof window rather than allowing a pre-seating success to count.
  const configUpdatedAt = statSync(configPath).mtimeMs;
  const seats = seatedGoLeaves(config, options.leafIds);
  const leaves = [...seats.keys()].sort();
  if (options.stage !== 'pre-seat' && !existsSync(dbPath)) {
    console.error(`VIOLATION: credential store not found: ${dbPath}`);
    return 2;
  }
  const store = leaves.length === 0 ? { credentialRows: [], attemptsByProvider: new Map() } : readStore(dbPath, leaves);
  const report = inspectSeating({ config, store, stage: options.stage, windowHours: options.windowHours, leafIds: options.leafIds, configUpdatedAt });
  if (options.json) {
    console.log(JSON.stringify({ config: configPath, db: dbPath, ...report }, null, 2));
  } else if (report.violations.length) {
    console.error(`Go instance seating guard failed for ${configPath}:`);
    for (const violation of report.violations) console.error(`- ${violation}`);
  } else {
    const attemptNote = options.stage === 'pre-seat' ? 'credential rows only (pre-seat)' : `credential rows + a successful attempt within ${options.windowHours}h`;
    console.log(
      `Go instance seating guard passed: ${configPath} (${leaves.length} seated leaf/leaves; ${attemptNote})`,
    );
  }
  return report.violations.length ? 1 : 0;
}

// Entry-point check: compare realpaths, not the raw strings (#4694; same
// pattern as check-opencode-metered-providers.mjs / #2235).
const isMain = (() => {
  try {
    if (!process.argv[1]) return false;
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();

if (isMain) {
  main(process.argv.slice(2)).then(
    (code) => {
      process.exitCode = code;
    },
    (error) => {
      console.error(`VIOLATION: Go instance seating check failed: ${error?.message ?? error}`);
      process.exitCode = 2;
    },
  );
}
