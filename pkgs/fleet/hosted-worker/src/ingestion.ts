// Ingestion — the hosted half of dual ingestion (#12590; design
// §7.3, #9060 §16). The native ingestion of #9076 remains a peer:
// both editions run the same check, both write the same DB, and the
// check is idempotent by construction (insert/refresh keyed on the
// item), so a duplicated webhook delivery, a sweeper pass over an
// already-ingested item, and the two editions racing all converge
// on one DB row and at most one triage task per item.
//
// THE CHECK (one code path, two triggers — the webhook route and
// the sweeper call runIngestionCheck over an item as read through
// the GitHub item source, so both answer identically for the same
// item state):
//
//   1. Parse the body's pm-metadata block. A malformed block fails
//      closed (400 invalid_metadata_block) — the create path's
//      rule: caller-owned fields are never silently dropped, and a
//      block the watcher cannot parse is a human's fix, not a
//      refresh.
//   2. No block → ascertain one (§16): the primary parent from the
//      item's native marks (below), timestamps and the rest of the
//      §12 provenance from the item read; sign the generated
//      provenance when stamp material is configured (an
//      unconfigured signer leaves it unsigned — the state the
//      query door already reports for native blocks — never a
//      forged or half-made signature); store the record; write
//      the block back to the body where the fleet has write access
//      (a refused edit is a named outcome; the DB row stands).
//   3. Insert or refresh the DB record, keyed on the item (§16).
//      The row's tenancy is the block's own stamped auth_subject;
//      a block with none (native-created, or ascertained here)
//      lands under UNATTRIBUTED_BINDING_ID — stored, gettable by
//      identity, and invisible to per-tenancy lists until a later
//      stamped edit attributes it (the query door's rule).
//   4. Sufficiency (§16's triage fork): Metadata is sufficient
//      when a parent resolves (direct_parent, or a parent link)
//      or the item is already queue-tracked (provenance.queue_id).
//      Insufficient items are forwarded for full triage — one
//      repo-task/v1 triage packet per item, deduped by the
//      queue's own dedupe key, so redelivery never duplicates it.
//
// Ascertainment of the parent, honestly scoped to what an item
// read carries: §3's canonical marks are the native sub-issue
// parent (not exposed on the item read) and the
// primary-parent/<slug> label naming the parent epic by its
// epic/<slug> label. The watcher resolves that slug against the
// repo's own items: exactly one OTHER item carrying both
// epic/<slug> and the plain `epic` label is the parent epic.
// Zero or several candidates — or several primary-parent labels,
// which §3's invariant forbids — leave the parent unresolvable,
// which is precisely §16's insufficient case: forward to triage.

import type { AuthDeps } from "./auth.ts";
import { SERVICE_NAME, type StampSigningMaterial } from "./config.ts";
import { FailClosed } from "./errors.ts";
import type { GitHubBodyEditor, GitHubItem, GitHubItemSource } from "./github.ts";
import { assembleStampedBlock, parseMetadataBlock, renderBodyWithBlock } from "./metadata.ts";
import { executeQueueVerb, LIVE_STATUSES, nativeQueueEnv } from "./queue.ts";
import type { QueueStore } from "./queue-store.ts";
import type { JsonObject } from "./queue-validate.ts";
import { importStampSigner, signStamp, type Provenance } from "./stamp-sig.ts";
import { extractKeys, type MetadataRecord, type MetadataStore } from "./stamping.ts";

/** The binding_id ingested rows carry when their block names no
 * tenancy (no stamped auth_subject): a fixed marker no minted
 * binding_id (32 lowercase hex) can collide with. */
export const UNATTRIBUTED_BINDING_ID = "unattributed";

/** The queue triage forwards land in: the pilot stream the
 * fleet's repo-task/v1 triage packets for the code repo use. */
export const TRIAGE_QUEUE = "repo-health-code-pilot";

/** The triage dedupe key for an item (repo-task/v1 triage family):
 * one live triage task per item, ever, across both editions and
 * any number of duplicate deliveries. */
