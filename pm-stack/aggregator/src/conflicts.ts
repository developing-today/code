// Source-conflict inbox (#1282, slices 1-2): ONE queue over every
// disagreement the facets already record, instead of a ⚠ badge that only
// exists where it happens to render.
//
// Slice 1 projects; it never resolves. Slice 2 adds per-class trust
// rulings for the Project-vs-derived class only (see
// conflict-rulings.ts): when a ruling stands, that class's entries are
// suppressed from the inbox and renders apply the trusted value instead
// of showing both sides as a clash. Nothing else changes: the quota
// class has no ruling path and is never suppressed.
//
// Honesty rules, same as the rest of the dashboard:
// - Only disagreements a facet ACTUALLY reports become entries. We do not
//   invent a facet adapter (packet scope: "collect what exists").
// - "Age" is measured from the newest timestamp the source itself carries
//   for the pair (the Project item's `updatedAt`, the service's
//   `measuredAt`). That is a FLOOR on how long the values have disagreed —
//   the item may have been edited for another reason yesterday while the
//   clash is months old — so every entry states its ageBasis and the page
//   says "at least", never "exactly".
import type { OrchestrationSection, ProjectPoolItem, RefKey, SaasService } from "./model.ts";
import type { ConflictRuling, TrustedSource } from "./conflict-rulings.ts";
import { derivedProjectState, projectDisagreement } from "./projects.ts";

/** Closed set of disagreement classes this slice knows how to collect. */
export type ConflictClassId = "project-vs-derived" | "quota-records";

export interface ConflictClassDef {
  id: ConflictClassId;
  /** Human label, used as the inbox's grouping heading. */
  label: string;
}

export const CONFLICT_CLASSES: ConflictClassDef[] = [
  { id: "project-vs-derived", label: "Project Status vs derived state" },
  { id: "quota-records", label: "Quota records vs recorded reads" },
];

/** The pair of sources and both of their values — what the queue shows. */
export interface ConflictEntry {
  /** Stable within one projection: `<class>:<subject>`. */
  id: string;
  classId: ConflictClassId;
  classLabel: string;
  /** The item the disagreement is about; ref is null when the facet has none. */
  item: { ref: RefKey | null; title: string; url: string | null };
  /** The two sources, named as the facet names them. */
  sources: { a: string; b: string };
  /** What each source says. */
  values: { a: string; b: string };
  /** When the pair was last on record (source timestamp), or null when undated. */
  observedAt: string | null;
  /** Whole days since observedAt; null when the source carries no date. */
  ageDays: number | null;
  /** Which timestamp ageDays is measured from — always a floor, never an exact age. */
  ageBasis: "project-item-updated-at" | "service-measured-at" | "undated";
  /** Where a human goes to see the underlying records. */
  href: string | null;
}

export interface ConflictCountByClass {
  classId: ConflictClassId;
  label: string;
  count: number;
}

/** The inbox as both /conflicts and /api/conflicts.json serve it. */
export interface ConflictInbox {
  generatedAt: string;
  total: number;
  byClass: ConflictCountByClass[];
  /** Oldest entry's ageDays, or null when no entry carries a date. */
  oldestAgeDays: number | null;
  entries: ConflictEntry[];
  /** Rulings that were applied to build this inbox (slice 2, data not code). */
  rulings: ConflictRuling[];
  /** Entries suppressed by a ruling — same shape, kept for audit, never counted. */
  suppressed: ConflictEntry[];
  /** How many entries each class suppressed (only project-vs-derived can be non-zero). */
  suppressedByClass: ConflictCountByClass[];
}

export interface ConflictInputs {
  /** The org Project pool — the Project-vs-derived facet. */
  pool: ReadonlyArray<ProjectPoolItem>;
  /** The canonical orchestration section the derived state is read against. */
  orchestration?: OrchestrationSection | null;
  /** Refs the dashboard additionally counts as in progress (#1206 convention). */
  additionalInProgressRefs?: ReadonlySet<RefKey>;
  /** Family-2 SaaS services — the only facet with recorded conflict lines today. */
  services?: ReadonlyArray<SaasService>;
  /** Injectable clock (tests); defaults to now. */
  now?: number;
  /** Slice-2 trust rulings (data). Only project-vs-derived is honored. */
  rulings?: ReadonlyArray<ConflictRuling>;
}

const DAY_MS = 86_400_000;

function ageDaysFrom(iso: string | null | undefined, nowMs: number): number | null {
  if (!iso) return null;
  const at = Date.parse(iso);
  if (!Number.isFinite(at)) return null;
  return Math.max(0, Math.floor((nowMs - at) / DAY_MS));
}

function projectEntries(inputs: ConflictInputs, nowMs: number): ConflictEntry[] {
  const out: ConflictEntry[] = [];
  for (const item of inputs.pool) {
    const clash = projectDisagreement(item, inputs.orchestration, inputs.additionalInProgressRefs);
    if (!clash) continue;
    out.push({
      id: `project-vs-derived:${item.ref}`,
      classId: "project-vs-derived",
      classLabel: CONFLICT_CLASSES[0].label,
      item: { ref: item.ref, title: item.title, url: item.issueUrl || item.url || null },
      sources: { a: "GitHub Project Status", b: "dashboard-derived state" },
      values: { a: clash.project, b: clash.derived },
      observedAt: item.updatedAt ?? null,
      ageDays: ageDaysFrom(item.updatedAt, nowMs),
      ageBasis: item.updatedAt ? "project-item-updated-at" : "undated",
      href: item.issueUrl || item.url || null,
    });
  }
  return out;
}

