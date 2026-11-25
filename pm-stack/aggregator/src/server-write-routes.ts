// The server's write surface: report-record resolution, the
// identity-registry admin + stack-token exchange, and the prompt /
// comment / action route families. Each handler returns null when
// its routes do not match, so server.ts keeps the dispatch order.
// Split out of server.ts (#13052).
import { validateNewAction } from "./actions.ts";
import { checkActionAgainstItem, findTierItem, questionIdForOption } from "./tier-items.ts";
import { authorizeCommentAuthor, validateNewComment } from "./comments.ts";
import { PROMPT_STATUSES, syncBlockHttpResult, validateNewPrompt, type PromptStatus } from "./prompts.ts";
import { readMultipartForm, submissionFieldsFromForm, uploadedFilesFromForm, type PromptAttachment } from "./prompt-attachments.ts";
import { notifyPromptCompletion, promptCompletionTransition } from "./prompt-notify.ts";
import { getAlertsStore } from "./alerts.ts";
import { confirmOutageWrite, rejectOutageWrite } from "./outage-queue.ts";
import { validateNewRoadmapAnnotation } from "./roadmap-annotations.ts";
import { createAccessAuthorEnforcer, enforceAccessAuthor } from "./access-identity.ts";
import { identityMappingAuditEvent, validateIdentityMapping } from "./identity-registry.ts";
import { authorizeWithBreakGlassToken, presentedBreakGlassToken } from "./break-glass.ts";
import { STACK_TOKEN_TTL_SECONDS, mintStackToken, type StackTokenRole } from "./stack-token.ts";
import { resolveBwsSecretLookup } from "./secrets.ts";
import { readJsonBody } from "./request-body.ts";
import { appendReportBlockConflictResolution, appendReportLateUpdate, compareReportBlock, ReportRecordConflictError, readReportRecords, readReportSnapshots, selectCurrentReportRecords, type ReportLateUpdateOperation, type ReportRecordSource } from "./report-records.ts";
import { renderReportReview, reportBlockReviews } from "./report-review.ts";
import { capUnits, cfg, promptsStore, promptAttachmentsStore, tryQueueOutageAction, roadmapAnnotationsStore, actionsStore, commentsStore, eventsStore, reportRecordsLocation, identityRegistryStore, breakGlassTokenStore, recordStreamEvent, outageQueueStore, source } from "./server-wiring.ts";
import { json, html } from "./server-respond.ts";
import { applyViewAction } from "./server-contexts.ts";

export async function handleReportsRead(req: Request, path: string): Promise<Response> {
  try {
    const records = await readReportRecords(reportRecordsLocation);
    const snapshots = await readReportSnapshots(reportRecordsLocation);
    const current = selectCurrentReportRecords(records);
    if (path === "/api/reports.json") {
      return json({
        records: current,
        conflicts: reportBlockReviews(records, current).map((review) => ({
          periodId: review.periodId,
          blockId: review.blockId,
          storedRecordId: review.storedRecord.id,
          renderedRecordId: review.renderedRecord.id,
          comparison: review.comparison,
        })),
      });
    }
    return html(
      renderReportReview(
        reportBlockReviews(records, current),
        records.length,
        new Date(),
        current,
        new Set(snapshots.map((snapshot) => snapshot.record.period.id)),
      ),
    );
  } catch {
    if (path === "/api/reports.json") return json({ error: "report records unavailable" }, 503);
    return html(`<h1>Report review unavailable</h1><p>Could not read retained report records.</p>`, 503);
  }
}