export function triageDedupeKey(repo: string, number: number): string {
  return `repo-health:v1:triage:${repo}:issue:${number}`;
}

export interface IngestionDeps {
  store: MetadataStore;
  items: GitHubItemSource;
  /** Write-back seam; absent → ascertained blocks are stored
   * DB-only and the check reports writeback "not_attempted". */
  editor?: GitHubBodyEditor;
  /** Triage-forward seam; absent → insufficient items report
   * queue action "unavailable" (the DB heal still lands). */
  queueStore?: QueueStore;
  /** Stamp signing material for ascertained provenance, or null:
   * generated blocks then carry an unsigned provenance. */
  stampSigning: StampSigningMaterial | null;
  /** This service's deployment identity (stamped_by). */
  stampedBy: string;
  now: () => number;
}

/** Resolve the engine's deps from the door's, failing closed with
 * the doors' 503 idiom on the two the check cannot run without:
 * the metadata store (the heal's target) and the GitHub item
 * source (the check's only read of the item — the webhook payload
 * is an event notification, never the read the check judges by;
 * the query door's fresh-read rule). */
export function ingestionDepsFrom(deps: AuthDeps): IngestionDeps {
  if (!deps.metadataStore) {
    throw new FailClosed(
      503,
      "metadata_store_not_configured",
      "this deployment has no metadata store configured; failing closed",
    );
  }
  if (!deps.githubItems) {
    throw new FailClosed(
      503,
      "github_item_source_not_configured",
      "this deployment has no GitHub item source configured; failing closed",
    );
  }
  return {
    store: deps.metadataStore,
    items: deps.githubItems,
    ...(deps.githubEditor ? { editor: deps.githubEditor } : {}),
    ...(deps.queueStore ? { queueStore: deps.queueStore } : {}),
    stampSigning: deps.config.stamp.signing,
    stampedBy: `${SERVICE_NAME}@${deps.config.version}`,
    now: deps.now ?? (() => Date.now()),
  };
}

// --- Webhook delivery authentication + parsing -------------------

/** Verify a delivery's X-Hub-Signature-256 against the configured
 * secret: HMAC-SHA256 over the raw body bytes, compared by
 * WebCrypto's verify (constant-time). The signature is the
 * delivery's credential — GitHub holds no fleet token — so this
 * check IS the webhook route's authentication step, run before
 * any parsing of the unauthenticated body. */
export async function verifyWebhookSignature(secret: string, rawBody: string, header: string | null): Promise<void> {
  if (!header) {
    throw new FailClosed(401, "webhook_signature_missing", "no X-Hub-Signature-256 header");
  }
  const match = /^sha256=([0-9a-f]{64})$/.exec(header.trim());
  if (!match) {
    throw new FailClosed(401, "webhook_signature_invalid", "X-Hub-Signature-256 is not a sha256 HMAC hex digest");
  }
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["verify"],
  );
  const signature = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    signature[i] = parseInt(match[1].slice(i * 2, i * 2 + 2), 16);
  }
  const ok = await crypto.subtle.verify(
    "HMAC",
    key,
    signature as BufferSource,
    new TextEncoder().encode(rawBody) as BufferSource,
  );
  if (!ok) {
    throw new FailClosed(401, "webhook_signature_invalid", "webhook signature does not verify");
  }
}

export interface WebhookDelivery {
  event: string;
  action: string;
  deliveryId: string | null;
  repo: string;
  kind: "issue" | "pr";
  number: number;
}

/** The actions §16's listener answers; any other action, and any
 * other event, is acknowledged and ignored (GitHub must not retry
 * a delivery the check has no business with). */
const INGESTED_ACTIONS = new Set(["opened", "edited", "closed", "reopened", "labeled"]);

export type WebhookParse = { ignored: string } | { delivery: WebhookDelivery };

/** Parse a signature-verified delivery body into the item it
 * names. Malformed shapes refuse 400 invalid_webhook_payload —
 * the delivery claims to be an issues/pull_request event but does
 * not name its item, which no retry will fix. */
