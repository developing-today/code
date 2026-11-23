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
 * Resolve the ascertained direct parent: explicit native parent first,
 * then `primary-parent/<slug>` / `epic/<slug>` labels through the
 * caller's slug map. Null means none resolvable; `ambiguous` lists the
 * distinct slugs when more than one resolves to different epics.
 */
export function resolveAscertainedParent(
  item: IngestionItem,
  epicSlugs: EpicSlugMap,
): { parent: ResolvedParent | null; ambiguous: string[]; candidateSlugs: string[] } {
  const slugs = candidateSlugs(item.labels);
  const resolved = new Map<string, ResolvedParent>();
  for (const slug of slugs) {
    const epic = epicSlugs.get(slug);
    if (epic) resolved.set(`${epic.repo}#${epic.number}`, { repo: epic.repo, number: epic.number, kind: "epic" });
  }
  if (item.nativeParent) {
    resolved.set(`${item.nativeParent.repo}#${item.nativeParent.number}`, { ...item.nativeParent });
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
      evidence: triageEvidence(item, ["existing metadata block is unparseable; ascertainment insufficient"], candidateSlugs, true),
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
export function buildTriagePacket(
  evidence: TriageEvidence,
  ingestedBy: string,
  at: string,
): Record<string, unknown> {
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
      dedupe: { checked_at: at, basis: `Dual-ingestion forward for ${evidence.repo}#${evidence.number}; canonical triage key anchors to the item.` },
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
  const db = open(path);
  createPmMetadataStore(db);
  return db;
}
