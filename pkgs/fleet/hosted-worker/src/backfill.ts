// Backfill runs + the body-vs-DB divergence report (#14312;
// the #9074 remainder the AUDIT-7 adjudication split out).
//
// The ingestion sweep (#12590) already walks a whole repo scope
// through the one check, but it recorded nothing: an operator
// could not answer "what did the last backfill do, and what did
// it fail on?" after the response was gone. A backfill is that
// same sweep with a recorded run — the run record (scope, the
// check's counts in/out, every per-item failure named) is stored
// behind the BackfillRunStore seam as the sweep completes, so
// re-running is auditable and the idempotency acceptance is a
// fact read back, not a claim: a second run over an unchanged
// store records 0 inserted / 0 refreshed because the check
// itself converges (insert/refresh keyed on the item).
//
// The divergence report is the aggregate counterpart of the
// query door's per-result §15 finding (src/query.ts): over a
// whole repo scope it compares every GitHub item's current body
// block against the metadata DB and names three classes —
//   missing_row: the item exists on GitHub, the DB has no row
//     (ingestion has not seen it, or its create-time insert was
//     never healed);
//   stale_field: a row exists but the body's block differs —
//     the fields are the same top-level keys divergenceFields
//     names for one result, with a removed or unparseable block
//     reported on the synthetic field "pm_meta_block", exactly
//     as the query door reports it;
//   orphan_row: a row exists whose item is absent from GitHub
//     (deleted or transferred away) — visible only through the
//     store's cross-tenancy repo seam (MetadataStore.listRepo),
//     never through a per-tenancy list.
// An item whose row agrees counts as agreeing. The report is a
// read: it heals nothing and overwrites nothing — the backfill
// (the sweep) is the heal, and §15's rule stands that a
// divergence is reported, never silently overwritten.

import type { AuthDeps } from "./auth.ts";
import { FailClosed } from "./errors.ts";
import { ingestionDepsFrom, runIngestionSweep, type IngestionDeps } from "./ingestion.ts";
import { parseMetadataBlock } from "./metadata.ts";
import { divergenceFields } from "./query.ts";
import type { MetadataRecord } from "./stamping.ts";

/** One failed item of a backfill run: the check's per-item
 * refusal, named by item so the failure is actionable without
 * re-running anything. */
export interface BackfillFailure {
  repo: string;
  kind: "issue" | "pr";
  number: number;
  url: string;
  /** The refusal's stable code (e.g. "invalid_metadata_block"). */
  error: string;
  message: string;
}

export interface BackfillRunRecord {
  id: string;
  /** The repo scope the run walked (owner/name), in request order. */
  scope: string[];
  startedAt: string;
  finishedAt: string;
  counts: {
    checked: number;
    inserted: number;
    refreshed: number;
    unchanged: number;
    errors: number;
  };
  failures: BackfillFailure[];
}

export interface BackfillRunStore {
  record(run: BackfillRunRecord): Promise<void>;
  /** Every recorded run, oldest first. */
  list(): Promise<BackfillRunRecord[]>;
  get(id: string): Promise<BackfillRunRecord | null>;
}

function cloneRun(run: BackfillRunRecord): BackfillRunRecord {
  return JSON.parse(JSON.stringify(run)) as BackfillRunRecord;
}

/** Fixture backfill run store: an in-memory list with the other
 * seams' posture — records are cloned in and out so callers
 * cannot mutate a stored run. The production home is #9074's
 * metadata DB, behind this interface. */
export class FixtureBackfillRunStore implements BackfillRunStore {
  private readonly runs: BackfillRunRecord[] = [];

  async record(run: BackfillRunRecord): Promise<void> {
    this.runs.push(cloneRun(run));
  }

  async list(): Promise<BackfillRunRecord[]> {
    return this.runs.map(cloneRun);
  }

  async get(id: string): Promise<BackfillRunRecord | null> {
    const run = this.runs.find((r) => r.id === id);
    return run ? cloneRun(run) : null;
  }
}

export interface BackfillDeps {
  ingestion: IngestionDeps;
  runs: BackfillRunStore;
}

/** The run store alone, failing closed with the doors' 503 idiom:
 * a deployment that records no runs cannot list them either. */
export function backfillRunStoreFrom(deps: AuthDeps): BackfillRunStore {
  if (!deps.backfillRunStore) {
    throw new FailClosed(
      503,
      "backfill_run_store_not_configured",
      "this deployment has no backfill run store configured; failing closed",
    );
  }
  return deps.backfillRunStore;
}

/** Resolve the backfill's deps from the door's: the ingestion
 * engine's (which 503 on the store / item source the sweep
 * cannot run without) plus the run store. */
export function backfillDepsFrom(deps: AuthDeps): BackfillDeps {
  return { ingestion: ingestionDepsFrom(deps), runs: backfillRunStoreFrom(deps) };
}

/** Run one backfill over the scope and record it. The sweep is
 * the check's own whole-scope pass, so a backfill and a sweep
 * over the same item state answer identically item by item; the
 * run record is the only addition. A repo that cannot be listed
 * at all is a transport failure and surfaces (502) before any
 * record is written — a partial walk is never recorded as a
 * completed run. */