export async function handleReportWriteRoutes(req: Request, path: string): Promise<Response | null> {
  if (req.method === "POST" && path === "/api/reports/resolve") {
    const access = await enforceAccessAuthor(req, { kind: "drew", id: "drew", label: "Drew" });
    if (!access.ok || !access.verified || access.author.kind !== "drew") {
      return json({ error: "verified Access owner required" }, 403);
    }
    let body: Record<string, unknown>;
    try {
      const parsed = await readJsonBody(req);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
        return json({ error: "invalid resolution body" }, 400);
      body = parsed as Record<string, unknown>;
    } catch {
      return json({ error: "invalid resolution body" }, 400);
    }
    const storedRecordId = typeof body.storedRecordId === "string" ? body.storedRecordId : "";
    const renderedRecordId = typeof body.renderedRecordId === "string" ? body.renderedRecordId : "";
    const blockId = typeof body.blockId === "string" ? body.blockId : "";
    const strategy = body.strategy;
    if (
      !storedRecordId ||
      !renderedRecordId ||
      !blockId ||
      typeof strategy !== "string" ||
      !["stored", "rendered", "edited", "combined"].includes(strategy)
    ) {
      return json({ error: "storedRecordId, renderedRecordId, blockId and a valid strategy are required" }, 400);
    }
    if (body.resolvedText !== undefined && typeof body.resolvedText !== "string") {
      return json({ error: "resolvedText must be text" }, 400);
    }
    let records;
    try {
      records = await readReportRecords(reportRecordsLocation);
    } catch {
      return json({ error: "report records unavailable" }, 503);
    }
    const storedRecord = records.find((record) => record.id === storedRecordId);
    const renderedRecord = records.find((record) => record.id === renderedRecordId);
    if (!storedRecord || !renderedRecord) return json({ error: "unknown report record" }, 404);
    if (
      storedRecord.period.id !== renderedRecord.period.id ||
      storedRecord.templateVersion !== renderedRecord.templateVersion
    ) {
      return json({ error: "compared records must have the same period and template" }, 400);
    }
    const storedBlock = storedRecord.blocks.find((block) => block.id === blockId);
    const renderedBlock = renderedRecord.blocks.find((block) => block.id === blockId);
    if (!storedBlock || !renderedBlock) return json({ error: "compared block is missing" }, 400);
    const comparison = compareReportBlock(storedBlock.text, renderedBlock.text);
    if (!comparison.conflict) return json({ error: "report block has no substantive conflict" }, 400);
    const selectedPair = reportBlockReviews(records, selectCurrentReportRecords(records)).some(
      (review) =>
        review.blockId === blockId &&
        review.storedRecord.id === storedRecordId &&
        review.renderedRecord.id === renderedRecordId,
    );
    const selectedText =
      strategy === "stored" ? storedBlock.text : strategy === "rendered" ? renderedBlock.text : body.resolvedText;
    const exactRetry = records.some((candidate) =>
      candidate.blocks.some(
        (block) =>
          block.id === blockId &&
          block.text === selectedText &&
          block.resolution?.sourceRecordId === storedRecordId &&
          block.resolution.comparedRecordId === renderedRecordId &&
          block.resolution.strategy === strategy,
      ),
    );
    if (!selectedPair && !exactRetry) {
      return json({ error: "compared records are not the current review selection" }, 409);
    }
    try {
      const result = await appendReportBlockConflictResolution(
        reportRecordsLocation,
        storedRecord,
        blockId,
        comparison,
        strategy as "stored" | "rendered" | "edited" | "combined",
        new Date().toISOString(),
        typeof body.resolvedText === "string" ? body.resolvedText : undefined,
        renderedRecordId,
      );
      return json({ inserted: result.inserted, record: result.record });
    } catch (error) {
      const message = error instanceof Error ? error.message : "report resolution failed";
      const status =
        error instanceof ReportRecordConflictError || message.startsWith("stale report block comparison:")
          ? 409
          : /^(report block comparison|compared report record|resolvedAt must be later|report operation |(?:stored|rendered|edited|combined) report block resolution)/.test(
                message,
              )
            ? 400
            : 500;
      return json({ error: message }, status);
    }
  }

  if (req.method === "POST" && path === "/api/reports/late-update") {
    const access = await enforceAccessAuthor(req, { kind: "drew", id: "drew", label: "Drew" });
    if (!access.ok || !access.verified || access.author.kind !== "drew") {
      return json({ error: "verified Access owner required" }, 403);
    }
    let body: Record<string, unknown>;
    try {
      const parsed = await readJsonBody(req);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
        return json({ error: "invalid late update body" }, 400);
      body = parsed as Record<string, unknown>;
    } catch {
      return json({ error: "invalid late update body" }, 400);
    }
    const reportRecordId = typeof body.reportRecordId === "string" ? body.reportRecordId : "";
    const operation = body.operation;
    const blockId = typeof body.blockId === "string" ? body.blockId : "";
    const text = typeof body.text === "string" ? body.text : "";
    const relatedAt = typeof body.relatedAt === "string" ? body.relatedAt : "";
    const lateOperations = ["quote", "append", "banner", "event-log", "feedback"];
    if (
      !reportRecordId ||
      !blockId ||
      !text.trim() ||
      !relatedAt ||
      typeof operation !== "string" ||
      !lateOperations.includes(operation)
    ) {
      return json({ error: "reportRecordId, a permitted operation, blockId, text and relatedAt are required" }, 400);
    }
    let source: ReportRecordSource | undefined;
    if (body.source !== undefined) {
      if (!body.source || typeof body.source !== "object" || Array.isArray(body.source))
        return json({ error: "invalid late update source" }, 400);
      const candidate = body.source as Record<string, unknown>;
      if (
        typeof candidate.id !== "string" ||
        typeof candidate.uri !== "string" ||
        typeof candidate.observedAt !== "string"
      ) {
        return json({ error: "late update source requires id, uri and observedAt" }, 400);
      }
      source = { id: candidate.id, uri: candidate.uri, observedAt: candidate.observedAt };
    }
    let records;
    try {
      records = await readReportRecords(reportRecordsLocation);
    } catch {
      return json({ error: "report records unavailable" }, 503);
    }
    const current = records.find((record) => record.id === reportRecordId);
    if (!current) return json({ error: "unknown report record" }, 404);
    try {
      const result = await appendReportLateUpdate(
        reportRecordsLocation,
        current,
        operation as ReportLateUpdateOperation,
        { id: blockId, text, relatedAt },
        new Date().toISOString(),
        source,
      );
      return json({ inserted: result.inserted, record: result.record });
    } catch (error) {
      const message = error instanceof Error ? error.message : "late report update failed";
      const status =
        error instanceof ReportRecordConflictError || message.startsWith("stale report block comparison:")
          ? 409
          : /^(report operation |quote report update|feedback report update|each late report update|block relatedAt|source observedAt|late update source|duplicate report block)/.test(
                message,
              )
            ? 400
            : 500;
      return json({ error: message }, status);
    }
  }
  return null;
}

