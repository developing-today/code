#!/usr/bin/env node
/**
 * opencode-usage-export — OpenCode usage exporter, slice 1 of
 * developing-today/code#2072 (owner direction 2026-10-29).
 *
 * PULL pattern, exactly like the Muse runtime-db export: this reads
 * OpenCode's own history database read-only on a cadence and writes a
 * file-export document for downstream attribution (reqs 4/5). NO push
 * instrumentation is added to OpenCode/OpenChamber.
 *
 * THE LIVE STORE (verified 2026-11-02 on the laptop, task 6610):
 *   ~/.local/share/opencode/opencode.db
 * The file is the managed instance's live store, but the live TABLES
 * are `session_v2` + `session_message` (4,489 sessions, newest activity
 * minutes before the probe). The legacy `session` / `message` / `part`
 * tables in the same file froze on 2026-10-06 — that frozen legacy
 * schema is the "stale default-path opencode.db" of the #2072
 * direction. This exporter reads ONLY the v2 tables and treats a store
 * whose session_v2 has no activity within the freshness window as
 * stale, so a future schema or path move fails closed instead of
 * exporting Oct-6 zeros forever.
 *
 * MOVE DETECTION: candidates are probed in order (explicit --db /
 * OPENCODE_USAGE_DB first, then the default candidates). The first
 * candidate with fresh session_v2 activity wins. If the winner is not
 * the previously recorded store path (state.json in the out-dir), the
 * document carries "moved": { "from": ..., "to": ... } so the move is
 * surfaced on the tick it happens. If NO candidate is fresh, the tick
 * writes a failure document and exits non-zero — never a zero-filled
 * success.
 *
 * Every run writes atomically to
 *   <out-dir>/latest.json     (default ~/atlas/opencode-usage-export/)
 * and appends the same document to <out-dir>/history.jsonl.
 * state.json in the out-dir records { storePath, lastTickAt }.
 *
 * Success document:
 *   { service: "opencode", source: "file-export/opencode-usage-laptop",
 *     status: "ok", measuredAt: <ISO>, storePath: <abs path>,
 *     storeNewestAt: <ISO>, windowHours: 24,
 *     moved: null | { from, to },
 *     sessions: [ { id, parentId, agent, directory,
 *                   model: { id, providerID },
 *                   cost, tokens: { input, output, reasoning,
 *                                   cacheRead, cacheWrite },
 *                   messages: { total, assistant,
 *                               byModel: [ { model, providerID, count,
 *                                            cost, tokens } ] },
 *                   timeCreated: <ISO>, timeUpdated: <ISO> } ],
 *     totals: { sessions, childSessions, cost, tokens: {...} } }
 *
 * Failure documents carry status + attemptedAt + detail and NO
 * sessions — a failed tick can never be mistaken for a reading:
 *   invalid-argument  a CLI value flag is missing, flag-shaped, NaN,
 *                  infinite, zero, or negative. Exit 2.
 *   store-missing  no candidate DB file exists. Exit 3.
 *   store-stale    candidates exist but none has fresh session_v2
 *                  activity (the store moved somewhere unknown, or
 *                  OpenCode has been down past the window). Exit 4.
 *   schema-drift   a candidate DB lacks the v2 tables. Exit 5.
 *   read-failed    any other read error. Exit 6.
 */
