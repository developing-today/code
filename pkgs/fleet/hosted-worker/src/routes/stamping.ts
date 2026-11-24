// Stamping door — the create path (#12585; design §3.1, §5).
//
// POST /v1/stamps creates an issue or PR through the service, which
// is the hosted world's only stamping authority:
//
//   1. Parse any pm-metadata block in the caller's body (metadata.ts;
//      a malformed block refuses the create — caller-owned fields
//      are never silently dropped).
//   2. Stamp every fleet-owned provenance field from service sources
//      only (the §5 table, below). Caller-supplied provenance values
//      are replaced wholesale; caller-owned fields (direct_parent,
//      links, lineage) are preserved exactly, with required-field
//      defaults injected when absent.
//   3. Sign the provenance (#12586's signStamp). Unconfigured
//      signing fails closed (503) BEFORE any GitHub write: this door
//      never creates an unstamped item.
//   4. Create the GitHub item through the hosted rotation point
//      (src/github.ts) with the stamped body.
//   5. Insert the full record into the metadata DB in the same
//      request. A failed insert leaves the GitHub item intact and is
//      reported in the response (metadata_inserted: false) — the
//      ingestion slice (#12590, design §7.3) heals the miss.
//
// §5 sourcing, as implemented here:
//   queue_id / scheduled_start / picked_up_at — the queue record
//     (QueueRecordSource), null when the work is not queue-originated.
//   work_started_at — the queue record's; where a hosted caller works
//     without a claim, the session-open time is the fallback (the
//     source is distinguishable via stamped_by, per §5).
//   created_at — the service clock at stamp time.
//   session_id — the request's cited session, resolved against the
//     registry (unknown/closed refused with the registry's named
//     errors, so a stamp can never cite a session the service did
//     not open). No session cited → null: the create is not
//     session-originated.
//   lane — the session record's lane, else the agent credential's.
//   model / lane_start_issues — the session record's (the queue
//     record's lane-start set when the session carries none).
//   host / worktree / workspace_ref — caller claims (design §3.2:
//     claims the service may verify, like repo and branch). The
//     service cannot observe a CLI caller's machine, so these are
//     taken from the request — never from the body's block, whose
//     provenance is replaced in full. For a service-token caller
//     with no host claim, host is the caller service's label.
//   auth_subject — the caller's credential-resolved binding_id.
//   stamped_by — this service's deployment identity.
//
// JUDGMENT CALLS (stated, per the slice): `scheduled_start` is
// stamped as a provenance key — §5 lists it in the stamp set and
// §12's format allows arbitrary keys; the native §12 example does
// not name the key, so the hosted edition fixes it here and the
// native side conforms when #9073 lands (design §6 discipline).
// A create citing no session stamps session_id null rather than
// refusing: §4.2 refuses unknown/closed CITED ids; an uncited create
// (a person at the CLI) is a shape the §12 stamp set already carries
// as nulls.

import type { AuthDeps, CallerContext } from "../auth.ts";
import { SERVICE_NAME } from "../config.ts";
import { FailClosed } from "../errors.ts";
import type { RouteDef } from "../router.ts";
import { checkedSessionId, requireOpenSession, type SessionRecord } from "../sessions.ts";
import { assembleStampedBlock, parseMetadataBlock, renderBodyWithBlock } from "../metadata.ts";
import { requireStampSigner, signStamp, type Provenance } from "../stamp-sig.ts";
import { extractKeys, type MetadataRecord } from "../stamping.ts";

