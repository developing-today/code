/**
 * Dual-ingestion shared Metadata check (issue #9076; design §16, item 12).
 *
 * Both watchers — the interval sweeper
 * (`scripts/metadata-sweeper.ts`) and the webhook listener (follow-up
 * slice) — run every item through {@link runMetadataCheck}. One
 * implementation, two triggers: the sweeper catches whatever the
 * listener missed because they share this exact function.
 *
 * Item contract:
 * - A body carrying a `<!-- pm-metadata:start --> ... <!-- pm-metadata:end -->`
 *   block (the canonical shape written by `setHistoricalParentMetadata`
 *   in `epic-reparent.ts`) is parsed with `parsePmMetadata` and stored.
 *   Bodies win on content: the block is trusted as the item's own record.
 * - A body with no block (or an unparseable one) is ascertained: native
 *   parent (when supplied), `primary-parent/<slug>` / `epic/<slug>`
 *   labels resolved through the caller's slug map, milestone, labels,
 *   timestamps, actor. A generated block is stored and returned as a
 *   replacement body for the caller to write back.
 * - No resolvable parent, or several distinct slugs (ambiguous program),
 *   is insufficient: the item is forwarded for full triage with queue
 *   evidence instead of being left half-described.
 *
 * The store layer (`metadata-store.ts`) is the system of record for
 * Metadata *queries*; GitHub stays the system of record for the items.
 */
import type { Database } from "bun:sqlite";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import {
  createPmMetadataStore,
  parsePmMetadata,
  upsertPmMetadata,
  type JsonObject,
  type ParsedPmMetadata,
  type PmMetadataRecord,
} from "./metadata-store.ts";

export const PM_METADATA_BLOCK_START = "<!-- pm-metadata:start -->";
export const PM_METADATA_BLOCK_END = "<!-- pm-metadata:end -->";

const BLOCK_PATTERN = /<!--\s*pm-metadata:start\s*-->([\s\S]*?)<!--\s*pm-metadata:end\s*-->/;
const JSON_FENCE_PATTERN = /```json\s*([\s\S]*?)```/i;

/** Minimal GitHub item snapshot the check needs. Both watchers build this. */
export interface IngestionItem {
  repo: string;
  number: number;
  kind: "issue" | "pr";
  title: string;
  body: string | null;
  labels: string[];
  milestoneTitle?: string | null;
  milestoneDueOn?: string | null;
  /** OPEN/CLOSED for issues; OPEN/CLOSED/MERGED for PRs. */
  state: string;
  author: string;
  createdAt: string;
  updatedAt: string;
  closedAt?: string | null;
  /** Sub-issue parent when the caller already resolved it; absent means unknown. */
  nativeParent?: { repo: string; number: number; kind: string } | null;
}

/** Epic slug (from `epic/<slug>` labels) to the epic issue it names. */
export type EpicSlugMap = Map<string, { repo: string; number: number }>;

export interface MetadataCheckContext {
  /** Pre-resolved per sweep run: one `gh` query per repo, not per item. */
  epicSlugs: EpicSlugMap;
  /** Recorded in generated provenance, e.g. `metadata-sweeper`. */
  ingestedBy: string;
  /** ISO timestamp of the run; defaults to now. */
  now?: string;
}

/** Evidence attached when an item is forwarded for full triage. */
export interface TriageEvidence {
  repo: string;
  number: number;
  kind: "issue" | "pr";
  title: string;
  reasons: string[];
  ascertained: {
    labels: string[];
    milestoneTitle: string | null;
    state: string;
    author: string;
    createdAt: string;
    updatedAt: string;
    closedAt: string | null;
    candidateSlugs: string[];
    nativeParent: { repo: string; number: number; kind: string } | null;
    invalidBlock: boolean;
  };
  sourceUrl: string;
}

export type MetadataCheckOutcome =
  | { kind: "current"; record: PmMetadataRecord }
  | { kind: "generated"; record: PmMetadataRecord; body: string; replacedInvalidBlock: boolean }
  | { kind: "needs-triage"; evidence: TriageEvidence };

export function sourceUrlFor(item: Pick<IngestionItem, "repo" | "number" | "kind">): string {
  const section = item.kind === "pr" ? "pull" : "issues";
  return `https://github.com/${item.repo}/${section}/${item.number}`;
}