import { appendFile, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { DatabaseSync } from "node:sqlite";

export const SOURCE = "file-export/opencode-usage-laptop";
export const DEFAULT_WINDOW_HOURS = 24;
export const DEFAULT_FRESH_HOURS = 24;

export class ExportError extends Error {
  constructor(status, exitCode, detail) {
    super(detail);
    this.status = status;
    this.exitCode = exitCode;
  }
}

/** Candidate store paths, most-expected first. The first entry is the
 * verified live store (2026-11-02); the rest are the plausible move
 * targets (OpenChamber-managed data dir, XDG state fallback). */
export function defaultCandidatePaths(home = homedir()) {
  return [
    join(home, ".local/share/opencode/opencode.db"),
    join(home, ".local/share/openchamber/opencode.db"),
    join(home, ".local/state/opencode/opencode.db"),
  ];
}

const iso = (ms) => (ms == null ? null : new Date(ms).toISOString());
const zeroTokens = () => ({ input: 0, output: 0, reasoning: 0, cacheRead: 0, cacheWrite: 0 });
const addTokens = (acc, t) => {
  acc.input += t.input || 0;
  acc.output += t.output || 0;
  acc.reasoning += t.reasoning || 0;
  acc.cacheRead += t.cacheRead || 0;
  acc.cacheWrite += t.cacheWrite || 0;
  return acc;
};
const msgTokens = (t) => ({
  input: t?.input || 0,
  output: t?.output || 0,
  reasoning: t?.reasoning || 0,
  cacheRead: t?.cache?.read || 0,
  cacheWrite: t?.cache?.write || 0,
});

/** Parse a session/model JSON text column defensively: current rows
 * are JSON ({"id","providerID","variant"}); a plain string is treated
 * as the bare model id. */
export function parseModel(raw) {
  if (raw == null) return { id: null, providerID: null };
  if (typeof raw === "object") return { id: raw.id ?? null, providerID: raw.providerID ?? null };
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object") {
      return { id: parsed.id ?? null, providerID: parsed.providerID ?? null };
    }
  } catch {
    /* fall through: plain-string model id */
  }
  return { id: String(raw), providerID: null };
}

function openReadOnly(path) {
  const db = new DatabaseSync(path, { readOnly: true });
  db.exec("PRAGMA busy_timeout = 5000");
  db.exec("PRAGMA query_only = ON");
  return db;
}

/** Probe one candidate path. Returns { path, exists, hasV2Schema,
 * newestUpdated } — opening only far enough to classify the store. */
export function probeStore(path) {
  if (!existsSync(path)) return { path, exists: false, hasV2Schema: false, newestUpdated: null };
  let db;
  try {
    db = openReadOnly(path);
    const tables = db
      .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ('session_v2', 'session_message')")
      .all()
      .map((r) => r.name);
    if (!tables.includes("session_v2") || !tables.includes("session_message")) {
      return { path, exists: true, hasV2Schema: false, newestUpdated: null };
    }
    const row = db.prepare("SELECT max(time_updated) AS newest FROM session_v2").get();
    return { path, exists: true, hasV2Schema: true, newestUpdated: row?.newest ?? null };
  } finally {
    db?.close();
  }
}

/** Choose the live store. `previousPath` (from state.json) only
 * affects the moved report; freshness decides the winner. */
export function resolveStore({ candidates, previousPath = null, now = Date.now(), freshMs }) {
  const probes = candidates.map((p) => {
    try {
      return probeStore(p);
    } catch (err) {
      return { path: p, exists: existsSync(p), hasV2Schema: false, newestUpdated: null, error: String(err) };
    }
  });
  const winner = probes.find(
    (p) => p.hasV2Schema && p.newestUpdated != null && now - p.newestUpdated <= freshMs,
  );
  if (!winner) {
    const anyExists = probes.some((p) => p.exists);
    const anyV2 = probes.some((p) => p.hasV2Schema);
    if (!anyExists) {
      throw new ExportError("store-missing", 3, `no candidate OpenCode store exists: ${candidates.join(", ")}`);
    }
    if (!anyV2) {
      throw new ExportError(
        "schema-drift",
        5,
        `candidate stores exist but none has session_v2/session_message: ${probes.map((p) => p.path).join(", ")}`,
      );
    }
    const newest = Math.max(...probes.filter((p) => p.newestUpdated != null).map((p) => p.newestUpdated));
    throw new ExportError(
      "store-stale",
      4,
      `no candidate store has session_v2 activity within ${Math.round(freshMs / 3600000)}h; newest activity ${iso(newest)} — the live store may have moved; candidates: ${candidates.join(", ")}`,
    );
  }
  const moved =
    previousPath && previousPath !== winner.path ? { from: previousPath, to: winner.path } : null;
  return { path: winner.path, newestUpdated: winner.newestUpdated, moved, probes };
}