export function parseWebhookDelivery(event: string | null, deliveryId: string | null, rawBody: string): WebhookParse {
  if (!event) {
    throw new FailClosed(400, "invalid_webhook_payload", "no X-GitHub-Event header");
  }
  if (event === "ping") return { ignored: "ping" };
  if (event !== "issues" && event !== "pull_request") {
    return { ignored: `event ${event} is not ingested` };
  }
  let payload: unknown;
  try {
    payload = JSON.parse(rawBody);
  } catch {
    throw new FailClosed(400, "invalid_webhook_payload", "delivery body is not valid JSON");
  }
  if (typeof payload !== "object" || payload === null || Array.isArray(payload)) {
    throw new FailClosed(400, "invalid_webhook_payload", "delivery body must be a JSON object");
  }
  const obj = payload as Record<string, unknown>;
  const action = obj["action"];
  if (typeof action !== "string" || !INGESTED_ACTIONS.has(action)) {
    return { ignored: `action ${typeof action === "string" ? action : "(absent)"} is not ingested` };
  }
  const repoObj = obj["repository"];
  const repo =
    typeof repoObj === "object" && repoObj !== null ? (repoObj as Record<string, unknown>)["full_name"] : undefined;
  const itemObj = obj[event === "issues" ? "issue" : "pull_request"];
  const number =
    typeof itemObj === "object" && itemObj !== null ? (itemObj as Record<string, unknown>)["number"] : undefined;
  if (typeof repo !== "string" || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) {
    throw new FailClosed(
      400,
      "invalid_webhook_payload",
      "delivery does not name its repository (repository.full_name)",
    );
  }
  if (typeof number !== "number" || !Number.isInteger(number) || number <= 0) {
    throw new FailClosed(400, "invalid_webhook_payload", "delivery does not name its item number");
  }
  return {
    delivery: {
      event,
      action,
      deliveryId,
      repo,
      kind: event === "issues" ? "issue" : "pr",
      number,
    },
  };
}

// --- The check ----------------------------------------------------

export interface IngestionCheckResult {
  repo: string;
  kind: "issue" | "pr";
  number: number;
  url: string;
  source: "webhook" | "sweep";
  /** The delivery that triggered the check (webhook only). */
  trigger: { event: string; action: string; delivery: string | null } | null;
  /** The DB effect: a row appeared, changed, or already matched.
   * "error" appears only in sweep reports (per-item refusal,
   * recorded while the sweep continues). */
  outcome: "inserted" | "refreshed" | "unchanged" | "error";
  /** Where the stored Metadata came from: the body's own block,
   * or one the watcher ascertained. Absent on error entries. */
  block?: "authored" | "ascertained";
  /** The row's tenancy: the block's stamped auth_subject, or
   * UNATTRIBUTED_BINDING_ID. Absent on error entries. */
  binding_id?: string;
  sufficient?: boolean;
  queue?: {
    action: "enqueued" | "already_enqueued" | "not_warranted" | "unavailable" | "failed";
    task_id?: number;
    dedupe_key?: string;
    error?: string;
  };
  /** Ascertained blocks only: the body write-back's fate. */
  writeback?: "written" | "refused" | "not_attempted";
  /** Ascertained blocks only: whether the generated provenance
   * carries the service's stamp signature. */
  stamp?: "signed" | "unsigned";
  state?: string | null;
  merged?: boolean;
  /** Sweep error entries only: the refusal's stable code. */
  error?: string;
  message?: string;
}

function isItemRef(value: unknown): value is { repo: string; number: number } {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    typeof (value as Record<string, unknown>)["repo"] === "string" &&
    typeof (value as Record<string, unknown>)["number"] === "number"
  );
}

/** §16's sufficiency fork: a parent resolves (direct_parent, or a
 * parent link), or the item is already queue-tracked. Anything
 * else is half-described and goes to full triage. */
