#!/usr/bin/env bun
// Sync the decision-set event outbox to the external sinks (#9335
// Track B): Turso (fleet-platform pattern, code#5690) and/or the
// decision-events Cloudflare Worker webhook (which stores the events
// in D1 and fans them out via a Durable Object per set).
//
// Command-line contract (#3352 — declared in full, parsed strictly
// before any network/file work; anything unaccounted for is a usage
// error, exit 2):
//
//   bun run scripts/sync-decision-events.ts
//     [--file <outbox.jsonl>]...        outbox file(s); repeatable
//     [--outbox-dir <dir>]              also read every *.jsonl in dir
//     [--target turso|webhook|both]     default: both when
//                                       DECISION_EVENTS_WEBHOOK_URL is
//                                       set, else turso
//     [--state <path>]                  per-leg sync-state file
//     [--dry-run]                       report pending counts only
//
// Credentials are resolved BY NAME from Bitwarden Secrets Manager
// (src/secrets.ts) — `turso.fleet-platform-rw-token` and
// `cloudflare.decision-events.webhook-secret` — or taken from the
// DECISION_EVENTS_TURSO_TOKEN / DECISION_EVENTS_WEBHOOK_SECRET env
// overrides (e.g. a systemd env file). Values are never printed.
//
// Env: DECISION_EVENTS_OUTBOX_FILE, DECISION_EVENTS_STATE_FILE,
//   DECISION_EVENTS_TURSO_URL (default the fleet-platform DB),
//   DECISION_EVENTS_WEBHOOK_URL (the Worker's /events endpoint),
//   DECISION_EVENTS_SYNC=0 (kill switch: no-op, exit 0).
//
// Idempotent: both sinks dedupe on eventId and the sync state only
// advances for acked events, so re-running (timer, cron, by hand) is
// always safe. Exit 0 = every requested leg synced (or had nothing
// pending); exit 1 = at least one leg failed (details on stdout).
import { readdir } from "node:fs/promises";
import { join } from "node:path";
import {
  DECISION_EVENTS_TURSO_TOKEN_NAME,
  DECISION_EVENTS_TURSO_URL,
  DECISION_EVENTS_WEBHOOK_SECRET_NAME,
  defaultDecisionEventsOutboxFile,
  defaultDecisionEventsStateFile,
  loadSyncState,
  pendingForLeg,
  readOutboxEvents,
  saveSyncState,
  syncDecisionEvents,
  SYNC_LEGS,
  type DecisionSetEvent,
  type SyncLeg,
} from "../src/decision-events-sync.ts";
import { resolveBwsSecret } from "../src/secrets.ts";

const USAGE =
  "usage: sync-decision-events.ts [--file <path>]... [--outbox-dir <dir>] [--target turso|webhook|both] [--state <path>] [--dry-run]";

function usageError(message: string): never {
  console.error(`sync-decision-events: ${message}`);
  console.error(USAGE);
  process.exit(2);
}

const VALUE_FLAGS = new Set(["--file", "--outbox-dir", "--target", "--state"]);

function parseArgs(): { files: string[]; outboxDir?: string; target?: string; state?: string; dryRun: boolean } {
  const args = process.argv.slice(2);
  const files: string[] = [];
  let outboxDir: string | undefined;
  let target: string | undefined;
  let state: string | undefined;
  let dryRun = false;
  const seenSingleton = new Set<string>();
  for (let i = 0; i < args.length; i++) {
    const token = args[i] as string;
    if (token === "--dry-run") {
      if (dryRun) usageError("duplicate --dry-run");
      dryRun = true;
      continue;
    }
    if (!VALUE_FLAGS.has(token)) usageError(`unknown argument ${JSON.stringify(token)}`);
    const value = args[i + 1];
    if (value === undefined || value === "" || value.startsWith("--")) usageError(`${token} requires a value`);
    i++;
    if (token === "--file") {
      files.push(value);
    } else {
      if (seenSingleton.has(token)) usageError(`duplicate ${token}`);
      seenSingleton.add(token);
      if (token === "--outbox-dir") outboxDir = value;
      else if (token === "--target") target = value;
      else state = value;
    }
  }
  if (target !== undefined && target !== "turso" && target !== "webhook" && target !== "both") {
    usageError(`--target must be turso, webhook, or both (got ${JSON.stringify(target)})`);
  }
  return { files, outboxDir, target, state, dryRun };
}

