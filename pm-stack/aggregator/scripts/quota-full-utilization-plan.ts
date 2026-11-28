#!/usr/bin/env bun
// Full-quota utilization check (#2173): the 4–6h check that keeps a
// standing burn plan for every provider pool, recomputes it from fresh
// readings + recorded burn history, diffs it against the standing plan,
// and either confirms on track or escalates the plan's dated audible.
//
// Integration: this is the check body pool-quota-steward runs — it is
// NOT a new schedule. The steward stays hourly; this script self-throttles
// to the 4–6h cadence on its own recorded check log, so the job's
// cadence does not change (cadence changes are Drew's call; his 4–6h
// direction in #2173 covers this check specifically).
//
// What a run does:
//   1. Reads the quota ledger (Turso fleet-platform.agent_quota_records,
//      source "opencode-quota" — the gatherer's 15-minute tick) for the
//      lookback window. The newest row is the fresh reading; the series
//      is the burn history. Nothing is assumed.
//   2. Builds the plan: per pool, the largest window, current %, reset
//      instant, measured rate (Theil–Sen over the current segment), the
//      rate that lands ~100% at reset, the landing at reset and in the
//      period after next, the cascade position, the spill to the next
//      provider, and a dated audible.
//   3. Diffs against the standing plan (docs/full-utilization-plan.json):
//      verdict changes, landing drift, new escalations, recoveries.
//   4. Writes the standing plan (md + json, atomically) and one add-only
//      check record under docs/quota-checks/.
//
// Modes:
//   --check            (default) due-guard → recompute → diff → record.
//   --write-plan       recompute and write the standing plan only.
//   --replay           run the check at successive past instants over the
//                      recorded series (--replay-hours, --step-hours), so
//                      a cadence of real checks is on record from history
//                      that predates the job change.
//
// Usage:
//   bun run scripts/quota-full-utilization-plan.ts --check
//   bun run scripts/quota-full-utilization-plan.ts --write-plan
//   bun run scripts/quota-full-utilization-plan.ts --replay --replay-hours 24 --step-hours 6
//
// The token comes from TURSO_QUOTA_TOKEN when set, otherwise from
// Bitwarden Secrets Manager by name (turso.fleet-platform-rw-token, the
// same secret gather-quota.ts uses). --from-file reads decoded ledger
// rows from JSONL instead of Turso (offline tests / dry runs).

import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { resolveBwsSecret } from "../src/secrets.ts";
import {
  DEFAULT_METHOD,
  type CheckRecord,
  type FullUtilizationPlan,
  type PlanFile,
  type RawSample,
  type RawWindow,
  buildPlan,
  checkDue,
  diffPlans,
  renderPlanMarkdown,
} from "../src/quota-plan.ts";

const TURSO_URL =
  process.env.TURSO_FLEET_PLATFORM_URL ?? "https://fleet-platform-dezren39.aws-us-east-1.turso.io";
const TURSO_TOKEN_NAME = "turso.fleet-platform-rw-token";
const GATHERER_SOURCE = "opencode-quota";

const DOCS_DIR = resolve(import.meta.dir, "..", "docs");
const PLAN_MD = "full-utilization-plan.md";
const PLAN_JSON = "full-utilization-plan.json";
const CHECKS_DIR = "quota-checks";

interface Args {
  mode: "check" | "write-plan" | "replay";
  asOf: string | null;
  lookbackDays: number;
  planDir: string;
  fromFile: string | null;
  minIntervalHours: number;
  force: boolean;
  replayHours: number;
  stepHours: number;
  json: boolean;
  failOnEscalate: boolean;
  noWritePlan: boolean;
}