function isSufficient(block: Record<string, unknown>): boolean {
  if (isItemRef(block["direct_parent"])) return true;
  const links = Array.isArray(block["links"]) ? block["links"] : [];
  for (const link of links) {
    if (
      typeof link === "object" &&
      link !== null &&
      (link as Record<string, unknown>)["relation"] === "parent" &&
      isItemRef((link as Record<string, unknown>)["target"])
    ) {
      return true;
    }
  }
  const provenance = block["provenance"];
  if (typeof provenance === "object" && provenance !== null) {
    const queueId = (provenance as Record<string, unknown>)["queue_id"];
    if (typeof queueId === "string" && queueId !== "") return true;
  }
  return false;
}

/** Canonical JSON for row-vs-block equality: sorted keys, so the
 * comparison is about content, not serialization order. */
function stableStringify(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map((v) => stableStringify(v)).join(",")}]`;
  }
  if (typeof value === "object" && value !== null) {
    const entries = Object.entries(value as Record<string, unknown>)
      .filter(([, v]) => v !== undefined)
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${stableStringify(v)}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

/** Resolve the §3 primary-parent label against the repo's items
 * (see the header note): the slug's epic is the one OTHER item
 * carrying both epic/<slug> and the plain `epic` label. */
async function resolvePrimaryParent(
  deps: IngestionDeps,
  item: GitHubItem,
): Promise<{ repo: string; number: number; kind: string } | null> {
  const labels = item.labels ?? [];
  const parentLabels = labels.filter((l) => l.startsWith("primary-parent/"));
  if (parentLabels.length !== 1) return null;
  const slug = parentLabels[0].slice("primary-parent/".length);
  if (slug === "") return null;
  const items = await deps.items.listItems(item.repo);
  const candidates = items.filter(
    (other) =>
      other.number !== item.number &&
      (other.labels ?? []).includes(`epic/${slug}`) &&
      (other.labels ?? []).includes("epic"),
  );
  if (candidates.length !== 1) return null;
  return { repo: item.repo, number: candidates[0].number, kind: "epic" };
}

/** Generate the block for an item that lacks one (§16): caller-
 * owned fields from ascertainment (the rest take the required-
 * field defaults), provenance from the watcher's own knowledge —
 * the item's timestamps, never a caller's assertion — signed when
 * the service holds stamp material. */
async function ascertainBlock(
  deps: IngestionDeps,
  item: GitHubItem,
  nowIso: string,
): Promise<{ block: Record<string, unknown>; stamp: "signed" | "unsigned" }> {
  const parent = await resolvePrimaryParent(deps, item);
  const links: Record<string, unknown>[] = parent
    ? [
        {
          target: { repo: parent.repo, number: parent.number },
          relation: "parent",
          status: "current",
          ...(item.createdAt ? { since: item.createdAt.slice(0, 10) } : {}),
        },
      ]
    : [];
  const provenance: Provenance = {
    queue_id: null,
    scheduled_start: null,
    picked_up_at: null,
    work_started_at: null,
    created_at: item.createdAt ?? nowIso,
    session_id: null,
    lane: null,
    host: null,
    worktree: null,
    model: null,
    lane_start_issues: [],
    workspace_ref: null,
    auth_subject: null,
    stamped_by: deps.stampedBy,
  };
  let signed = provenance;
  let stamp: "signed" | "unsigned" = "unsigned";
  if (deps.stampSigning) {
    signed = await signStamp(provenance, await importStampSigner(deps.stampSigning));
    stamp = "signed";
  }
  const block = assembleStampedBlock(
    {
      direct_parent: parent,
      links,
      lineage: { design: null, roadmap_item: null, priority: null },
    },
    signed,
  );
  return { block, stamp };
}

/** The triage packet for an insufficient item: the repo-task/v1
 * triage shape the native filers use, built from service facts
 * only (the item's identity and URL, the check's findings). */
function buildTriagePacket(item: GitHubItem, dedupeKey: string, nowIso: string): JsonObject {
  const ref = `${item.repo}#${item.number}`;
  return {
    queue: TRIAGE_QUEUE,
    title: `Triage ${ref}: ingested without sufficient Metadata`,
    body:
      `Forwarded for full triage by hosted ingestion (#12590, #9060 §16): ` +
      `the item's Metadata is insufficient (no resolvable parent, not queue-tracked). ` +
      `Ascertain or assign its parentage and program, then disposition it.`,
    priority: 30,
    dedupe_key: dedupeKey,
    payload: {
      schema: "repo-task/v1",
      kind: "triage",
      repo: item.repo,
      issue: { number: item.number },
      issues: [item.number],
      problem:
        `Item ${ref} was ingested by the hosted watcher with insufficient Metadata: ` +
        `no resolvable parent and no queue provenance. It is stored in the metadata DB ` +
        `but half-described; full triage must place it.`,
      evidence: [item.url],
      context_refs: ["https://github.com/developing-today/code/issues/12590"],
      acceptance: [
        "The item's parentage and program are ascertained or assigned and recorded on the item",
        "The item's Metadata block (or the triage disposition) reflects the placement",
      ],
      scope: {
        include: [`the item ${ref} and its Metadata`],
        exclude: ["implementing the work the item describes"],
      },
      tests: ["read back the item's Metadata after placement"],
      risk: { level: "low", flags: [] },
      priority_reason:
        "Ingestion forwards insufficient items for full triage instead of leaving them half-described (#9060 §16).",
      timebox_minutes: 45,
      dedupe: {
        key: dedupeKey,
        checked_at: nowIso,
        basis: "hosted ingestion check: item stored with insufficient Metadata",
      },
      identity: { issue_number: item.number },
    },
  } as JsonObject;
}