/** Deterministic artifact identity: one GitHub item, one metadata row. */
export function artifactIdFor(item: Pick<IngestionItem, "repo" | "number" | "kind">): string {
  return `github:${item.repo}#${item.kind}/${item.number}`;
}

/** Extract the raw JSON payload of the canonical block, if any. */
export function extractMetadataPayload(
  body: string | null,
): { found: true; json: string } | { found: false; malformed: boolean } {
  if (!body) return { found: false, malformed: false };
  const block = BLOCK_PATTERN.exec(body)?.[1];
  if (block === undefined) return { found: false, malformed: false };
  const json = JSON_FENCE_PATTERN.exec(block)?.[1];
  if (json === undefined) return { found: false, malformed: true };
  return { found: true, json: json.trim() };
}

/** Render the canonical block around a metadata object. */
export function renderMetadataBlock(metadata: JsonObject): string {
  return `${PM_METADATA_BLOCK_START}\n\`\`\`json\n${JSON.stringify(metadata, null, 2)}\n\`\`\`\n${PM_METADATA_BLOCK_END}`;
}

/**
 * Return the body with the canonical block replaced, or appended when
 * absent. Idempotent: running it twice yields the same text.
 */
export function upsertMetadataBlock(body: string | null, metadata: JsonObject): string {
  const block = renderMetadataBlock(metadata);
  const current = body ?? "";
  if (BLOCK_PATTERN.test(current)) return current.replace(BLOCK_PATTERN, () => block);
  return `${current}${current ? "\n\n" : ""}${block}\n`;
}

const PARENT_LABEL_PATTERN = /^(?:primary-parent|epic)\/(.+)$/;

function candidateSlugs(labels: string[]): string[] {
  const slugs: string[] = [];
  for (const label of labels) {
    const slug = PARENT_LABEL_PATTERN.exec(label)?.[1]?.trim().toLowerCase();
    if (slug && !slugs.includes(slug)) slugs.push(slug);
  }
  return slugs;
}

export interface ResolvedParent {
  repo: string;
  number: number;
  kind: string;
}

/**
 * Resolve the ascertained direct parent: an explicit native parent is
 * authoritative, then `primary-parent/<slug>` / `epic/<slug>` labels are
 * resolved through the caller's slug map. Self-parent candidates are
 * excluded. Null means none resolvable; `ambiguous` lists distinct label
 * candidates when more than one resolves to different epics.
 */
export function resolveAscertainedParent(
  item: IngestionItem,
  epicSlugs: EpicSlugMap,
): { parent: ResolvedParent | null; ambiguous: string[]; candidateSlugs: string[] } {
  const slugs = candidateSlugs(item.labels);
  const isSelf = (parent: { repo: string; number: number }): boolean =>
    parent.repo.toLowerCase() === item.repo.toLowerCase() && parent.number === item.number;
  if (item.nativeParent && !isSelf(item.nativeParent)) {
    return { parent: { ...item.nativeParent }, ambiguous: [], candidateSlugs: slugs };
  }
  const resolved = new Map<string, ResolvedParent>();
  for (const slug of slugs) {
    const epic = epicSlugs.get(slug);
    if (epic && !isSelf(epic)) {
      resolved.set(`${epic.repo}#${epic.number}`, { repo: epic.repo, number: epic.number, kind: "epic" });
    }
  }
  if (resolved.size > 1) {
    return { parent: null, ambiguous: [...resolved.keys()].sort(), candidateSlugs: slugs };
  }
  const only = [...resolved.values()][0];
  return { parent: only ?? null, ambiguous: [], candidateSlugs: slugs };
}

function toRecord(item: IngestionItem, parsed: ParsedPmMetadata): PmMetadataRecord {
  return {
    artifactId: artifactIdFor(item),
    repo: item.repo,
    itemNumber: item.number,
    itemKind: item.kind,
    sourceUrl: sourceUrlFor(item),
    parsed,
    sourceUpdatedAt: item.updatedAt,
  };
}

/** Canonical `repo#number` key for cross-repo ancestor comparisons. */
export function itemRefKey(ref: { repo: string; number: number }): string {
  return `${ref.repo.toLowerCase()}#${ref.number}`;
}