/** Read the export window out of an open store DB. Sessions come from
 * session_v2 (authoritative per-session cost/token totals); the
 * per-model breakdown is aggregated from session_message assistant
 * rows, whose data.model carries { id, providerID }.
 * All reads run inside a single read transaction so the session totals
 * and the per-message breakdown come from one consistent snapshot even
 * while the live instance is writing (Codex finding on PR #4680). */
export function readUsage(db, { sinceMs }) {
  db.exec("BEGIN");
  try {
    return readUsageSnapshot(db, { sinceMs });
  } finally {
    // COMMIT releases the snapshot; if the snapshot read threw, this
    // still ends the transaction (nothing was written — query_only).
    try { db.exec("COMMIT"); } catch { /* already ended */ }
  }
}

function readUsageSnapshot(db, { sinceMs }) {
  const sessionRows = db
    .prepare(
      `SELECT id, parent_id, agent, directory, model, cost,
              tokens_input, tokens_output, tokens_reasoning,
              tokens_cache_read, tokens_cache_write,
              time_created, time_updated
         FROM session_v2
        WHERE time_updated >= ?
        ORDER BY time_updated ASC`,
    )
    .all(sinceMs);
  const msgStmt = db.prepare("SELECT type, data FROM session_message WHERE session_id = ?");
  const sessions = [];
  const totals = { sessions: 0, childSessions: 0, cost: 0, tokens: zeroTokens() };
  for (const row of sessionRows) {
    const byModel = new Map();
    let total = 0;
    let assistant = 0;
    for (const msg of msgStmt.iterate(row.id)) {
      total += 1;
      if (msg.type !== "assistant") continue;
      assistant += 1;
      let data;
      try {
        data = JSON.parse(msg.data);
      } catch {
        continue;
      }
      const model = parseModel(data.model);
      const key = `${model.providerID ?? ""}\u0000${model.id ?? ""}`;
      const entry =
        byModel.get(key) ??
        { model: model.id, providerID: model.providerID, count: 0, cost: 0, tokens: zeroTokens() };
      entry.count += 1;
      entry.cost += data.cost || 0;
      addTokens(entry.tokens, msgTokens(data.tokens));
      byModel.set(key, entry);
    }
    const tokens = {
      input: row.tokens_input || 0,
      output: row.tokens_output || 0,
      reasoning: row.tokens_reasoning || 0,
      cacheRead: row.tokens_cache_read || 0,
      cacheWrite: row.tokens_cache_write || 0,
    };
    sessions.push({
      id: row.id,
      parentId: row.parent_id ?? null,
      agent: row.agent ?? null,
      directory: row.directory ?? null,
      model: parseModel(row.model),
      cost: row.cost || 0,
      tokens,
      messages: { total, assistant, byModel: [...byModel.values()] },
      timeCreated: iso(row.time_created),
      timeUpdated: iso(row.time_updated),
    });
    totals.sessions += 1;
    if (row.parent_id) totals.childSessions += 1;
    totals.cost += row.cost || 0;
    addTokens(totals.tokens, tokens);
  }
  return { sessions, totals };
}

export function successDocument({ measuredAt, storePath, storeNewestAt, windowHours, moved, sessions, totals }) {
  return {
    service: "opencode",
    source: SOURCE,
    status: "ok",
    measuredAt,
    storePath,
    storeNewestAt,
    windowHours,
    moved,
    sessions,
    totals,
  };
}

export function failureDocument(status, detail, attemptedAt) {
  return { service: "opencode", source: SOURCE, status, attemptedAt, detail };
}

async function writeDocument(outDir, doc) {
  await mkdir(outDir, { recursive: true });
  const tmp = join(outDir, `.latest.json.tmp-${process.pid}`);
  await writeFile(tmp, `${JSON.stringify(doc, null, 2)}\n`);
  await rename(tmp, join(outDir, "latest.json"));
  await appendFile(join(outDir, "history.jsonl"), `${JSON.stringify(doc)}\n`);
}