/** Forward an insufficient item for full triage. The queue's own
 * dedupe (same key, live row) is the idempotency: a redelivery
 * finds the live task and enqueues nothing. A rejected packet is
 * recorded, never thrown — the DB heal has already landed, and a
 * retry of the delivery would fail the same way; the named gap
 * rides the check result instead. */
function forwardToTriage(
  deps: IngestionDeps,
  item: GitHubItem,
  nowIso: string,
): NonNullable<IngestionCheckResult["queue"]> {
  if (!deps.queueStore) return { action: "unavailable" };
  const dedupeKey = triageDedupeKey(item.repo, item.number);
  const live = deps.queueStore.tasksByDedupe(TRIAGE_QUEUE, dedupeKey).find((t) => LIVE_STATUSES.includes(t.status));
  if (live) return { action: "already_enqueued", task_id: live.id, dedupe_key: dedupeKey };
  const resp = executeQueueVerb(
    deps.queueStore,
    "enqueue",
    { queue: TRIAGE_QUEUE, packet: buildTriagePacket(item, dedupeKey, nowIso) },
    nativeQueueEnv(() => Math.floor(deps.now() / 1000)),
  );
  if (resp["ok"] === true) {
    const row = resp["row"] as Record<string, unknown> | undefined;
    return {
      action: "enqueued",
      ...(typeof row?.["id"] === "number" ? { task_id: row["id"] } : {}),
      dedupe_key: dedupeKey,
    };
  }
  return {
    action: "failed",
    dedupe_key: dedupeKey,
    error: typeof resp["error"] === "string" ? resp["error"] : "triage enqueue failed",
  };
}

/** The §16 Metadata check over one item, as read from GitHub.
 * Throws FailClosed on a malformed block (400) — the webhook
 * route surfaces it; the sweeper records it per item. */