export async function runMetadataBackfill(deps: BackfillDeps, repos: string[]): Promise<BackfillRunRecord> {
  const startedAt = new Date(deps.ingestion.now()).toISOString();
  const sweep = await runIngestionSweep(deps.ingestion, repos);
  const finishedAt = new Date(deps.ingestion.now()).toISOString();
  const record: BackfillRunRecord = {
    id: `backfill-${crypto.randomUUID()}`,
    scope: [...repos],
    startedAt,
    finishedAt,
    counts: {
      checked: sweep.counts.checked,
      inserted: sweep.counts.inserted,
      refreshed: sweep.counts.refreshed,
      unchanged: sweep.counts.unchanged,
      errors: sweep.counts.errors,
    },
    failures: sweep.results
      .filter((r) => r.outcome === "error")
      .map((r) => ({
        repo: r.repo,
        kind: r.kind,
        number: r.number,
        url: r.url,
        error: r.error ?? "internal",
        message: r.message ?? "ingestion check failed",
      })),
  };
  await deps.runs.record(record);
  return record;
}

// --- The divergence report ---------------------------------------

export type DivergenceFinding =
  | {
      class: "missing_row";
      repo: string;
      kind: "issue" | "pr";
      number: number;
      url: string;
    }
  | {
      class: "stale_field";
      repo: string;
      kind: "issue" | "pr";
      number: number;
      url: string;
      /** The top-level Metadata keys that differ (divergenceFields),
       * or ["pm_meta_block"] when the body's block was removed or
       * no longer parses — the query door's §15 idiom. */
      fields: string[];
    }
  | {
      class: "orphan_row";
      repo: string;
      kind: "issue" | "pr";
      number: number;
      url: string;
      /** The tenancy the orphaned row is stored under. */
      binding_id: string;
    };

export interface DivergenceReport {
  repos: string[];
  generatedAt: string;
  counts: {
    /** GitHub items compared. */
    items: number;
    /** DB rows in scope. */
    rows: number;
    /** Items whose row exists and agrees with the body block. */
    agreeing: number;
    missing_row: number;
    stale_field: number;
    orphan_row: number;
  };
  findings: DivergenceFinding[];
}

function itemKey(repo: string, kind: string, number: number): string {
  return `${repo}#${kind}#${number}`;
}

/** Compare a repo scope's GitHub items against the metadata DB
 * and name every divergence (see the header for the classes).
 * Rows are read through the store's repo seam: orphan rows and
 * unattributed rows are exactly what a per-tenancy list cannot
 * show, and this report exists to show them. A repo that cannot
 * be listed surfaces as the item source's failure, like the
 * sweep's. */
export async function runDivergenceReport(deps: IngestionDeps, repos: string[]): Promise<DivergenceReport> {
  const findings: DivergenceFinding[] = [];
  let items = 0;
  let rows = 0;
  let agreeing = 0;
  for (const repo of repos) {
    const repoItems = await deps.items.listItems(repo);
    const repoRows = await deps.store.listRepo(repo);
    items += repoItems.length;
    rows += repoRows.length;
    const rowByKey = new Map(repoRows.map((r) => [itemKey(r.repo, r.kind, r.number), r]));
    const itemKeys = new Set<string>();
    for (const item of repoItems) {
      const key = itemKey(item.repo, item.kind, item.number);
      itemKeys.add(key);
      const row: MetadataRecord | undefined = rowByKey.get(key);
      if (!row) {
        findings.push({ class: "missing_row", repo: item.repo, kind: item.kind, number: item.number, url: item.url });
        continue;
      }
      let bodyBlock: Record<string, unknown> | null = null;
      try {
        bodyBlock = parseMetadataBlock(item.body).block;
      } catch {
        bodyBlock = null; // a block that no longer parses is a finding, below
      }
      if (bodyBlock === null) {
        findings.push({
          class: "stale_field",
          repo: item.repo,
          kind: item.kind,
          number: item.number,
          url: item.url,
          fields: ["pm_meta_block"],
        });
        continue;
      }
      const fields = divergenceFields(row.metadata, bodyBlock);
      if (fields.length > 0) {
        findings.push({
          class: "stale_field",
          repo: item.repo,
          kind: item.kind,
          number: item.number,
          url: item.url,
          fields,
        });
      } else {
        agreeing += 1;
      }
    }
    for (const row of repoRows) {
      if (itemKeys.has(itemKey(row.repo, row.kind, row.number))) continue;
      findings.push({
        class: "orphan_row",
        repo: row.repo,
        kind: row.kind,
        number: row.number,
        url: row.url,
        binding_id: row.bindingId,
      });
    }
  }
  findings.sort(
    (a, b) =>
      a.repo.localeCompare(b.repo) ||
      a.kind.localeCompare(b.kind) ||
      a.number - b.number ||
      a.class.localeCompare(b.class),
  );
  return {
    repos: [...repos],
    generatedAt: new Date(deps.now()).toISOString(),
    counts: {
      items,
      rows,
      agreeing,
      missing_row: findings.filter((f) => f.class === "missing_row").length,
      stale_field: findings.filter((f) => f.class === "stale_field").length,
      orphan_row: findings.filter((f) => f.class === "orphan_row").length,
    },
    findings,
  };
}