async function readState(outDir) {
  try {
    return JSON.parse(await readFile(join(outDir, "state.json"), "utf8"));
  } catch {
    return {};
  }
}

/** Extract --out-dir without validating anything else, so even a
 * usage failure can record its document where the caller asked. A
 * missing or flag-shaped --out-dir falls back to the default. */
function peekOutDir(argv, home) {
  for (let i = 0; i < argv.length - 1; i += 1) {
    if (argv[i] === "--out-dir") {
      const v = argv[i + 1];
      if (v != null && !v.startsWith("--")) return v;
    }
  }
  return join(home, "atlas/opencode-usage-export");
}

function parseArgs(argv) {
  const args = { db: null, outDir: null, windowHours: DEFAULT_WINDOW_HOURS, freshHours: DEFAULT_FRESH_HOURS };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--db") args.db = argv[++i];
    else if (a === "--out-dir") args.outDir = argv[++i];
    else if (a === "--window-hours") args.windowHours = parseHoursFlag(a, argv[++i]);
    else if (a === "--fresh-hours") args.freshHours = parseHoursFlag(a, argv[++i]);
    else throw new Error(`unknown argument: ${a}`);
  }
  return args;
}

/** Validate a value-taking hours flag. A missing token, a flag-shaped
 * token, NaN, an infinite value, zero, or a negative value is a usage
 * error (exit 2) that names the flag — raised before any store is
 * opened or read, so junk can never produce a zero-filled success or
 * a misleading store-stale diagnosis. */
function parseHoursFlag(flag, raw) {
  const value = Number(raw);
  if (raw == null || !Number.isFinite(value) || value <= 0) {
    throw new ExportError(
      "invalid-argument",
      2,
      `${flag} requires a positive finite number of hours, got ${raw == null ? "(missing)" : JSON.stringify(String(raw))}`,
    );
  }
  return value;
}

/** One exporter tick. Injectable `now`/`home` keep the tests offline
 * and deterministic; production uses the defaults. */
export async function run({
  argv = [],
  env = process.env,
  now = Date.now(),
  home = homedir(),
} = {}) {
  const attemptedAt = new Date(now).toISOString();
  const outDir = peekOutDir(argv, home);
  try {
    const args = parseArgs(argv);
    const explicit = args.db ?? env.OPENCODE_USAGE_DB ?? null;
    const candidates = explicit ? [explicit] : defaultCandidatePaths(home);
    const state = await readState(outDir);
    const { path, newestUpdated, moved } = resolveStore({
      candidates,
      previousPath: state.storePath ?? null,
      now,
      freshMs: args.freshHours * 3600000,
    });
    let db;
    try {
      db = openReadOnly(path);
      const sinceMs = now - args.windowHours * 3600000;
      const { sessions, totals } = readUsage(db, { sinceMs });
      const doc = successDocument({
        measuredAt: attemptedAt,
        storePath: path,
        storeNewestAt: iso(newestUpdated),
        windowHours: args.windowHours,
        moved,
        sessions,
        totals,
      });
      await writeDocument(outDir, doc);
      await writeFile(
        join(outDir, "state.json"),
        `${JSON.stringify({ storePath: path, lastTickAt: attemptedAt }, null, 2)}\n`,
      );
      return { exitCode: 0, doc };
    } finally {
      db?.close();
    }
  } catch (err) {
    const e =
      err instanceof ExportError
        ? err
        : new ExportError("read-failed", 6, `opencode usage export failed: ${err?.message ?? err}`);
    const doc = failureDocument(e.status, e.message, attemptedAt);
    try {
      await writeDocument(outDir, doc);
    } catch {
      /* nowhere to write the failure either; exit code carries it */
    }
    return { exitCode: e.exitCode, doc };
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { exitCode, doc } = await run({ argv: process.argv.slice(2) });
  console.log(JSON.stringify({ status: doc.status, exitCode }));
  process.exit(exitCode);
}