function caller(ctx: CallerContext | null): CallerContext {
  if (!ctx) {
    throw new FailClosed(401, "missing_token", "stamping routes require an authenticated caller");
  }
  return ctx;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function jsonBody(req: Request): Promise<Record<string, unknown>> {
  let parsed: unknown;
  try {
    parsed = await req.json();
  } catch {
    throw new FailClosed(400, "invalid_body", "request body is not valid JSON");
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new FailClosed(400, "invalid_body", "request body must be a JSON object");
  }
  return parsed as Record<string, unknown>;
}

function requiredString(body: Record<string, unknown>, field: string): string {
  const value = body[field];
  if (typeof value !== "string" || value.trim() === "") {
    throw new FailClosed(400, "invalid_body", `${field} must be a non-empty string`);
  }
  return value;
}

function optionalString(body: Record<string, unknown>, field: string): string | null {
  const value = body[field];
  if (value === undefined || value === null) return null;
  if (typeof value !== "string" || value.trim() === "") {
    throw new FailClosed(400, "invalid_body", `${field} must be a non-empty string when present`);
  }
  return value;
}

const WORKSPACE_KINDS = ["worktree", "session", "codespace", "worker", "service"] as const;

/** Normalize the caller's workspace_ref claim (design §5): kind from
 * the fixed vocabulary; the remaining fields are strings or null. */
function workspaceRefClaim(
  body: Record<string, unknown>,
  fallback: Record<string, unknown> | null,
): Record<string, unknown> | null {
  const raw = body["workspace_ref"];
  if (raw === undefined || raw === null) return fallback;
  if (typeof raw !== "object" || Array.isArray(raw)) {
    throw new FailClosed(400, "invalid_body", "workspace_ref must be an object when present");
  }
  const claim = raw as Record<string, unknown>;
  const kind = claim["kind"];
  if (typeof kind !== "string" || !(WORKSPACE_KINDS as readonly string[]).includes(kind)) {
    throw new FailClosed(400, "invalid_body", `workspace_ref.kind must be one of ${WORKSPACE_KINDS.join(", ")}`);
  }
  const out: Record<string, unknown> = { kind };
  for (const field of ["id", "repo", "branch", "sha"]) {
    const value = claim[field];
    if (value === undefined || value === null) {
      out[field] = null;
    } else if (typeof value === "string" && value !== "") {
      out[field] = value;
    } else {
      throw new FailClosed(400, "invalid_body", `workspace_ref.${field} must be a string when present`);
    }
  }
  if (out["id"] === null && fallback && fallback["kind"] === kind) {
    out["id"] = fallback["id"];
  }
  return out;
}

async function createStamped(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  const me = caller(ctx);

  // Fail closed on every missing service input BEFORE any side
  // effect: no signer, no store, no writer → no create at all.
  const signer = await requireStampSigner(deps.config.stamp.signing);
  if (!deps.metadataStore) {
    throw new FailClosed(
      503,
      "metadata_store_not_configured",
      "this deployment has no metadata store configured; failing closed",
    );
  }
  if (!deps.githubWriter) {
    throw new FailClosed(
      503,
      "github_writer_not_configured",
      "this deployment has no GitHub writer configured; failing closed",
    );
  }

  const body = await jsonBody(req);
  const repo = requiredString(body, "repo");
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) {
    throw new FailClosed(400, "invalid_body", "repo must be in owner/name form");
  }
  const kindRaw = body["kind"];
  if (kindRaw !== "issue" && kindRaw !== "pr") {
    throw new FailClosed(400, "invalid_body", 'kind must be "issue" or "pr"');
  }
  const kind = kindRaw as "issue" | "pr";
  const title = requiredString(body, "title");
  // The caller's body text: any string is legitimate, including ""
  // (an item whose only content is its Metadata block).
  const callerBodyRaw = body["body"];
  if (callerBodyRaw !== undefined && callerBodyRaw !== null && typeof callerBodyRaw !== "string") {
    throw new FailClosed(400, "invalid_body", "body must be a string when present");
  }
  const callerBody: string = typeof callerBodyRaw === "string" ? callerBodyRaw : "";
  const head = optionalString(body, "head") ?? undefined;
  const base = optionalString(body, "base") ?? undefined;
  const hostClaim = optionalString(body, "host");
  const worktreeClaim = optionalString(body, "worktree");

  const nowIso = new Date(deps.now?.() ?? Date.now()).toISOString();

  // The cited session, resolved against the registry (design §4.2).
  // An agent credential's session claim is the ceiling of what the
  // caller may cite, exactly as its lane is on session open.
  let session: SessionRecord | null = null;
  let sessionId: string | null = null;
  const citedRaw = body["session_id"];
  if (citedRaw !== undefined && citedRaw !== null) {
    sessionId = checkedSessionId(citedRaw);
    if (me.kind === "agent" && me.sessionId && me.sessionId !== sessionId) {
      throw new FailClosed(
        403,
        "session_beyond_credential",
        "cited session differs from the agent credential's session",
      );
    }
    if (!deps.sessions) {
      throw new FailClosed(
        503,
        "session_registry_not_configured",
        "this deployment has no session registry configured; failing closed",
      );
    }
    session = await requireOpenSession(deps.sessions, me.bindingId, sessionId);
  }

  // The queue record, when this work is queue-originated (design §5).
  const queue =
    sessionId && deps.queueRecords ? await deps.queueRecords.recordForSession(me.bindingId, sessionId) : null;

  // workspace_ref: the caller's claim, or the strongest locator the
  // request supports — the worktree claim, else the cited session.
  let workspaceFallback: Record<string, unknown> | null = null;
  if (worktreeClaim !== null) {
    workspaceFallback = { kind: "worktree", id: worktreeClaim, repo, branch: null, sha: null };
  } else if (sessionId !== null) {
    workspaceFallback = { kind: "session", id: sessionId, repo, branch: null, sha: null };
  }
  const workspaceRef = workspaceRefClaim(body, workspaceFallback);

  const provenance: Provenance = {
    queue_id: queue?.queueId ?? null,
    scheduled_start: queue?.scheduledStart ?? null,
    picked_up_at: queue?.pickedUpAt ?? null,
    work_started_at: queue?.workStartedAt ?? session?.openedAt ?? null,
    created_at: nowIso,
    session_id: sessionId,
    lane: session?.lane ?? (me.kind === "agent" ? (me.lane ?? null) : null),
    host: hostClaim ?? (me.kind === "service" ? (me.serviceLabel ?? SERVICE_NAME) : null),
    worktree: worktreeClaim,
    model: session?.model ?? null,
    lane_start_issues:
      session && session.laneStartIssues.length > 0
        ? [...session.laneStartIssues]
        : [...(queue?.laneStartIssues ?? [])],
    workspace_ref: workspaceRef,
    auth_subject: me.bindingId,
    stamped_by: `${SERVICE_NAME}@${deps.config.version}`,
  };

  // Parse the caller's block (refuses on malformed), preserve its
  // caller-owned fields, replace provenance in full, sign.
  const parsed = parseMetadataBlock(callerBody);
  const signedProvenance = await signStamp(provenance, signer);
  const block = assembleStampedBlock(parsed.block, signedProvenance);
  const stampedBody = renderBodyWithBlock(parsed, block);

  // The GitHub write, through the hosted rotation point.
  const created = await deps.githubWriter.createItem({
    repo,
    kind,
    title,
    body: stampedBody,
    ...(head ? { head } : {}),
    ...(base ? { base } : {}),
  });

  // The DB insert, in the same request. A failure here never loses
  // the GitHub item: the response reports the miss and ingestion
  // heals it (design §7.3).
  const record: MetadataRecord = {
    repo,
    kind,
    number: created.number,
    url: created.url,
    bindingId: me.bindingId,
    metadata: block,
    extracted: extractKeys(block),
    insertedAt: nowIso,
  };
  let metadataInserted = true;
  let insertError: string | undefined;
  try {
    await deps.metadataStore.insert(record);
  } catch (e) {
    metadataInserted = false;
    insertError = e instanceof Error ? e.message : "metadata insert failed";
  }

  return json(
    {
      repo,
      kind,
      number: created.number,
      url: created.url,
      body: stampedBody,
      metadata: block,
      metadata_inserted: metadataInserted,
      ...(insertError ? { insert_error: insertError } : {}),
    },
    201,
  );
}

export const routes: RouteDef[] = [{ method: "POST", path: "/v1/stamps", door: "stamping", handler: createStamped }];