function parseArgs(argv: string[]): Args {
  const args: Args = {
    mode: "check",
    asOf: null,
    lookbackDays: 14,
    planDir: DOCS_DIR,
    fromFile: null,
    minIntervalHours: DEFAULT_METHOD.checkIntervalHours,
    force: false,
    replayHours: 24,
    stepHours: 6,
    json: false,
    failOnEscalate: false,
    noWritePlan: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => {
      const v = argv[++i];
      if (v === undefined) throw new Error(`${a} needs a value`);
      return v;
    };
    switch (a) {
      case "--check":
        args.mode = "check";
        break;
      case "--write-plan":
        args.mode = "write-plan";
        break;
      case "--replay":
        args.mode = "replay";
        break;
      case "--as-of":
        args.asOf = next();
        break;
      case "--lookback-days":
        args.lookbackDays = Number(next());
        break;
      case "--plan-dir":
        args.planDir = resolve(next());
        break;
      case "--from-file":
        args.fromFile = resolve(next());
        break;
      case "--min-interval-hours":
        args.minIntervalHours = Number(next());
        break;
      case "--replay-hours":
        args.replayHours = Number(next());
        break;
      case "--step-hours":
        args.stepHours = Number(next());
        break;
      case "--force":
        args.force = true;
        break;
      case "--json":
        args.json = true;
        break;
      case "--fail-on-escalate":
        args.failOnEscalate = true;
        break;
      case "--no-write-plan":
        args.noWritePlan = true;
        break;
      case "--help":
      case "-h":
        console.log(
          [
            "quota-full-utilization-plan — standing full-quota plan + 4–6h check (#2173)",
            "",
            "  --check                 run the check (default)",
            "  --write-plan            regenerate the standing plan only",
            "  --replay                run consecutive past checks from the recorded series",
            "  --as-of ISO             project as of this instant",
            "  --lookback-days N       ledger rows to read (default 14)",
            "  --plan-dir DIR          where the plan + check records live",
            "  --from-file FILE        decoded ledger rows (JSONL) instead of Turso",
            "  --min-interval-hours N  check due-guard (default 6)",
            "  --replay-hours N        replay span (default 24)",
            "  --step-hours N          replay/check step (default 6)",
            "  --force                 run the check even when not due",
            "  --no-write-plan         do not rewrite the standing plan in check mode",
            "  --json                  machine-readable result on stdout",
            "  --fail-on-escalate      exit 2 when the check escalates (CI use only)",
          ].join("\n"),
        );
        process.exit(0);
        break;
      default:
        throw new Error(`unrecognized argument: ${a}`);
    }
  }
  return args;
}

function sqlString(v: string): string {
  return `'${v.replace(/'/g, "''")}'`;
}

/**
 * The Turso libsql HTTP pipeline, exactly as gather-quota.ts's sink uses
 * it. Returns typed cells as plain JS values.
 */
async function tursoRows(sql: string, token: string, url: string): Promise<Record<string, unknown>[]> {
  const res = await fetch(`${url.replace(/\/$/, "")}/v2/pipeline`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ requests: [{ type: "execute", stmt: { sql } }, { type: "close" }] }),
  });
  if (!res.ok) throw new Error(`turso pipeline HTTP ${res.status}`);
  const body = (await res.json()) as {
    results?: { type: string; error?: { message?: string }; response?: { result?: { cols: { name: string }[]; rows: unknown[][] } } }[];
  };
  const first = body.results?.[0];
  if (!first || first.type === "error") throw new Error(`turso pipeline error: ${first?.error?.message ?? "empty result"}`);
  const result = first.response?.result;
  if (!result) return [];
  const cols = result.cols.map((c) => c.name);
  const value = (cell: unknown): unknown => {
    const c = cell as { type?: string; value?: unknown };
    if (!c || typeof c !== "object") return cell;
    if (c.type === "null") return null;
    if (c.type === "integer") return Number(c.value);
    if (c.type === "float") return Number(c.value);
    return c.value ?? null;
  };
  return result.rows.map((row) => Object.fromEntries(cols.map((c, i) => [c, value(row[i])])));
}

/** Decode one ledger row's payload into a RawSample. */
function decodeRow(row: { payload?: unknown; ts?: unknown }): RawSample | null {
  let payload: Record<string, unknown> | null = null;
  if (typeof row.payload === "string") {
    try {
      payload = JSON.parse(row.payload) as Record<string, unknown>;
    } catch {
      return null;
    }
  } else if (row.payload && typeof row.payload === "object") {
    payload = row.payload as Record<string, unknown>;
  }
  if (!payload) return null;
  const pool = typeof payload.id === "string" ? payload.id : null;
  const ts = typeof payload.ts === "string" ? payload.ts : typeof row.ts === "string" ? row.ts : null;
  if (!pool || !ts) return null;
  const windows: RawWindow[] = Array.isArray(payload.windows)
    ? (payload.windows as RawWindow[]).filter((w) => w && typeof w.name === "string")
    : [];
  return {
    pool,
    provider: typeof payload.provider === "string" ? payload.provider : pool,
    plan: typeof payload.plan === "string" ? payload.plan : null,
    ts,
    windows,
  };
}