/** `primary-parent/<slug>` labels carried by an item, in label order. */
export function primaryParentLabels(labels: string[]): string[] {
  return labels.filter((label) => label.startsWith("primary-parent/"));
}

export type PrimaryParentViolationKind =
  /** More than one `primary-parent/*` label names a parent. */
  | "multiple-primary-parent-labels"
  /** The label and the native sub-issue parent name different parents, and
   * the label is not an ancestor of the native parent (program-level label). */
  | "label-native-disagreement"
  /** A `primary-parent/<slug>` label whose slug resolves to no epic. */
  | "unresolved-primary-parent-label";

export interface PrimaryParentViolation {
  kind: PrimaryParentViolationKind;
  /** Human-readable finding, as it should appear in audit output. */
  message: string;
  item: { repo: string; number: number; kind: "issue" | "pr" };
  labels: string[];
  labelParents: ResolvedParent[];
  nativeParent: ResolvedParent | null;
}

/**
 * Collector-side validation of the primary-parent convention (design §3):
 * at most one `primary-parent/<slug>` per issue, and when a native
 * sub-issue parent exists the label must either agree with it or name one
 * of its ancestors (the program-level parent).
 *
 * `ancestorRefs` is the native parent's ancestor chain as
 * {@link itemRefKey} values. Pass it when known; pass `undefined` when the
 * chain could not be read, in which case only direct agreement is checked
 * (an unreadable chain must not manufacture a disagreement finding).
 *
 * This never resolves a violation: it reports it, exactly as the design
 * requires. Resolution stays a human/reconcile decision.
 */
export function collectPrimaryParentViolations(
  item: IngestionItem,
  epicSlugs: EpicSlugMap,
  ancestorRefs?: ReadonlySet<string>,
): PrimaryParentViolation[] {
  const labels = primaryParentLabels(item.labels);
  if (labels.length === 0) return [];
  const violations: PrimaryParentViolation[] = [];
  const base = {
    item: { repo: item.repo, number: item.number, kind: item.kind },
    labels,
  };

  const unresolved = labels.filter(
    (label) => !epicSlugs.has(label.slice("primary-parent/".length).trim().toLowerCase()),
  );
  if (unresolved.length > 0) {
    violations.push({
      ...base,
      kind: "unresolved-primary-parent-label",
      message: `primary-parent label(s) name no known epic: ${unresolved.join(", ")}`,
      labelParents: [],
      nativeParent: item.nativeParent ?? null,
    });
  }

  const labelParents: ResolvedParent[] = [];
  for (const label of labels) {
    const slug = label.slice("primary-parent/".length).trim().toLowerCase();
    const epic = epicSlugs.get(slug);
    if (!epic) continue;
    if (!labelParents.some((parent) => parent.repo === epic.repo && parent.number === epic.number)) {
      labelParents.push({ repo: epic.repo, number: epic.number, kind: "epic" });
    }
  }

  const distinct = new Set(labelParents.map((parent) => itemRefKey(parent)));
  if (labels.length > 1) {
    violations.push({
      ...base,
      kind: "multiple-primary-parent-labels",
      message:
        distinct.size > 1
          ? `${labels.length} primary-parent labels name ${distinct.size} distinct parents (${[...distinct].join(", ")})`
          : `${labels.length} primary-parent labels present; at most one is allowed`,
      labelParents,
      nativeParent: item.nativeParent ?? null,
    });
  }

  const nativeParent = item.nativeParent ?? null;
  if (nativeParent && labelParents.length > 0 && ancestorRefs !== undefined) {
    const agrees = labelParents.some((parent) => itemRefKey(parent) === itemRefKey(nativeParent));
    const ancestorAgrees = labelParents.some((parent) => ancestorRefs.has(itemRefKey(parent)));
    if (!agrees && !ancestorAgrees) {
      violations.push({
        ...base,
        kind: "label-native-disagreement",
        message:
          `primary-parent label names ${labelParents.map((parent) => itemRefKey(parent)).join(", ")} but the native parent is ` +
          `${itemRefKey(nativeParent)} and the label is not one of its ancestors`,
        labelParents,
        nativeParent,
      });
    }
  }

  return violations;
}