async function main(): Promise<number> {
  if (process.env.DECISION_EVENTS_SYNC === "0") {
    console.log("sync-decision-events: disabled by DECISION_EVENTS_SYNC=0");
    return 0;
  }
  const opts = parseArgs();
  const files = opts.files.length > 0 ? [...opts.files] : [defaultDecisionEventsOutboxFile()];
  if (opts.outboxDir) {
    let entries: string[];
    try {
      entries = await readdir(opts.outboxDir);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error(`sync-decision-events: cannot read --outbox-dir ${JSON.stringify(opts.outboxDir)}: ${msg}`);
      return 1;
    }
    for (const name of entries.filter((n) => n.endsWith(".jsonl")).sort()) files.push(join(opts.outboxDir, name));
  }

  const events: DecisionSetEvent[] = [];
  const seen = new Set<string>();
  let badLines = 0;
  for (const file of files) {
    const read = await readOutboxEvents(file);
    badLines += read.errors.length;
    for (const e of read.errors) console.log(`sync-decision-events: ${file}:${e.line}: skipped (${e.error})`);
    for (const ev of read.events) {
      if (seen.has(ev.eventId)) continue;
      seen.add(ev.eventId);
      events.push(ev);
    }
  }

  const webhookUrl = process.env.DECISION_EVENTS_WEBHOOK_URL?.trim() || undefined;
  const legs: SyncLeg[] =
    opts.target === "both"
      ? [...SYNC_LEGS]
      : opts.target === "turso" || opts.target === "webhook"
        ? [opts.target]
        : webhookUrl
          ? [...SYNC_LEGS]
          : ["turso"];

  const statePath = opts.state ?? defaultDecisionEventsStateFile();
  const state = await loadSyncState(statePath);

  if (opts.dryRun) {
    for (const leg of legs) {
      console.log(`sync-decision-events: dry-run leg=${leg} pending=${pendingForLeg(events, state, leg).length}`);
    }
    console.log(`sync-decision-events: dry-run events=${events.length} badLines=${badLines}`);
    return 0;
  }

  const tursoToken = process.env.DECISION_EVENTS_TURSO_TOKEN?.trim()
    ? process.env.DECISION_EVENTS_TURSO_TOKEN.trim()
    : await resolveBwsSecret(DECISION_EVENTS_TURSO_TOKEN_NAME);
  const webhookSecret = process.env.DECISION_EVENTS_WEBHOOK_SECRET?.trim()
    ? process.env.DECISION_EVENTS_WEBHOOK_SECRET.trim()
    : legs.includes("webhook")
      ? await resolveBwsSecret(DECISION_EVENTS_WEBHOOK_SECRET_NAME)
      : null;

  const reports = await syncDecisionEvents({
    events,
    state,
    legs,
    tursoUrl: process.env.DECISION_EVENTS_TURSO_URL?.trim() || DECISION_EVENTS_TURSO_URL,
    tursoToken,
    webhookUrl,
    webhookSecret,
  });
  await saveSyncState(statePath, state);

  let failed = false;
  for (const r of reports) {
    if (r.error) {
      failed = true;
      console.log(`sync-decision-events: leg=${r.leg} FAILED pending=${r.pending}: ${r.error}`);
    } else {
      console.log(
        `sync-decision-events: leg=${r.leg} pending=${r.pending} attempted=${r.attempted} inserted=${r.inserted}`,
      );
    }
  }
  if (badLines > 0) console.log(`sync-decision-events: ${badLines} outbox line(s) skipped as invalid (see above)`);
  return failed ? 1 : 0;
}

process.exit(await main());