export async function runIngestionCheck(
  deps: IngestionDeps,
  item: GitHubItem,
  source: "webhook" | "sweep",
  trigger: IngestionCheckResult["trigger"],
): Promise<IngestionCheckResult> {
  const nowIso = new Date(deps.now()).toISOString();
  const parsed = parseMetadataBlock(item.body);

  let block: Record<string, unknown>;
  let blockKind: "authored" | "ascertained";
  let writeback: IngestionCheckResult["writeback"];
  let stamp: IngestionCheckResult["stamp"];
  if (parsed.block !== null) {
    block = parsed.block;
    blockKind = "authored";
  } else {
    const generated = await ascertainBlock(deps, item, nowIso);
    block = generated.block;
    blockKind = "ascertained";
    stamp = generated.stamp;
    if (deps.editor) {
      try {
        await deps.editor.updateItemBody(item.repo, item.kind, item.number, renderBodyWithBlock(parsed, block));
        writeback = "written";
      } catch {
        // No write access (or the item vanished): the DB row
        // below is the record; the body catches up when a later
        // check runs with access (§15 divergence, reported by
        // the query door meanwhile).
        writeback = "refused";
      }
    } else {
      writeback = "not_attempted";
    }
  }

  const provenance =
    typeof block["provenance"] === "object" && block["provenance"] !== null
      ? (block["provenance"] as Record<string, unknown>)
      : {};
  const authSubject = provenance["auth_subject"];
  const bindingId = typeof authSubject === "string" ? authSubject : UNATTRIBUTED_BINDING_ID;
  const sufficient = isSufficient(block);

  const record: MetadataRecord = {
    repo: item.repo,
    kind: item.kind,
    number: item.number,
    url: item.url,
    bindingId,
    metadata: block,
    extracted: extractKeys(block),
    insertedAt: nowIso,
  };
  const existing = await deps.store.get(item.repo, item.kind, item.number);
  let outcome: IngestionCheckResult["outcome"];
  if (!existing) {
    await deps.store.insert(record);
    outcome = "inserted";
  } else if (existing.bindingId === bindingId && stableStringify(existing.metadata) === stableStringify(block)) {
    outcome = "unchanged";
  } else {
    await deps.store.insert(record);
    outcome = "refreshed";
  }

  const queue = sufficient ? { action: "not_warranted" as const } : forwardToTriage(deps, item, nowIso);

  return {
    repo: item.repo,
    kind: item.kind,
    number: item.number,
    url: item.url,
    source,
    trigger,
    outcome,
    block: blockKind,
    binding_id: bindingId,
    sufficient,
    queue,
    ...(writeback !== undefined ? { writeback } : {}),
    ...(stamp !== undefined ? { stamp } : {}),
    state: item.state ?? null,
    merged: item.merged ?? false,
  };
}

// --- The sweeper ---------------------------------------------------

export interface IngestionSweepResult {
  repos: string[];
  results: IngestionCheckResult[];
  counts: {
    checked: number;
    inserted: number;
    refreshed: number;
    unchanged: number;
    ascertained: number;
    triage_enqueued: number;
    errors: number;
  };
}

/** The interval sweeper (§16 watcher 2): re-scan the in-scope
 * repos for items the listener missed and run the same check.
 * Per-item refusals (a malformed block is the expected one) are
 * recorded as error entries while the sweep continues — one bad
 * item never starves the rest of the repo. A failure to list a
 * repo at all is a transport failure and surfaces (502). */
export async function runIngestionSweep(deps: IngestionDeps, repos: string[]): Promise<IngestionSweepResult> {
  const results: IngestionCheckResult[] = [];
  for (const repo of repos) {
    const items = await deps.items.listItems(repo);
    for (const item of items) {
      try {
        results.push(await runIngestionCheck(deps, item, "sweep", null));
      } catch (e) {
        results.push({
          repo: item.repo,
          kind: item.kind,
          number: item.number,
          url: item.url,
          source: "sweep",
          trigger: null,
          outcome: "error",
          error: e instanceof FailClosed ? e.code : "internal",
          message: e instanceof Error ? e.message : "ingestion check failed",
          state: item.state ?? null,
          merged: item.merged ?? false,
        });
      }
    }
  }
  const counts = {
    checked: results.length,
    inserted: results.filter((r) => r.outcome === "inserted").length,
    refreshed: results.filter((r) => r.outcome === "refreshed").length,
    unchanged: results.filter((r) => r.outcome === "unchanged").length,
    ascertained: results.filter((r) => r.block === "ascertained").length,
    triage_enqueued: results.filter((r) => r.queue?.action === "enqueued").length,
    errors: results.filter((r) => r.outcome === "error").length,
  };
  return { repos: [...repos], results, counts };
}