async function readLedger(args: Args, asOfMs: number): Promise<{ samples: RawSample[]; latestAt: string | null }> {
  if (args.fromFile) {
    const lines = readFileSync(args.fromFile, "utf8").split("\n").filter((l) => l.trim());
    const samples = lines
      .map((l) => {
        try {
          return decodeRow(JSON.parse(l) as { payload?: unknown; ts?: unknown });
        } catch {
          return null;
        }
      })
      .filter((s): s is RawSample => s !== null)
      .filter((s) => Date.parse(s.ts) <= asOfMs);
    const latestAt = samples.reduce<string | null>((max, s) => (!max || s.ts > max ? s.ts : max), null);
    return { samples, latestAt };
  }

  const token = process.env.TURSO_QUOTA_TOKEN ?? (await resolveBwsSecret(TURSO_TOKEN_NAME));
  if (!token) throw new Error(`quota token unresolved (set TURSO_QUOTA_TOKEN or make ${TURSO_TOKEN_NAME} readable)`);
  const since = new Date(asOfMs - args.lookbackDays * 86_400_000).toISOString();
  const rows = await tursoRows(
    `select id, ts, payload from agent_quota_records where source=${sqlString(GATHERER_SOURCE)} ` +
      `and ts >= ${sqlString(since)} order by ts asc`,
    token,
    process.env.TURSO_FLEET_PLATFORM_URL ?? TURSO_URL,
  );
  const samples = rows
    .map((r) => decodeRow(r as { payload?: unknown; ts?: unknown }))
    .filter((s): s is RawSample => s !== null)
    .filter((s) => Date.parse(s.ts) <= asOfMs);
  const latestAt = samples.reduce<string | null>((max, s) => (!max || s.ts > max ? s.ts : max), null);
  return { samples, latestAt };
}

function readJson<T>(path: string): T | null {
  if (!existsSync(path)) return null;
  try {
    return JSON.parse(readFileSync(path, "utf8")) as T;
  } catch {
    return null;
  }
}

function writeAtomic(path: string, contents: string): void {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, contents);
  renameSync(tmp, path);
}

function planMarkdown(plan: FullUtilizationPlan): string {
  return [
    "# Full-quota utilization plan",
    "",
    "<!-- Generated by pm-stack/aggregator/scripts/quota-full-utilization-plan.ts (#2173).",
    "     Regenerated each check from the recorded quota ledger; do not hand-edit the tables.",
    "     The dated audibles below are the plan's standing instructions, not a provider config change. -->",
    "",
    renderPlanMarkdown(plan, { headingLevel: 2 }),
  ].join("\n");
}

function newestCheck(dir: string): { at: string; path: string } | null {
  const checksDir = join(dir, CHECKS_DIR);
  if (!existsSync(checksDir)) return null;
  const files = readdirSync(checksDir).filter((f) => f.endsWith(".json")).sort();
  if (!files.length) return null;
  const path = join(checksDir, files[files.length - 1]);
  const record = readJson<CheckRecord>(path);
  return record ? { at: record.checkAt, path } : null;
}

function checkFileName(iso: string): string {
  return `${iso.replace(/\./g, "-").replace(/:/g, "-")}.json`;
}

function writePlan(dir: string, plan: FullUtilizationPlan): void {
  writeAtomic(join(dir, PLAN_MD), planMarkdown(plan));
  writeAtomic(join(dir, PLAN_JSON), JSON.stringify(plan, null, 2) + "\n");
}

function recordCheck(dir: string, record: CheckRecord): string {
  const path = join(dir, CHECKS_DIR, checkFileName(record.checkAt));
  writeAtomic(path, JSON.stringify(record, null, 2) + "\n");
  return path;
}

function summarize(plan: FullUtilizationPlan): string {
  const s = plan.summary;
  return (
    `${s.underPace} under pace · ${s.overPace} over pace · ${s.exhausted} fully used · ` +
    `${s.onTrack} on track · ${s.stale} stale · ${s.noReset} no reset · ${s.noData} no data`
  );
}

async function runOnce(args: Args, asOfIso: string, previous: PlanFile | null, mode: "live" | "replay" = "live"): Promise<{
  plan: FullUtilizationPlan;
  diff: ReturnType<typeof diffPlans>;
  record: CheckRecord;
  latestAt: string | null;
}> {
  const asOfMs = Date.parse(asOfIso);
  const { samples, latestAt } = await readLedger(args, asOfMs);
  if (!samples.length) throw new Error("no ledger samples in the lookback window");
  const plan = buildPlan(samples, asOfIso, { generatedAt: new Date().toISOString() });

  const ledgerAgeMinutes = latestAt ? Math.round((asOfMs - Date.parse(latestAt)) / 60_000) : null;
  if (ledgerAgeMinutes !== null && ledgerAgeMinutes > 30) {
    plan.dataQuality.push(
      `ledger lag: newest reading is ${ledgerAgeMinutes} min old at the check instant — readings are not fresh`,
    );
  } else if (latestAt) {
    plan.dataQuality.push(`ledger fresh: newest reading ${ledgerAgeMinutes} min before the check instant`);
  }

  const diff = diffPlans(previous, plan);
  const actionable = plan.pools.filter((p) => p.audible && p.audible.action !== "confirm");
  const escalates = diff.escalations.length > 0 || plan.summary.underPace > 0 || plan.summary.overPace > 0;
  const repairs = plan.summary.stale + plan.summary.noReset + plan.summary.noData;
  const verdict: CheckRecord["verdict"] = escalates ? "escalate" : repairs > 0 ? "repair" : "confirm";

  const record: CheckRecord = {
    version: 1,
    mode,
    checkAt: new Date(asOfMs).toISOString(),
    asOf: plan.asOf,
    planGeneratedAt: plan.generatedAt,
    verdict,
    summary: plan.summary,
    escalations: diff.escalations,
    verdictChanges: diff.verdictChanges,
    landingDrift: diff.landingDrift,
    recovered: diff.recovered,
    audibleActions: actionable.map((p) => ({ pool: p.pool, at: p.audible!.at, action: p.audible!.action, detail: p.audible!.detail })),
    firstRun: diff.firstRun,
    added: diff.added,
    removed: diff.removed,
  };
  return { plan, diff, record, latestAt };
}

