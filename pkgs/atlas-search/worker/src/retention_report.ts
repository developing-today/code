/**
 * Retention dry-run reports for retained Atlas objects (#9140 design
 * record, build slice 6), shaped after the retention rule engine's report
 * (#6041 design §6).
 *
 * A report lists, per class, every live version its rule would let go:
 * older than the class window (the age trigger fired) and outside the
 * class's keep-newest floor. Versions the age trigger fired on but the
 * floor kept are counted as protected. In `dry-run` mode every action is
 * `report-only`; in `live-cold` the would-be action is `cold-tier`. No
 * mode deletes. Reports name versions by object id and version only, never
 * payload content, and record the rule set they ran under.
 *
 * A class moves from `dry-run` to `live-cold` only through `promoteClass`,
 * and only after it appears unchanged in the two latest scheduled reports
 * (rule engine §5). On-demand reports never count toward promotion.
 */
import { CLASS_NAMES, RetainedObjectStore, type Family, type RetentionClassName } from "./retained_object";

export const ENGINE_VERSION = "atlas-retained-report-v1";

export type Mode = "dry-run" | "live-cold";
export type ReportKind = "scheduled" | "on-demand";

export class RetentionReportError extends Error {
  constructor(
    readonly code: "unknown_retention_class" | "not_earned" | "duplicate_run",
    message: string,
  ) {
    super(message);
    this.name = "RetentionReportError";
  }
}

export interface RuleRecord {
  class: RetentionClassName;
  window_ms: number;
  keep_newest: number;
  enabled: boolean;
  mode: Mode;
  promoted_from: string[];
}

export interface CandidateVersion {
  object_id: string;
  version: number;
  family: Family;
  bytes: number;
  triggers: ["age"];
  action: "report-only" | "cold-tier";
}

export interface ClassSection {
  rule: RuleRecord;
  candidates: { count: number; bytes: number; would_cold_tier: number; would_delete: 0 };
  triggers: { age: { count: number; bytes: number } };
  protected: { floor: number };
  versions: CandidateVersion[];
}

export interface RetentionReport {
  run_id: string;
  kind: ReportKind;
  at: string;
  engine: string;
  rule_set: RuleRecord[];
  classes: ClassSection[];
  deltas: {
    previous_run_id: string | null;
    newly_eligible: string[];
    no_longer_eligible: string[];
    mode_changes: Array<{ class: RetentionClassName; from: Mode; to: Mode }>;
  };
}

interface RuleRow {
  class: RetentionClassName;
  window_ms: number;
  keep_newest: number;
  enabled: number;
  mode: Mode;
  promoted_from: string | null;
}

interface VersionRow {
  object_id: string;
  version: number;
  family: Family;
  class: RetentionClassName;
  written_at_ms: number;
  bytes: number;
  newest: number;
}

async function rules(db: D1Database): Promise<RuleRecord[]> {
  const { results } = await db
    .prepare(
      "SELECT class, window_ms, keep_newest, enabled, mode, promoted_from FROM atlas_retention_classes ORDER BY class",
    )
    .all<RuleRow>();
  return results.map((row) => ({
    class: row.class,
    window_ms: row.window_ms,
    keep_newest: row.keep_newest,
    enabled: row.enabled === 1,
    mode: row.mode,
    promoted_from: row.promoted_from ? (JSON.parse(row.promoted_from) as string[]) : [],
  }));
}

async function previousReport(db: D1Database, kind: ReportKind, limit: number): Promise<RetentionReport[]> {
  const { results } = await db
    .prepare("SELECT report FROM atlas_retention_reports WHERE kind = ? ORDER BY at_ms DESC, run_id DESC LIMIT ?")
    .bind(kind, limit)
    .all<{ report: string }>();
  return results.map((row) => JSON.parse(row.report) as RetentionReport);
}

function key(version: { object_id: string; version: number }): string {
  return `${version.object_id}@${version.version}`;
}

/**
 * Evaluate every class against the live versions and store the report.
 * Changes nothing but the report table.
 */
export async function runRetentionReport(
  db: D1Database,
  runId: string,
  kind: ReportKind,
  now: () => number = Date.now,
): Promise<RetentionReport> {
  const at = now();
  const ruleSet = await rules(db);
  const { results } = await db
    .prepare(
      `SELECT o.object_id, o.version, o.family, o.class, o.written_at_ms, length(o.payload) AS bytes, n.newest
         FROM atlas_retained_objects AS o
         JOIN (SELECT object_id, MAX(version) AS newest FROM atlas_retained_objects GROUP BY object_id) AS n
           ON n.object_id = o.object_id
        WHERE o.retired_at_ms IS NULL
        ORDER BY o.object_id, o.version`,
    )
    .all<VersionRow>();
  const classes: ClassSection[] = ruleSet.map((rule) => {
    const section: ClassSection = {
      rule,
      candidates: { count: 0, bytes: 0, would_cold_tier: 0, would_delete: 0 },
      triggers: { age: { count: 0, bytes: 0 } },
      protected: { floor: 0 },
      versions: [],
    };
    for (const row of results) {
      if (row.class !== rule.class) continue;
      if (row.written_at_ms > at - rule.window_ms) continue;
      section.triggers.age.count += 1;
      section.triggers.age.bytes += row.bytes;
      if (row.version > row.newest - rule.keep_newest) {
        section.protected.floor += 1;
        continue;
      }
      section.candidates.count += 1;
      section.candidates.bytes += row.bytes;
      section.candidates.would_cold_tier += 1;
      section.versions.push({
        object_id: row.object_id,
        version: row.version,
        family: row.family,
        bytes: row.bytes,
        triggers: ["age"],
        action: rule.mode === "live-cold" ? "cold-tier" : "report-only",
      });
    }
    return section;
  });

  const [previous] = await previousReport(db, kind, 1);
  const before = new Set(previous?.classes.flatMap((section) => section.versions.map(key)) ?? []);
  const after = new Set(classes.flatMap((section) => section.versions.map(key)));
  const previousModes = new Map(previous?.rule_set.map((rule) => [rule.class, rule.mode]) ?? []);
  const report: RetentionReport = {
    run_id: runId,
    kind,
    at: new Date(at).toISOString(),
    engine: ENGINE_VERSION,
    rule_set: ruleSet,
    classes,
    deltas: {
      previous_run_id: previous?.run_id ?? null,
      newly_eligible: [...after].filter((entry) => !before.has(entry)),
      no_longer_eligible: [...before].filter((entry) => !after.has(entry)),
      mode_changes: ruleSet
        .filter((rule) => previousModes.has(rule.class) && previousModes.get(rule.class) !== rule.mode)
        .map((rule) => ({ class: rule.class, from: previousModes.get(rule.class)!, to: rule.mode })),
    },
  };
  const stored = await db
    .prepare(
      "INSERT INTO atlas_retention_reports (run_id, kind, at_ms, report) VALUES (?, ?, ?, ?) ON CONFLICT (run_id) DO NOTHING RETURNING run_id",
    )
    .bind(runId, kind, at, JSON.stringify(report))
    .first<{ run_id: string }>();
  if (!stored) throw new RetentionReportError("duplicate_run", `report ${runId} already exists`);
  return report;
}