export async function handleIdentityRoutes(req: Request, path: string): Promise<Response | null> {
  // #9604: role mappings are managed only by a verified Access owner.
  // The asserted author here is deliberately fixed; request-body identity
  // or role claims never authorize registry changes.
  if (
    path === "/api/admin/identity-registry" &&
    (req.method === "GET" || req.method === "POST" || req.method === "DELETE")
  ) {
    const access = await enforceAccessAuthor(req, { kind: "drew", id: "drew", label: "Drew" });
    if (!access.ok) return json({ error: access.error }, access.status);
    let auditActor: string;
    if (access.verified && access.author.kind === "drew") {
      // Access-verified owner proceeds exactly as before; a presented
      // break-glass token is never touched on this path.
      auditActor = access.identity?.sub || access.identity?.email || access.author.id;
    } else {
      // #9608: the Access/IdP-outage path. A presented single-use
      // break-glass token is consumed atomically (with its audit
      // event) and authorizes exactly this one admin request;
      // anything invalid — or no token at all — fails closed.
      const presented = presentedBreakGlassToken(req);
      if (!presented) return json({ error: "verified Access owner required" }, 403);
      const grant = await authorizeWithBreakGlassToken({
        store: breakGlassTokenStore,
        events: eventsStore,
        token: presented,
      });
      if (!grant.ok) return json({ error: grant.error }, 403);
      auditActor = grant.auditActor;
    }
    if (req.method === "GET") return json({ registry: await identityRegistryStore.list() });
    try {
      const body = await readJsonBody(req);
      if (req.method === "POST") {
        const parsed = validateIdentityMapping(body);
        if (!parsed.ok) return json({ error: parsed.error }, 400);
        const result = await identityRegistryStore.set(parsed.identity, parsed.role, auditActor);
        await eventsStore.record(
          identityMappingAuditEvent({
            identity: parsed.identity,
            previousRole: result.previousRole,
            role: parsed.role,
            revision: result.registry.revision,
            actor: auditActor,
          }),
        );
        return json({ registry: result.registry, previousRole: result.previousRole });
      }
      const identity =
        typeof body === "object" && body !== null ? (body as { identity?: unknown }).identity : undefined;
      if (
        typeof identity !== "string" ||
        identity.trim() === "" ||
        identity.trim().length > 200 ||
        /\s/.test(identity.trim())
      ) {
        return json({ error: "identity must be a non-empty whitespace-free string (max 200 chars)" }, 400);
      }
      const result = await identityRegistryStore.remove(identity.trim());
      if (result.previousRole !== null) {
        await eventsStore.record(
          identityMappingAuditEvent({
            identity: identity.trim(),
            previousRole: result.previousRole,
            role: null,
            revision: result.registry.revision,
            actor: auditActor,
          }),
        );
      }
      return json({ registry: result.registry, previousRole: result.previousRole });
    } catch (error) {
      return json(
        { error: `identity registry operation failed: ${error instanceof Error ? error.message : error}` },
        500,
      );
    }
  }

  // #9605: exchange the already-verified Access identity for a short-lived
  // layer-scoped token. The target and action are request metadata; the
  // principal and role always come from verified Access + the registry.
  if (req.method === "POST" && path === "/api/stack-token/exchange") {
    const access = await enforceAccessAuthor(req, { kind: "agent", id: "exchange", label: "exchange" });
    if (!access.ok) return json({ error: access.error }, access.status);
    if (!access.verified || !access.identity) return json({ error: "verified Access identity required" }, 403);
    const secret = cfg.stackTokenSecret;
    if (!secret) return json({ error: "stack token exchange is not configured" }, 503);
    const identity = access.identity.sub || access.identity.email;
    const role: StackTokenRole =
      access.author.kind === "drew" ? "owner" : await identityRegistryStore.roleFor(identity);
    if (role === "read-only") return json({ error: "identity role cannot mint write tokens" }, 403);
    try {
      const body = await readJsonBody(req);
      const target =
        typeof body === "object" && body !== null && typeof (body as { target?: unknown }).target === "string"
          ? (body as { target: string }).target.trim()
          : "";
      const action =
        typeof body === "object" && body !== null && typeof (body as { action?: unknown }).action === "string"
          ? (body as { action: string }).action.trim()
          : "";
      if (!["kaneo", "pad", "grist"].includes(target))
        return json({ error: "target must be kaneo, pad, or grist" }, 400);
      if (!/^[A-Za-z][A-Za-z0-9._:-]{0,63}$/.test(action)) return json({ error: "action is invalid" }, 400);
      const token = mintStackToken({ sub: identity, role, aud: target, act: action }, secret);
      return json({ token, tokenType: "Bearer", audience: target, expiresIn: STACK_TOKEN_TTL_SECONDS });
    } catch (error) {
      return json({ error: `stack token exchange failed: ${error instanceof Error ? error.message : error}` }, 400);
    }
  }
  return null;
}