function triageEvidence(
  item: IngestionItem,
  reasons: string[],
  candidateSlugs: string[],
  invalidBlock: boolean,
): TriageEvidence {
  return {
    repo: item.repo,
    number: item.number,
    kind: item.kind,
    title: item.title,
    reasons,
    ascertained: {
      labels: [...item.labels],
      milestoneTitle: item.milestoneTitle ?? null,
      state: item.state,
      author: item.author,
      createdAt: item.createdAt,
      updatedAt: item.updatedAt,
      closedAt: item.closedAt ?? null,
      candidateSlugs,
      nativeParent: item.nativeParent ?? null,
      invalidBlock,
    },
    sourceUrl: sourceUrlFor(item),
  };
}

/**
 * The shared check both ingestion paths run. Pure except for the
 * returned record: persistence and body write-back are the caller's
 * job (the sweeper and the listener have different write seams), so
 * this function is identical for both by construction.
 */
export function runMetadataCheck(item: IngestionItem, context: MetadataCheckContext): MetadataCheckOutcome {
  const now = context.now ?? new Date().toISOString();
  const extracted = extractMetadataPayload(item.body);

  if (extracted.found) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(extracted.json) as unknown;
    } catch {
      parsed = null;
    }
    if (parsed !== null) {
      const result = parsePmMetadata(parsed);
      if (result.ok) return { kind: "current", record: toRecord(item, result.value) };
    }
    // A block that exists but does not parse is not trusted as content;
    // fall through to ascertainment and replace it on write-back.
    const generated = ascertain(item, context, { invalidBlock: true, now });
    if (generated) {
      return {
        kind: "generated",
        record: generated.record,
        body: upsertMetadataBlock(item.body, generated.metadata),
        replacedInvalidBlock: true,
      };
    }
    const { candidateSlugs } = resolveAscertainedParent(item, context.epicSlugs);
    return {
      kind: "needs-triage",
      evidence: triageEvidence(
        item,
        ["existing metadata block is unparseable; ascertainment insufficient"],
        candidateSlugs,
        true,
      ),
    };
  }

  const generated = ascertain(item, context, { invalidBlock: extracted.malformed, now });
  if (generated) {
    return {
      kind: "generated",
      record: generated.record,
      body: upsertMetadataBlock(item.body, generated.metadata),
      replacedInvalidBlock: false,
    };
  }
  const { ambiguous, candidateSlugs } = resolveAscertainedParent(item, context.epicSlugs);
  const reasons =
    ambiguous.length > 0
      ? [`ambiguous program: ${ambiguous.length} distinct parent candidates (${ambiguous.join(", ")})`]
      : ["no resolvable parent (no native parent; no primary-parent/<slug> or epic/<slug> label resolves to an epic)"];
  if (extracted.malformed) reasons.unshift("existing metadata block is present but has no JSON payload");
  return {
    kind: "needs-triage",
    evidence: triageEvidence(item, reasons, candidateSlugs, extracted.malformed),
  };
}

function ascertain(
  item: IngestionItem,
  context: MetadataCheckContext,
  extra: { invalidBlock: boolean; now: string },
): { record: PmMetadataRecord; metadata: JsonObject } | null {
  const { parent } = resolveAscertainedParent(item, context.epicSlugs);
  if (!parent) return null;
  const metadata: JsonObject = {
    pm_meta: 1,
    direct_parent: { repo: parent.repo, number: parent.number, kind: parent.kind },
    links: [],
    provenance: { actor: item.author, ingested_by: context.ingestedBy, ingested_at: extra.now },
    ascertained: {
      title: item.title,
      state: item.state,
      labels: [...item.labels],
      milestone: item.milestoneTitle ?? null,
      created_at: item.createdAt,
      updated_at: item.updatedAt,
      closed_at: item.closedAt ?? null,
      ...(extra.invalidBlock ? { replaced_invalid_block: true } : {}),
    },
  };
  const result = parsePmMetadata(metadata);
  // Constructed by this function against the v1 adapter; a failure here
  // is a programming error, never an item problem.
  if (!result.ok) throw new Error(`ascertained metadata failed the v1 adapter: ${result.error}`);
  return { record: toRecord(item, result.value), metadata };
}