function sameRule(a: RuleRecord, b: RuleRecord): boolean {
  return a.window_ms === b.window_ms && a.keep_newest === b.keep_newest && a.mode === b.mode;
}

/**
 * Move one class from `dry-run` to `live-cold`, only when it appears
 * unchanged in the two latest scheduled reports. Returns the run ids that
 * earned it.
 */
export async function promoteClass(db: D1Database, name: string): Promise<string[]> {
  if (!(CLASS_NAMES as readonly string[]).includes(name)) {
    throw new RetentionReportError("unknown_retention_class", `no retention class ${name}`);
  }
  const current = (await rules(db)).find((rule) => rule.class === name);
  if (!current) throw new RetentionReportError("unknown_retention_class", `no retention class ${name}`);
  if (current.mode === "live-cold") return current.promoted_from;
  const reports = await previousReport(db, "scheduled", 2);
  const appearances = reports.map((report) => report.rule_set.find((rule) => rule.class === name));
  if (reports.length < 2 || appearances.some((rule) => !rule || !sameRule(rule, current))) {
    throw new RetentionReportError(
      "not_earned",
      `class ${name} has not appeared unchanged in two consecutive scheduled reports`,
    );
  }
  const earned = reports.map((report) => report.run_id).reverse();
  const updated = await db
    .prepare(
      "UPDATE atlas_retention_classes SET mode = 'live-cold', promoted_from = ? WHERE class = ? AND mode = 'dry-run' AND window_ms = ? AND keep_newest = ? RETURNING class",
    )
    .bind(JSON.stringify(earned), name, current.window_ms, current.keep_newest)
    .first<{ class: string }>();
  if (!updated) throw new RetentionReportError("not_earned", `class ${name} changed while it was being promoted`);
  return earned;
}

/**
 * The most objects one scheduled cycle retires in. Each object is one D1
 * write, so a cycle stays well inside a Worker invocation's query budget;
 * the rest are retired by the next cycle, whose report still lists them.
 */
export const MAX_RETIRE_OBJECTS_PER_CYCLE = 200;

/** What one scheduled retention cycle did (#9140 slice 7). */
export interface ScheduledCycle {
  run_id: string;
  /** False when a cycle for the same scheduled time already stored its report. */
  ran: boolean;
  /** Versions retired, as `object_id@version`, in `live-cold` classes only. */
  retired: string[];
  /** Objects with candidates left for the next cycle by the per-cycle cap. */
  deferred: number;
}

/** The report run id of the cycle the cron scheduled at `scheduledTime`. */
export function scheduledRunId(scheduledTime: number): string {
  return `scheduled-${new Date(scheduledTime).toISOString()}`;
}

/**
 * One cron cycle: store the scheduled report (the reports promotion counts),
 * then retire what it lists in classes an admin has promoted to `live-cold`.
 * A `dry-run` class is reported and never touched. Retiring marks versions
 * cold; nothing deletes. A second delivery of the same cron time finds its
 * report already stored and does nothing.
 */
export async function runScheduledCycle(
  db: D1Database,
  scheduledTime: number,
  now: () => number = Date.now,
): Promise<ScheduledCycle> {
  const runId = scheduledRunId(scheduledTime);
  let report: RetentionReport;
  try {
    report = await runRetentionReport(db, runId, "scheduled", now);
  } catch (error) {
    if (error instanceof RetentionReportError && error.code === "duplicate_run") {
      return { run_id: runId, ran: false, retired: [], deferred: 0 };
    }
    throw error;
  }
  const objects = [
    ...new Set(
      report.classes
        .filter((section) => section.rule.mode === "live-cold")
        .flatMap((section) => section.versions.map((version) => version.object_id)),
    ),
  ].sort();
  const store = new RetainedObjectStore(db, now);
  const retired: string[] = [];
  for (const objectId of objects.slice(0, MAX_RETIRE_OBJECTS_PER_CYCLE)) {
    for (const version of await store.retireExpired(objectId)) retired.push(`${objectId}@${version}`);
  }
  return {
    run_id: runId,
    ran: true,
    retired,
    deferred: Math.max(0, objects.length - MAX_RETIRE_OBJECTS_PER_CYCLE),
  };
}