function printRun(plan: FullUtilizationPlan, diff: ReturnType<typeof diffPlans>, args: Args, recordPath: string | null): void {
  console.log(`full-quota check — as of ${plan.asOf}`);
  console.log(`  ${summarize(plan)}`);
  if (diff.firstRun) console.log("  first run: no standing plan to diff against");
  for (const e of diff.escalations) console.log(`  ESCALATE ${e.pool}: ${e.verdict} — ${e.detail}`);
  for (const c of diff.verdictChanges) console.log(`  changed ${c.pool}: ${c.from} → ${c.to}`);
  for (const d of diff.landingDrift) {
    console.log(`  drift ${d.pool}: landing ${d.from ?? "—"}% → ${d.to ?? "—"}% (${d.delta > 0 ? "+" : ""}${d.delta})`);
  }
  if (diff.recovered.length) console.log(`  recovered: ${diff.recovered.join(", ")}`);
  const actionable = plan.pools.filter((p) => p.audible && p.audible.action !== "confirm");
  for (const p of actionable) console.log(`  AUDIBLE ${p.audible!.at} ${p.audible!.action} ${p.pool}: ${p.audible!.detail}`);
  if (recordPath) console.log(`  check record: ${recordPath}`);
  if (args.mode !== "write-plan") {
    console.log(`  standing plan: ${join(args.planDir, PLAN_MD)}`);
  }
}

async function main(): Promise<number> {
  const args = parseArgs(process.argv.slice(2));
  const planPath = join(args.planDir, PLAN_JSON);

  if (args.mode === "write-plan") {
    const asOf = args.asOf ?? new Date().toISOString();
    const { plan } = await runOnce(args, asOf, null);
    writePlan(args.planDir, plan);
    if (args.json) console.log(JSON.stringify(plan, null, 2));
    else printRun(plan, diffPlans(null, plan), args, null);
    return 0;
  }

  if (args.mode === "replay") {
    const end = args.asOf ? Date.parse(args.asOf) : Date.now();
    const start = end - args.replayHours * 3_600_000;
    const step = args.stepHours * 3_600_000;
    let previous: PlanFile | null = null;
    const records: CheckRecord[] = [];
    for (let t = start; t <= end; t += step) {
      const { plan, record } = await runOnce(args, new Date(t).toISOString(), previous, "replay");
      const path = recordCheck(args.planDir, record);
      records.push(record);
      previous = plan;
      console.log(
        `${record.asOf}  ${record.verdict.padEnd(8)}  ${summarize(plan)}` +
          (record.escalations.length ? `  escalations: ${record.escalations.map((e) => e.pool).join(", ")}` : ""),
      );
      console.log(`  record: ${path}`);
    }
    if (args.json) console.log(JSON.stringify(records, null, 2));
    const last = records[records.length - 1];
    return args.failOnEscalate && last?.verdict === "escalate" ? 2 : 0;
  }

  // --check
  const last = newestCheck(args.planDir);
  const now = args.asOf ? Date.parse(args.asOf) : Date.now();
  if (!args.force && !args.asOf && !checkDue(last?.at ?? null, now, args.minIntervalHours)) {
    console.log(
      `full-quota check: not due — newest check ${last?.at} is younger than ${args.minIntervalHours}h ` +
        "(4–6h cadence; use --force to run anyway)",
    );
    return 0;
  }

  const previous = readJson<PlanFile>(planPath);
  const asOf = args.asOf ?? new Date().toISOString();
  const { plan, diff, record } = await runOnce(args, asOf, previous);
  if (!args.noWritePlan) writePlan(args.planDir, plan);
  const path = recordCheck(args.planDir, record);
  if (args.json) console.log(JSON.stringify({ plan, diff, record }, null, 2));
  else printRun(plan, diff, args, path);
  return args.failOnEscalate && record.verdict === "escalate" ? 2 : 0;
}

main()
  .then((code) => process.exit(code))
  .catch((err) => {
    console.error(`full-quota check failed: ${err instanceof Error ? err.message : String(err)}`);
    process.exit(1);
  });