export async function handleWriteRoutes(req: Request, url: URL, path: string, remoteAddress?: string): Promise<Response | null> {
  // --- Dashboard comment writes (#906 §2) --------------------------------
  if (req.method === "POST" && path === "/api/roadmap/annotations") {
    try {
      const parsed = validateNewRoadmapAnnotation(await readJsonBody(req));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      // #1253 slice: when Access is configured, authorship derives from
      // the verified JWT, never the caller-supplied author.
      const annotationAccess = await enforceAccessAuthor(req, parsed.value.author);
      if (!annotationAccess.ok) return json({ error: annotationAccess.error }, annotationAccess.status);
      parsed.value.author = annotationAccess.author;
      const rows = await source.roadmap().catch(() => []);
      if (!rows.some((row) => row.subject === parsed.value.objectRef)) {
        return json({ error: "objectRef is not a current roadmap row" }, 404);
      }
      try {
        const annotation = await roadmapAnnotationsStore.add(parsed.value);
        return json({ annotation }, 201);
      } catch (err) {
        return json({ error: `annotation persistence failed: ${err instanceof Error ? err.message : err}` }, 500);
      }
    } catch (err) {
      return json({ error: `invalid annotation: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (req.method === "POST" && path === "/api/roadmap/annotations/alias") {
    try {
      const body = (await readJsonBody(req)) as { aliasRef?: unknown; stableRef?: unknown };
      if (typeof body.aliasRef !== "string" || typeof body.stableRef !== "string") {
        return json({ error: "aliasRef and stableRef are required" }, 400);
      }
      const rows = await source.roadmap().catch(() => []);
      if (!rows.some((row) => row.subject === body.aliasRef)) {
        return json({ error: "aliasRef is not a current roadmap row" }, 404);
      }
      if (!(await roadmapAnnotationsStore.forObject(body.stableRef)).length) {
        return json({ error: "stableRef has no existing annotation history" }, 404);
      }
      await roadmapAnnotationsStore.addAlias(body.aliasRef, body.stableRef);
      return json({ aliases: await roadmapAnnotationsStore.aliasEntries() });
    } catch (err) {
      return json({ error: `invalid annotation alias: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }

  if (req.method === "POST" && path === "/api/prompt") {
    // #10036 S7 (design #9539 §3.2/§6): a multipart submission carries
    // file parts under "attachments"; its text fields are shaped into
    // the same JSON body the validator accepts, the files are persisted
    // to the attachments dir first, and the record carries their
    // storage refs. The JSON path is unchanged.
    let savedAttachments: PromptAttachment[] = [];
    let parsed: ReturnType<typeof validateNewPrompt>;
    if ((req.headers.get("content-type") ?? "").startsWith("multipart/form-data")) {
      let form: FormData;
      try {
        form = await readMultipartForm(req);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        return json({ error: `invalid multipart body: ${message}` }, message === "body too large" ? 413 : 400);
      }
      try {
        parsed = validateNewPrompt(submissionFieldsFromForm(form));
      } catch (err) {
        return json({ error: err instanceof Error ? err.message : String(err) }, 400);
      }
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      try {
        savedAttachments = await promptAttachmentsStore.saveAll(await uploadedFilesFromForm(form));
      } catch (err) {
        return json({ error: err instanceof Error ? err.message : String(err) }, 400);
      }
      if (savedAttachments.length > 0) parsed.value.attachments = savedAttachments;
    } else {
      parsed = validateNewPrompt(await readJsonBody(req).catch(() => null));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
    }
    try {
      const record = await promptsStore.add(parsed.value);
      // #10033 S3 (design §3.5/§3.7): a sync-block submission waits
      // in-band for the answer and returns it in the response body,
      // bounded by the configured timeout. Async returns the record
      // immediately, and so does sync-ws — its callers then open the
      // /ws/prompt/:id channel named by the returned id.
      if (parsed.value.mode === "sync-block") {
        const wait = await promptsStore.waitForAnswer(record.id, { timeoutMs: cfg.promptSyncTimeoutMs });
        const result = syncBlockHttpResult(wait, cfg.promptSyncTimeoutMs);
        return json(result.body, result.status);
      }
      return json({ prompt: record }, 201);
    } catch (err) {
      // A submission that failed to persist must not orphan its blobs.
      for (const attachment of savedAttachments) {
        await promptAttachmentsStore.remove(attachment.ref).catch(() => {});
      }
      return json({ error: `prompt persistence failed: ${err instanceof Error ? err.message : err}` }, 500);
    }
  }
  if (req.method === "GET" && path === "/api/prompt/unread-count") {
    const count = await promptsStore.unreadCount();
    return json({ count, unread: count, total: count });
  }
  if (req.method === "GET" && path === "/api/prompt/threads") {
    return json({ threads: await promptsStore.threads() });
  }
  if (req.method === "GET" && path.startsWith("/api/prompt/thread/")) {
    const threadId = decodeURIComponent(path.slice("/api/prompt/thread/".length));
    return json({ threadId, prompts: await promptsStore.thread(threadId) });
  }
  if (req.method === "GET" && path === "/api/prompt") {
    const status = url.searchParams.get("status");
    const page = url.searchParams.get("page") ?? undefined;
    const threadId = url.searchParams.get("threadId") ?? undefined;
    const replyTo = url.searchParams.get("replyTo") ?? undefined;
    const archivedParam = url.searchParams.get("archived");
    const includeArchived =
      url.searchParams.get("includeArchived") === "1" || url.searchParams.get("includeArchived") === "true";
    const archived = archivedParam === "true" ? true : archivedParam === "false" ? false : undefined;
    const filter: Parameters<typeof promptsStore.list>[0] = {
      page,
      status: PROMPT_STATUSES.includes(status as PromptStatus) ? (status as PromptStatus) : undefined,
      threadId,
      replyTo,
      archived,
      includeArchived,
    };
    const prompts = await promptsStore.list(filter);
    // newest first
    prompts.sort((a, b) => (a.createdAt < b.createdAt ? 1 : a.createdAt > b.createdAt ? -1 : 0));
    return json({ prompts });
  }
  if (req.method === "GET" && path.startsWith("/api/prompt/") && path.includes("/attachments/")) {
    // #10036 S7: fetch one attachment blob by its storage ref. The ref
    // must be carried by the named prompt's record — a ref alone is not
    // a capability — and the store revalidates the ref's shape before
    // resolving it. Served as a download (never inline) with nosniff,
    // so an uploaded HTML file cannot render as a stack page.
    const rest = decodeURIComponent(path.slice("/api/prompt/".length));
    const segments = rest.split("/");
    if (segments.length !== 3 || segments[1] !== "attachments") {
      return json({ error: "prompt not found" }, 404);
    }
    const [id, , ref] = segments;
    const prompt = await promptsStore.get(id);
    if (!prompt) return json({ error: "prompt not found" }, 404);
    if (!prompt.attachments.some((a) => a.ref === ref)) {
      return json({ error: "attachment not found" }, 404);
    }
    const found = await promptAttachmentsStore.read(ref);
    if (!found) return json({ error: "attachment not found" }, 404);
    return new Response(found.bytes, {
      headers: {
        "content-type": found.meta.contentType,
        "content-length": String(found.bytes.byteLength),
        "content-disposition": `attachment; filename="${found.meta.name.replace(/"/g, "")}"; filename*=UTF-8''${encodeURIComponent(found.meta.name)}`,
        "x-content-type-options": "nosniff",
        "cache-control": "private, no-store",
      },
    });
  }
  if (req.method === "GET" && path.startsWith("/api/prompt/")) {
    const rest = decodeURIComponent(path.slice("/api/prompt/".length));
    if (rest === "thread" || rest.startsWith("thread/")) {
      // handled above via query; fall through to 404
    }
    const id = rest;
    const prompt = await promptsStore.get(id);
    if (!prompt) return json({ error: "prompt not found" }, 404);
    return json({
      id: prompt.id,
      status: prompt.status,
      answer: prompt.answer,
      error: prompt.error,
      threadId: prompt.threadId,
      archived: prompt.archived,
      read: prompt.read,
      replyTo: prompt.replyTo,
      refs: prompt.refs,
      attachments: prompt.attachments,
    });
  }
  if (req.method === "PATCH" && path.startsWith("/api/prompt/")) {
    const id = decodeURIComponent(path.slice("/api/prompt/".length));
    let body: unknown = null;
    try {
      body = await readJsonBody(req);
    } catch {
      body = null;
    }
    const b = body && typeof body === "object" && !Array.isArray(body) ? (body as Record<string, unknown>) : {};
    const patch: Parameters<typeof promptsStore.update>[1] = {};
    if (typeof b.status === "string" && PROMPT_STATUSES.includes(b.status as PromptStatus))
      patch.status = b.status as PromptStatus;
    if (typeof b.answer === "string") patch.answer = b.answer;
    if (typeof b.error === "string") patch.error = b.error;
    if (typeof b.archived === "boolean") patch.archived = b.archived;
    if (typeof b.read === "boolean") patch.read = b.read;
    if (Array.isArray(b.refs) && b.refs.every((x) => typeof x === "string")) patch.refs = b.refs as string[];
    if (b.replyTo === null || typeof b.replyTo === "string") patch.replyTo = b.replyTo as string | null;
    const before = await promptsStore.get(id);
    const updated = await promptsStore.update(id, patch);
    if (!updated) return json({ error: "prompt not found" }, 404);
    // #10035 S6 (design #9539 §4): push on async completion, opt-in
    // per submission (the record's notify flag). Answers land only
    // through this route, so the terminal transition is detected
    // here; the send is fire-and-forget — a push failure must never
    // fail the answer itself.
    if (promptCompletionTransition(before, updated)) {
      void (async () => {
        const store = await getAlertsStore();
        if (store) await notifyPromptCompletion(store, updated, cfg.publicOrigin);
      })().catch((err) => {
        console.error(
          `[prompt-notify] completion push failed for ${updated.id}: ${err instanceof Error ? err.message : String(err)}`,
        );
      });
    }
    return json({ prompt: updated });
  }
  if (req.method === "POST" && path === "/api/comments") {
    try {
      const parsed = validateNewComment(await readJsonBody(req));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      // #1253 slice: when Access is configured, authorship derives from
      // the verified JWT, never the caller-supplied author.
      const commentAccess = await enforceAccessAuthor(req, parsed.value.author);
      if (!commentAccess.ok) return json({ error: commentAccess.error }, commentAccess.status);
      parsed.value.author = commentAccess.author;
      // #941: a privileged author additionally requires the operator
      // token when one is configured (fail closed on a secret-store
      // outage); with no secret configured the write stays unverified.
      // Runs on the Access-derived author above, so the two gates
      // compose: identity first, privilege second.
      const authorization = authorizeCommentAuthor(
        parsed.value.author,
        req.headers.get("authorization"),
        await resolveBwsSecretLookup("pm-aggregator.comments-operator-token"),
      );
      if (!authorization.ok) return json({ error: authorization.error }, authorization.status);
      // #1509: a persistence failure is a server error, not bad input -
      // only the validation/body-parse path above earns a 400.
      let comment;
      try {
        comment = await commentsStore.add(parsed.value, {
          authorVerified: authorization.authorVerified,
          // #1253 split 4 (#9606): "verified" only when the author came
          // from the Access JWT above, never from the request body.
          provenance: commentAccess.verified ? "verified" : "asserted-legacy",
        });
      } catch (err) {
        return json({ error: `comment persistence failed: ${err instanceof Error ? err.message : err}` }, 500);
      }
      return json({ comment }, 201);
    } catch (err) {
      return json({ error: `invalid comment: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (req.method === "POST" && path === "/api/comments/triage") {
    try {
      const body = (await readJsonBody(req)) as { ids?: unknown };
      const ids = Array.isArray(body.ids) ? body.ids.filter((x): x is string => typeof x === "string") : [];
      return json({ triaged: await commentsStore.markTriaged(ids) });
    } catch (err) {
      return json({ error: `invalid triage: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (req.method === "POST" && path === "/api/comments/github-result") {
    try {
      const body = (await readJsonBody(req)) as { id?: unknown; url?: unknown; error?: unknown };
      if (typeof body.id !== "string") return json({ error: "id required" }, 400);
      const updated = await commentsStore.markGithubResult(body.id, {
        url: typeof body.url === "string" ? body.url : undefined,
        error: typeof body.error === "string" ? body.error : undefined,
      });
      return updated ? json({ comment: updated }) : json({ error: "no such github comment" }, 404);
    } catch (err) {
      return json({ error: `invalid result: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  // --- Dashboard action writes (#1152/#1154/#1156) -----------------------
  // The triage write path: forms POST DashActions here; the fleet
  // consumers (orchestration pass) poll state=new and
  // report outcomes to /api/actions/result. The dashboard itself never
  // writes GitHub, the Project, or a session server.
  if (req.method === "POST" && path === "/api/actions") {
    try {
      const parsed = validateNewAction(await readJsonBody(req));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      // #1253 slice: gate every action write, including non-triage and
      // preference actions, on the configured Access identity.
      const actionAccess = await enforceAccessAuthor(req, parsed.value.author);
      if (!actionAccess.ok) {
        // #9608 (design #1253 section 6): during an Access/IdP outage
        // an eligible write from the stack-local network queues as an
        // unverified claim (provenance: outage-queued) instead of
        // failing — held out of the actions store until its claimant
        // confirms it with a fresh login. Anything else keeps the
        // enforcer's refusal, byte-identical to before.
        try {
          const queued = await tryQueueOutageAction(parsed.value, actionAccess.status, remoteAddress);
          if (queued) {
            return json(
              {
                queued: true,
                provenance: "outage-queued",
                queue: queued.record,
                ...(queued.created ? {} : { deduped: true }),
              },
              202,
            );
          }
        } catch (err) {
          return json({ error: `outage queueing failed: ${err instanceof Error ? err.message : err}` }, 500);
        }
        return json({ error: actionAccess.error }, actionAccess.status);
      }
      parsed.value.author = actionAccess.author;
      // #1291/#1293: checks that need the currently published item —
      // option/survey membership, required-question coverage, proposal
      // identity, the access/policy auto ban. Enforced only when the
      // aggregator actually holds a section; with none, the store
      // stays the permissive boundary of the triage slice and the
      // consumer rejects what it cannot apply.
      if (parsed.value.kind === "triage") {
        const section = await source.orchestration().catch(() => null);
        let item: ReturnType<typeof findTierItem>;
        if (section) {
          item = findTierItem(section, parsed.value.objectRef);
          const problem = checkActionAgainstItem(parsed.value, item, Date.now());
          if (problem) return json({ error: problem }, 400);
        }
        if (parsed.value.verb === "answer" && parsed.value.payload.optionId) {
          parsed.value.payload.questionId = questionIdForOption(item, parsed.value.payload.optionId) ?? "$options";
        }
      }
      // #1509: persistence failure -> 5xx (see /api/comments above).
      let added;
      try {
        added = await actionsStore.add(parsed.value);
      } catch (err) {
        return json({ error: `action persistence failed: ${err instanceof Error ? err.message : err}` }, 500);
      }
      const { action, created } = added;
      if (added.duplicateOf) {
        return json(
          {
            error: `duplicate answer recorded as ${action.id}; it points to existing answer action ${added.duplicateOf}`,
            action,
            duplicateOf: added.duplicateOf,
          },
          409,
        );
      }
      // A client-token replay of an already-persisted duplicate attempt
      // keeps the rejected semantics instead of reading as a fresh
      // "sent · pending" success (Codex P2 on PR #4024).
      if (!created && action.duplicateOf) {
        return json(
          {
            error: `duplicate answer recorded as ${action.id}; it points to existing answer action ${action.duplicateOf}`,
            action,
            duplicateOf: action.duplicateOf,
          },
          409,
        );
      }
      // #1276: saved-view preference verbs are applied by the
      // aggregator itself to the views store and marked applied (or
      // rejected, with the reason) in the same request — the action
      // record stays the audit trail for every view change.
      if (
        created &&
        action.kind === "preference" &&
        (action.verb === "save-view" ||
          action.verb === "rename-view" ||
          action.verb === "delete-view" ||
          action.verb === "set-default-view")
      ) {
        try {
          const note = await applyViewAction(action);
          const updated = await actionsStore.markResult(action.id, {
            outcome: "applied",
            byRun: "aggregator-views",
            note,
          });
          return json({ action: updated ?? action }, 201);
        } catch (err) {
          const updated = await actionsStore.markResult(action.id, {
            outcome: "rejected",
            byRun: "aggregator-views",
            note: err instanceof Error ? err.message : String(err),
          });
          return json({ action: updated ?? action, error: err instanceof Error ? err.message : String(err) }, 400);
        }
      }
      return json(created ? { action } : { action, deduped: true }, created ? 201 : 200);
    } catch (err) {
      return json({ error: `invalid action: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  // #1277: bulk triage — one mediated batch, validated and appended
  // item by item under the exact single-action rules above. Every
  // accepted item becomes its OWN attributed audit entry (never one
  // opaque batch record), so the batch is recoverable from the
  // entries themselves; every rejected item is reported by ref with
  // its error and appends nothing, so a partial batch never silently
  // drops work — the failed items simply remain in Needs-you.
  if (req.method === "POST" && path === "/api/actions/batch") {
    try {
      const body = (await readJsonBody(req)) as { actions?: unknown };
      if (!Array.isArray(body.actions) || body.actions.length === 0 || body.actions.length > 100) {
        return json({ error: "actions must be an array of 1-100 action objects" }, 400);
      }
      const entries: unknown[] = body.actions;
      // The published-item checks (#1291/#1293) need the section once,
      // not once per item.
      const section = entries.some(
        (a) => typeof a === "object" && a !== null && (a as { kind?: unknown }).kind === "triage",
      )
        ? await source.orchestration().catch(() => null)
        : null;
      const results: {
        index: number;
        objectRef: string | null;
        ok: boolean;
        action?: unknown;
        deduped?: boolean;
        // #9608: set when the item outage-queued instead of recording.
        queued?: boolean;
        queueId?: string;
        error?: string;
      }[] = [];
      // One enforcer for the whole batch: the request's JWT is
      // verified once, not once per action (see access-identity.ts).
      const enforceBatchAuthor = createAccessAuthorEnforcer(req);
      for (const [index, raw] of entries.entries()) {
        const objectRef =
          typeof raw === "object" && raw !== null && typeof (raw as { objectRef?: unknown }).objectRef === "string"
            ? (raw as { objectRef: string }).objectRef
            : null;
        const parsed = validateNewAction(raw);
        if (!parsed.ok) {
          results.push({ index, objectRef, ok: false, error: parsed.error });
          continue;
        }
        const batchAccess = await enforceBatchAuthor(parsed.value.author);
        if (!batchAccess.ok) {
          // #9608: the single-action outage posture, per item — an
          // eligible item queues as an unverified claim; the rest of
          // the batch is unaffected.
          try {
            const queued = await tryQueueOutageAction(parsed.value, batchAccess.status, remoteAddress);
            if (queued) {
              results.push({
                index,
                objectRef,
                ok: true,
                queued: true,
                queueId: queued.record.id,
                ...(queued.created ? {} : { deduped: true }),
              });
              continue;
            }
          } catch (err) {
            results.push({
              index,
              objectRef,
              ok: false,
              error: `outage queueing failed: ${err instanceof Error ? err.message : err}`,
            });
            continue;
          }
          results.push({ index, objectRef, ok: false, error: batchAccess.error });
          continue;
        }
        parsed.value.author = batchAccess.author;
        if (parsed.value.kind === "triage" && section) {
          const problem = checkActionAgainstItem(
            parsed.value,
            findTierItem(section, parsed.value.objectRef),
            Date.now(),
          );
          if (problem) {
            results.push({ index, objectRef, ok: false, error: problem });
            continue;
          }
        }
        try {
          const { action, created } = await actionsStore.add(parsed.value);
          results.push({ index, objectRef, ok: true, action, ...(created ? {} : { deduped: true }) });
        } catch (err) {
          results.push({
            index,
            objectRef,
            ok: false,
            error: `action persistence failed: ${err instanceof Error ? err.message : err}`,
          });
        }
      }
      const recorded = results.filter((r) => r.ok).length;
      return json({ results, recorded, failed: results.length - recorded }, 200);
    } catch (err) {
      return json({ error: `invalid batch: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (req.method === "POST" && path === "/api/actions/result") {
    try {
      const body = (await readJsonBody(req)) as { id?: unknown; outcome?: unknown; byRun?: unknown; note?: unknown };
      if (typeof body.id !== "string") return json({ error: "id required" }, 400);
      if (body.outcome !== "applied" && body.outcome !== "rejected") {
        return json({ error: 'outcome must be "applied" | "rejected"' }, 400);
      }
      if (typeof body.byRun !== "string" || body.byRun.trim() === "") return json({ error: "byRun required" }, 400);
      const result = await actionsStore.markResultTransition(body.id, {
        outcome: body.outcome,
        byRun: body.byRun,
        note: typeof body.note === "string" ? body.note : undefined,
      });
      if (result?.transitioned) {
        const updated = result.action;
        // #1739: the action-result route is one of the event log's
        // writers — the outcome lands in the cross-stack stream.
        // Fire-and-forget: the response never waits on, and is never
        // failed by, the event append.
        recordStreamEvent({
          actor: updated.appliedByRun ?? capUnits(body.byRun, 200),
          kind: body.outcome === "applied" ? "action-applied" : "action-rejected",
          ref: updated.objectRef,
          summary: capUnits(
            `${updated.kind}/${updated.verb} ${body.outcome} by ${body.byRun}` +
              (typeof body.note === "string" && body.note ? ` — ${body.note}` : ""),
            500,
          ),
        });
      }
      const updated = result?.action;
      return updated ? json({ action: updated }) : json({ error: "no such action" }, 404);
    } catch (err) {
      return json({ error: `invalid result: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  // --- Outage-queued action writes (#9608, design #1253 section 6) ---
  // Queued writes appear ONLY on this read surface, stamped
  // outage-queued with the claimed author kept a claim — never in
  // /api/actions.json — until the claimant confirms one with a fresh
  // verified login (confirm) or it is rejected (reject).
  if (path === "/api/actions/outage-queue.json") {
    const state = url.searchParams.get("state");
    if (state && state !== "queued" && state !== "confirmed" && state !== "rejected") {
      return json({ error: "state must be one of: queued, confirmed, rejected" }, 400);
    }
    let list = await outageQueueStore.list();
    if (state) list = list.filter((w) => w.state === state);
    return json({ queue: list });
  }
  if (req.method === "POST" && path === "/api/actions/outage-queue/confirm") {
    try {
      const body = (await readJsonBody(req)) as { id?: unknown };
      if (typeof body.id !== "string" || body.id === "") return json({ error: "id required" }, 400);
      const record = await outageQueueStore.get(body.id);
      if (!record) return json({ error: "no such outage-queued write" }, 404);
      // Confirmation IS a fresh login: the enforcer must succeed with
      // a verified identity. The claimed author rides as the asserted
      // author, so a verified non-owner presenting an owner claim is
      // refused by the enforcer itself, exactly as on a fresh write.
      const access = await enforceAccessAuthor(req, record.claimedAuthor);
      if (!access.ok) return json({ error: access.error }, access.status);
      if (!access.verified) {
        return json({ error: "identity confirmation requires a fresh verified login" }, 403);
      }
      const result = await confirmOutageWrite({
        store: outageQueueStore,
        events: eventsStore,
        id: record.id,
        derivedAuthor: access.author,
        release: async (action) => {
          // The released write passes the same published-item checks
          // as a fresh POST /api/actions (see that route): the item
          // may have changed while the write sat in the queue.
          if (action.kind === "triage") {
            const section = await source.orchestration().catch(() => null);
            let item: ReturnType<typeof findTierItem>;
            if (section) {
              item = findTierItem(section, action.objectRef);
              const problem = checkActionAgainstItem(action, item, Date.now());
              if (problem) return { ok: false as const, status: 400, error: problem };
            }
            if (action.verb === "answer" && action.payload.optionId) {
              action.payload.questionId = questionIdForOption(item, action.payload.optionId) ?? "$options";
            }
          }
          try {
            const added = await actionsStore.add(action);
            if (added.duplicateOf) {
              return {
                ok: false as const,
                status: 409,
                error: `duplicate answer recorded as ${added.action.id}; it points to existing answer action ${added.duplicateOf}`,
              };
            }
            return { ok: true as const, actionId: added.action.id };
          } catch (err) {
            return {
              ok: false as const,
              status: 500,
              error: `action persistence failed: ${err instanceof Error ? err.message : err}`,
            };
          }
        },
      });
      if (!result.ok) return json({ error: result.error }, result.status);
      return json({ confirmed: true, queue: result.record, actionId: result.actionId });
    } catch (err) {
      return json({ error: `invalid confirmation: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (req.method === "POST" && path === "/api/actions/outage-queue/reject") {
    try {
      const body = (await readJsonBody(req)) as { id?: unknown };
      if (typeof body.id !== "string" || body.id === "") return json({ error: "id required" }, 400);
      const record = await outageQueueStore.get(body.id);
      if (!record) return json({ error: "no such outage-queued write" }, 404);
      // Same fresh-login gate as confirm; rejectOutageWrite owns the
      // claimant/owner authorization for the rejection itself.
      const access = await enforceAccessAuthor(req, record.claimedAuthor);
      if (!access.ok) return json({ error: access.error }, access.status);
      if (!access.verified) {
        return json({ error: "rejecting a queued write requires a fresh verified login" }, 403);
      }
      const result = await rejectOutageWrite({
        store: outageQueueStore,
        events: eventsStore,
        id: record.id,
        derivedAuthor: access.author,
      });
      if (!result.ok) return json({ error: result.error }, result.status);
      return json({ rejected: true, queue: result.record, byOwner: result.byOwner });
    } catch (err) {
      return json({ error: `invalid rejection: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  return null;
}