function quotaEntries(inputs: ConflictInputs, nowMs: number): ConflictEntry[] {
  const out: ConflictEntry[] = [];
  for (const svc of inputs.services ?? []) {
    if (!svc.conflicts.length) continue;
    const freshestRead = svc.reads[0]?.text ?? null;
    svc.conflicts.forEach((line, index) => {
      out.push({
        id: `quota-records:${svc.id}:${index}`,
        classId: "quota-records",
        classLabel: CONFLICT_CLASSES[1].label,
        item: { ref: null, title: svc.name, url: `/quota/service/${svc.id}` },
        sources: {
          a: svc.sources?.quotaViewSection?.replace(/^#+\s*/, "") ?? "quota view",
          b: freshestRead ? "freshest dated read" : "dated reads",
        },
        values: { a: line, b: freshestRead ?? "no dated read on record" },
        observedAt: svc.measuredAt,
        ageDays: ageDaysFrom(svc.measuredAt, nowMs),
        ageBasis: svc.measuredAt ? "service-measured-at" : "undated",
        href: `/quota/service/${svc.id}`,
      });
    });
  }
  return out;
}

/**
 * Collect every disagreement the facets already report. Read-only and pure
 * apart from the injected clock: the same inputs always yield the same
 * inbox, which is what lets the page and /api/conflicts.json be one truth.
 * Rulings never change detection — they only suppress at the inbox layer.
 */
export function detectConflicts(inputs: ConflictInputs): ConflictEntry[] {
  const nowMs = inputs.now ?? Date.now();
  return [...projectEntries(inputs, nowMs), ...quotaEntries(inputs, nowMs)];
}

/** The ruling that suppresses project-vs-derived entries, if one stands. */
export function rulingForClass(
  rulings: ReadonlyArray<ConflictRuling> | undefined,
  classId: string,
): ConflictRuling | null {
  if (!rulings) return null;
  return rulings.find((r) => r.classId === classId) ?? null;
}

/**
 * The disagreement to display for one item after applying the ruling.
 * Null means "no clash to show": either the sources agree (slice 1 rule)
 * or a ruling trusts one side (slice 2 rule). Callers that render the
 * effective value use resolveTrustedProjectValue for the display value.
 */
export function effectiveProjectDisagreement(
  item: ProjectPoolItem,
  orchestration?: OrchestrationSection | null,
  additionalInProgressRefs?: ReadonlySet<RefKey>,
  ruling?: ConflictRuling | null,
): { project: string; derived: string } | null {
  const clash = projectDisagreement(item, orchestration, additionalInProgressRefs);
  if (!clash) return null;
  if (ruling && ruling.classId === "project-vs-derived") return null;
  return clash;
}

/**
 * The value renders apply while a project-vs-derived ruling stands: the
 * trusted side's value for an item that currently disagrees, or the
 * agreed value (either side) when the sources already agree. Null when
 * the item carries no Project Status to rule on.
 */
export function resolveTrustedProjectValue(
  item: ProjectPoolItem,
  orchestration?: OrchestrationSection | null,
  additionalInProgressRefs?: ReadonlySet<RefKey>,
  ruling?: ConflictRuling | null,
): string | null {
  if (!item.status) return null;
  if (!ruling || ruling.classId !== "project-vs-derived") return null;
  const trusted: TrustedSource = ruling.trusted;
  if (trusted === "project") return item.status;
  return derivedProjectState(item, orchestration, additionalInProgressRefs);
}

// The inbox summary the health surface counts and /conflicts renders. */
export function conflictInbox(inputs: ConflictInputs): ConflictInbox {
  const nowMs = inputs.now ?? Date.now();
  const all = detectConflicts(inputs);
  const ruling = rulingForClass(inputs.rulings, "project-vs-derived");
  const suppressed = ruling ? all.filter((e) => e.classId === "project-vs-derived") : [];
  const suppressedIds = new Set(suppressed.map((e) => e.id));
  const entries = ruling ? all.filter((e) => !suppressedIds.has(e.id)) : all;
  const byClass = CONFLICT_CLASSES.map((cls) => ({
    classId: cls.id,
    label: cls.label,
    count: entries.filter((e) => e.classId === cls.id).length,
  }));
  const suppressedByClass = CONFLICT_CLASSES.map((cls) => ({
    classId: cls.id,
    label: cls.label,
    count: suppressed.filter((e) => e.classId === cls.id).length,
  }));
  const ages = entries.map((e) => e.ageDays).filter((d): d is number => d !== null);
  return {
    generatedAt: new Date(nowMs).toISOString(),
    total: entries.length,
    byClass,
    oldestAgeDays: ages.length ? Math.max(...ages) : null,
    entries,
    rulings: ruling ? [ruling] : [],
    suppressed,
    suppressedByClass,
  };
}