/**
 * Build the full-triage queue packet for a needs-triage outcome. Lives in
 * the shared module (not the sweeper) so the packet shape is one
 * implementation too, and so tests can pin the repo-task/v1 required
 * fields the broker validates (schema, kind, problem, priority_reason,
 * non-empty evidence/acceptance/tests, scope, risk, timebox, attempts).
 */
export function buildTriagePacket(evidence: TriageEvidence, ingestedBy: string, at: string): Record<string, unknown> {
  const problem = [
    `Dual ingestion (${ingestedBy}) could not ascertain sufficient Metadata for ${evidence.repo}#${evidence.number} (${evidence.kind}):`,
    ...evidence.reasons.map((reason) => `- ${reason}`),
    `Forwarded for full triage with this evidence instead of leaving the item half-described.`,
  ].join("\n");
  const snap = evidence.ascertained;
  const labelList = snap.labels.join(", ") || "(none)";
  return {
    title: `triage: ascertain Metadata for ${evidence.repo}#${evidence.number} (dual-ingestion forward)`,
    body: [
      problem,
      "",
      `Item: ${evidence.sourceUrl} — ${evidence.title}`,
      `State: ${snap.state}; author: ${snap.author}; labels: ${labelList}`,
    ].join("\n"),
    // Broker design: triage keys anchor to issue:<n> even for PR items.
    dedupe_key: `repo-health:v1:triage:${evidence.repo}:issue:${evidence.number}`,
    priority: 30,
    payload: {
      schema: "repo-task/v1",
      kind: "triage",
      repo: evidence.repo,
      issue: { number: evidence.number, url: evidence.sourceUrl, state_checked_at: at },
      problem,
      evidence: [
        `Dual ingestion (${ingestedBy}) at ${at}: ${evidence.reasons.join("; ")}`,
        `Ascertained snapshot: state=${snap.state} author=${snap.author} labels=[${labelList}] milestone=${snap.milestoneTitle ?? "none"} created=${snap.createdAt} updated=${snap.updatedAt} closed=${snap.closedAt ?? "n/a"}`,
        `candidate slugs=[${snap.candidateSlugs.join(", ") || "none"}] nativeParent=${JSON.stringify(snap.nativeParent)} invalidBlock=${snap.invalidBlock}`,
        `Item: ${evidence.sourceUrl}`,
      ],
      acceptance: [
        "Metadata for the item is ascertained (native parent, labels, milestone, timestamps, actor), stored in the metadata DB, and written back to the body where the fleet has write access",
        "If still insufficient, findings are recorded on the issue with the blocking evidence named",
      ],
      tests: ["The metadata DB holds a current row for the item's artifact id"],
      context_refs: [evidence.sourceUrl, "developing-today/code#9076"],
      scope: {
        include: [`ascertain and record Metadata for ${evidence.repo}#${evidence.number}`],
        exclude: ["Unrelated changes outside this item's Metadata"],
      },
      risk: { level: "low", flags: [] },
      priority_reason:
        "Dual-ingestion forward: item lacks an ascertainable Metadata parent, so full triage owns it (resolved to 30 on the queue design scale).",
      timebox_minutes: 20,
      max_attempts: 1000,
      dedupe: {
        checked_at: at,
        basis: `Dual-ingestion forward for ${evidence.repo}#${evidence.number}; canonical triage key anchors to the item.`,
      },
    },
  };
}

/**
 * Persist an outcome's record. Triage outcomes store nothing — the
 * queue packet the caller enqueues is their durable trail.
 */
export function storeOutcome(db: Database, outcome: MetadataCheckOutcome): void {
  if (outcome.kind === "needs-triage") return;
  upsertPmMetadata(db, outcome.record);
}

/** Open (creating) the sweeper/listener database with the store installed. */
export function openMetadataStore(path: string, open: (path: string) => Database): Database {
  // bun:sqlite creates the database file but not its parent directories,
  // so a clean environment (no state directory yet) crashed the server at
  // boot with SQLITE_CANTOPEN (#13293). Create the parent first, like the
  // other state stores do on write; genuine open failures (permissions,
  // a parent that is not a directory) still throw — only the
  // missing-directory case self-heals.
  if (path !== ":memory:") mkdirSync(dirname(path), { recursive: true });
  const db = open(path);
  createPmMetadataStore(db);
  return db;
}
