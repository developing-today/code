// Needs-Drew answer + decision-set routes (#8568/#8766/#9335): the
// token-authorized submit/defer/request-info/drafts APIs and the
// decision-set pages' dispatch. Each handler returns null when its
// routes do not match. Split out of server.ts (#13052).
import { decodePathSegment } from "./path-segment.ts";
import { getNeedsDrew, listOpenNeedsDrew, needsDrewIncidentKey, recordAnswer } from "./needs-drew.ts";
import { decisionSetAging, decisionSetOpenState, deferWithSubmissions, getDecisionDrafts, getDecisionSet, dismissDecisionSets, isDeferLean, isDecisionSetView, listDecisionSets, raiseDecisionSet, refreshDecisionSet, recordResolutions, requestInfoOpenQuestions, sweepDecisionSets, upsertDecisionDrafts, validateDecisionDraftUpsert, validateDecisionSet, withGeneratedSetId, type DecisionSet, type DecisionSetView, type DecisionSubmission } from "./decisions.ts";
import { buildNeedsUserList, onQuestionAnswered, parseBlockedByCount, parsePriority, questionOptions, setQuestionBlockedByCount, setQuestionPriority, stageRowsFor, type NeedsUserListItem } from "./needs-user.ts";
import { appendDecisionEvents, readDecisionEvents, validateDecisionEventInput, type DecisionEventInput } from "./decision-events.ts";
import { getAlertsStore, requestAlertTick } from "./alerts.ts";
import { readJsonBody } from "./request-body.ts";
import { cfg, source } from "./server-wiring.ts";
import { json, html } from "./server-respond.ts";
import { renderAnswerPage, renderAnswerForbidden, decisionsPageUrl, logDecisionEvents, questionRefFor, eventTypeForKind, submissionEventPayload, parseDecisionSubmissions, renderDecisionsPage, renderDecisionsIndex, renderNeedsUserPage, renderDecisionsForbidden } from "./server-pages.ts";
import { alertsIntakeAuthorized } from "./server-alert-routes.ts";

export async function handleDecisionRoutes(req: Request, url: URL, path: string): Promise<Response | null> {
  // --- Needs-Drew answers (#8568) --------------------------------------
  const needsDrewAnswerMatch = /^\/api\/needs-drew\/([^/]+)\/answer$/.exec(path);
  if (req.method === "POST" && needsDrewAnswerMatch) {
    const ledgerId = decodePathSegment(needsDrewAnswerMatch[1]);
    if (ledgerId === null) return json({ error: "bad ledgerId" }, 400);
    let body: { token?: unknown; optionId?: unknown; freeText?: unknown };
    try {
      body = (await readJsonBody(req)) as typeof body;
    } catch {
      return json({ error: "invalid answer body" }, 400);
    }
    // No bearer: the per-question token is the authority.
    const item = await getNeedsDrew(cfg.needsDrewFile, ledgerId).catch(() => null);
    if (!item || typeof body.token !== "string" || body.token !== item.token) {
      return json({ error: "unknown question or bad token" }, 403);
    }
    const optionId = typeof body.optionId === "string" ? body.optionId : undefined;
    if (optionId !== undefined && !questionOptions(item).some((o) => o.id === optionId)) {
      return json({ error: "optionId is not one of the question options" }, 400);
    }
    const freeText = typeof body.freeText === "string" ? body.freeText : undefined;
    if (body.freeText !== undefined && freeText === undefined) {
      return json({ error: "freeText must be a string" }, 400);
    }
    if (optionId === undefined && !freeText?.trim()) {
      return json({ error: "optionId or freeText required" }, 400);
    }
    const updated = await recordAnswer(cfg.needsDrewFile, ledgerId, {
      ...(optionId !== undefined ? { optionId } : {}),
      ...(freeText !== undefined ? { freeText } : {}),
      via: "api",
    });
    if (!updated?.answer) return json({ error: "answer not recorded" }, 500);
    // #8605: the answer resolves the question, so close its alert
    // incident — the raiser posted it under this question's source with
    // key needs-drew:<ledgerId>, and until now nothing ever closed it.
    // Best-effort, like the stack-health close in the intake above: a
    // store failure is logged and never fails the recorded answer.
    try {
      const store = await getAlertsStore();
      if (store) {
        await store.closeOpenIncidentByKey(item.source, needsDrewIncidentKey(ledgerId));
        requestAlertTick();
      }
    } catch (err) {
      console.error(`needs-drew answer: incident close failed (${err instanceof Error ? err.message : "error"})`);
    }
    // #14084 S1: "Send back" returns the item to review; any other
    // answer resolves it. Keyed on the recorded (first) answer.
    try {
      const store = await getAlertsStore();
      if (store) await onQuestionAnswered(store.itemStages(), ledgerId, updated.answer.optionId);
    } catch (err) {
      console.error(`needs-drew answer: stage update failed (${err instanceof Error ? err.message : "error"})`);
    }
    return json({
      ledgerId,
      answered: true,
      answer: {
        ...(updated.answer.optionId !== undefined ? { optionId: updated.answer.optionId } : {}),
        ...(updated.answer.freeText !== undefined ? { freeText: updated.answer.freeText } : {}),
        at: updated.answer.at,
        via: updated.answer.via,
      },
    });
  }
  // --- Needs-user list (#14084 S1) ------------------------------------
  // Open ledger questions in answering order (priority, then issues
  // waiting on the answer, then age). Answer tokens never appear here.
  if (path === "/api/needs-user" || path === "/needs-user") {
    if (req.method !== "GET") return json({ error: "method not allowed" }, 405);
    const store = await getAlertsStore();
    const questions = await listOpenNeedsDrew(cfg.needsDrewFile).catch(() => []);
    let items: NeedsUserListItem[];
    let stagesAvailable = true;
    try {
      items = buildNeedsUserList(questions, store ? await stageRowsFor(store.itemStages(), questions) : new Map());
      if (!store) stagesAvailable = false;
    } catch (err) {
      console.error(`needs-user list: stage read failed (${err instanceof Error ? err.message : "error"})`);
      items = buildNeedsUserList(questions, new Map());
      stagesAvailable = false;
    }
    if (path === "/needs-user") return html(renderNeedsUserPage(items, stagesAvailable));
    return json({ items, stagesAvailable });
  }
  const needsUserSetMatch = /^\/api\/needs-user\/([^/]+)\/(priority|blocked-by)$/.exec(path);
  if (needsUserSetMatch) {
    if (req.method !== "POST") return json({ error: "method not allowed" }, 405);
    const ledgerId = decodePathSegment(needsUserSetMatch[1]);
    if (ledgerId === null) return json({ error: "bad ledgerId" }, 400);
    const field = needsUserSetMatch[2];
    // The blocked-by count is the fleet's to report (intake bearer);
    // priority is set from the list, like the push preferences.
    if (field === "blocked-by" && !(await alertsIntakeAuthorized(req))) {
      return json({ error: "unauthorized" }, 401);
    }
    let body: { priority?: unknown; count?: unknown };
    try {
      body = (await readJsonBody(req)) as typeof body;
    } catch {
      return json({ error: "invalid body" }, 400);
    }
    const item = await getNeedsDrew(cfg.needsDrewFile, ledgerId).catch(() => null);
    if (!item) return json({ error: "unknown question" }, 404);
    if (item.answer) return json({ error: "question already answered" }, 409);
    const store = await getAlertsStore();
    if (!store) return json({ error: "alerts store unavailable" }, 503);
    if (field === "priority") {
      const p = parsePriority(body.priority);
      if (!p.ok) return json({ error: p.error }, 400);
      const row = await setQuestionPriority(store.itemStages(), ledgerId, p.value);
      if (!row) return json({ error: "priority not recorded" }, 500);
      return json({ ledgerId, priority: row.priority === null ? null : Number(row.priority) });
    }
    const c = parseBlockedByCount(body.count);
    if (!c.ok) return json({ error: c.error }, 400);
    const row = await setQuestionBlockedByCount(store.itemStages(), ledgerId, c.value);
    if (!row) return json({ error: "count not recorded" }, 500);
    return json({ ledgerId, blockedByCount: Number(row.blocked_by_count) });
  }
  const answerPageMatch = /^\/answer\/([^/]+)$/.exec(path);
  if (req.method === "GET" && answerPageMatch) {
    const ledgerId = decodePathSegment(answerPageMatch[1]);
    const token = url.searchParams.get("t") ?? "";
    const item = ledgerId === null ? null : await getNeedsDrew(cfg.needsDrewFile, ledgerId).catch(() => null);
    // Unknown question or token mismatch: 403, and the question is
    // never rendered — the token is the only authority here.
    if (!item || !token || token !== item.token) return html(renderAnswerForbidden(), 403);
    return html(renderAnswerPage(item));
  }
  // --- Decision-set submit API + event log (#9335 v2) -----------------
  // POST /api/decision-sets (alias: /api/decision-sets/submit): the
  // authenticated submit API. Intake-bearer auth, the same token as
  // /api/alerts. Body: the bare v2 set object or { decisionSet: {...} }
  // (a sibling `source` string is honored). setId may be omitted — a
  // UUID is minted. The response returns the setId and the full token
  // page URL to the authenticated submitter ONLY; the URL is never
  // logged. Exactly one push per set: a fresh raise requests the alert
  // tick exactly like the /api/alerts decisionSet path; a re-submit of
  // an existing setId is idempotent and sends no second push.
  if (req.method === "POST" && (path === "/api/decision-sets" || path === "/api/decision-sets/submit")) {
    if (!(await alertsIntakeAuthorized(req))) return json({ error: "unauthorized" }, 401);
    let rawBody: unknown;
    try {
      rawBody = await readJsonBody(req);
    } catch {
      return json({ error: "invalid submit body" }, 400);
    }
    const prepared = withGeneratedSetId(rawBody);
    const ext =
      prepared && typeof prepared === "object" && !Array.isArray(prepared) ? (prepared as Record<string, unknown>) : {};
    const candidate = ext.decisionSet !== undefined ? ext.decisionSet : prepared;
    const v = validateDecisionSet(candidate);
    if (!v.ok) return json({ error: v.error }, 400);
    const source = typeof ext.source === "string" && ext.source ? ext.source : "decisions-ledger";
    // #9354 refresh mode: a sibling `refresh: true` opts a standing
    // (ledger-mirror) set into refreshDecisionSet — contents are
    // replaced, token/createdAt/source and resolutions-by-qid are
    // preserved. No push and no set.created event on a refresh: the
    // set is not new, and a standing set refreshed hourly must not
    // push hourly. Refresh of a not-yet-existing set is a normal
    // create (201, created: true, push + set.created as usual).
    const refresh = ext.refresh === true;
    const before = await getDecisionSet(cfg.decisionsFile, v.value.setId).catch(() => null);
    try {
      if (refresh && before) {
        const set = await refreshDecisionSet(cfg.decisionsFile, v.value, source);
        // The event log accepts a set-level "edit" event without a
        // questionRef (validateDecisionEventInput / the type enum
        // include "edit"), so a refresh is recorded that way.
        await logDecisionEvents([
          {
            setId: set.setId,
            type: "edit",
            payload: {
              via: "api/decision-sets",
              refresh: true,
              items: set.items.length,
              questions: set.items.reduce((n, i) => n + i.questions.length, 0),
            },
          },
        ]);
        const pageUrl = decisionsPageUrl(set);
        return json({ setId: set.setId, pageUrl, url: pageUrl, created: false, refreshed: true }, 200);
      }
      const set = await raiseDecisionSet(cfg.decisionsFile, v.value, source);
      if (!before) {
        await logDecisionEvents([
          {
            setId: set.setId,
            type: "set.created",
            payload: {
              via: "api/decision-sets",
              source,
              title: set.title,
              items: set.items.length,
              questions: set.items.reduce((n, i) => n + i.questions.length, 0),
            },
          },
        ]);
        requestAlertTick();
      }
      const pageUrl = decisionsPageUrl(set);
      return json({ setId: set.setId, pageUrl, url: pageUrl, created: !before }, before ? 200 : 201);
    } catch (err) {
      console.error(`decision-sets submit: raise failed (${err instanceof Error ? err.message : "error"})`);
      return json({ error: "decision set raise failed" }, 500);
    }
  }
  // Decision-set events: read (activity processor polling) + report
  // (the page's draft/edit events). Two prefixes name the same routes:
  // /api/decision-sets/<setId>/events (the documented contract) and
  // /api/decisions/<setId>/events (the page's own prefix).
  // Auth: the intake bearer OR the per-set token (as ?t=, as a bearer,
  // or — for POST — as body.token). Nothing about the set's events is
  // served without one of them.
  const decisionEventsMatch = /^\/api\/(?:decision-sets|decisions)\/([^/]+)\/events$/.exec(path);
  if ((req.method === "GET" || req.method === "POST") && decisionEventsMatch) {
    const setId = decodePathSegment(decisionEventsMatch[1]);
    if (setId === null) return json({ error: "bad setId" }, 400);
    const set = await getDecisionSet(cfg.decisionsFile, setId).catch(() => null);
    if (!set) return json({ error: "unknown set" }, 404);
    const bearer = (req.headers.get("authorization") ?? "").replace(/^Bearer\s+/i, "").trim();
    const tokenOk = (t: unknown): boolean => typeof t === "string" && t.length > 0 && t === set.token;
    if (req.method === "GET") {
      const qToken = url.searchParams.get("t") ?? "";
      if (!tokenOk(qToken) && !tokenOk(bearer) && !(await alertsIntakeAuthorized(req))) {
        return json({ error: "unknown set or bad token" }, 403);
      }
      const since = url.searchParams.get("since") ?? undefined;
      const limitRaw = Number(url.searchParams.get("limit") ?? "");
      const page = await readDecisionEvents(cfg.decisionsEventsFile, setId, {
        since,
        ...(Number.isFinite(limitRaw) && limitRaw > 0 ? { limit: limitRaw } : {}),
      });
      return json({ setId, events: page.events, nextCursor: page.nextCursor, count: page.count });
    }
    let evBody: { token?: unknown; events?: unknown };
    try {
      evBody = (await readJsonBody(req)) as typeof evBody;
    } catch {
      return json({ error: "invalid events body" }, 400);
    }
    if (!tokenOk(evBody.token) && !tokenOk(bearer) && !(await alertsIntakeAuthorized(req))) {
      return json({ error: "unknown set or bad token" }, 403);
    }
    if (!Array.isArray(evBody.events) || evBody.events.length < 1 || evBody.events.length > 32) {
      return json({ error: "events must be an array of 1-32 entries" }, 400);
    }
    const inputs: DecisionEventInput[] = [];
    for (const rawEv of evBody.events) {
      const ve = validateDecisionEventInput(rawEv);
      if (!ve.ok) return json({ error: ve.error }, 400);
      inputs.push({
        setId,
        type: ve.value.type,
        ...(ve.value.questionRef ? { questionRef: ve.value.questionRef } : {}),
        payload: ve.value.payload,
      });
    }
    const stored = await appendDecisionEvents(cfg.decisionsEventsFile, cfg.decisionsOutboxFile, inputs).catch(
      () => null,
    );
    if (!stored) return json({ error: "event append failed" }, 500);
    return json({ appended: stored.length, eventIds: stored.map((e) => e.eventId) });
  }
  return null;
}

export async function handleDecisionsIndex(url: URL): Promise<Response> {
  const sets = await listDecisionSets(cfg.decisionsFile).catch(() => [] as DecisionSet[]);
  const viewParam = url.searchParams.get("view");
    // Default is the rolling last-7-days window (#14064): the landing
    // view is the current workload, with all-time behind the toggle.
    const view: DecisionSetView = isDecisionSetView(viewParam) ? viewParam : "recent";
  return html(renderDecisionsIndex(sets, view, url.searchParams.get("open") === "1"));
}

export async function handleDecisionTailRoutes(req: Request, url: URL, path: string): Promise<Response | null> {
  const decisionsPageMatch = /^\/decisions\/([^/]+)$/.exec(path);
  if (req.method === "GET" && decisionsPageMatch) {
    const setId = decodePathSegment(decisionsPageMatch[1]);
    const token = url.searchParams.get("t") ?? "";
    const set = setId === null ? null : await getDecisionSet(cfg.decisionsFile, setId).catch(() => null);
    // Unknown set or token mismatch: 403, and the set is never rendered
    // — the per-set token is the only authority here.
    if (!set || !token || token !== set.token) return html(renderDecisionsForbidden(), 403);
    return html(renderDecisionsPage(set));
  }
  const decisionsSubmitMatch = /^\/api\/decisions\/([^/]+)\/submit$/.exec(path);
  if (req.method === "POST" && decisionsSubmitMatch) {
    const setId = decodePathSegment(decisionsSubmitMatch[1]);
    if (setId === null) return json({ error: "bad setId" }, 400);
    let body: { token?: unknown; submissions?: unknown };
    try {
      body = (await readJsonBody(req)) as typeof body;
    } catch {
      return json({ error: "invalid submit body" }, 400);
    }
    // No bearer: the per-set token is the authority.
    const set = await getDecisionSet(cfg.decisionsFile, setId).catch(() => null);
    if (!set || typeof body.token !== "string" || body.token !== set.token) {
      return json({ error: "unknown set or bad token" }, 403);
    }
    const parsedSubs = parseDecisionSubmissions(body.submissions);
    if (!parsedSubs.ok) return json({ error: parsedSubs.error }, 400);
    const result = await recordResolutions(cfg.decisionsFile, setId, parsedSubs.value);
    if (!result) return json({ error: "unknown set" }, 404);
    await logDecisionEvents(
      result.outcomes
        .filter((o) => o.applied)
        .map((o) => {
          const sub = parsedSubs.value.find((x) => x.qid === o.qid)!;
          return {
            setId,
            type: eventTypeForKind(sub.kind),
            questionRef: questionRefFor(set, sub.ledgerId, sub.qid),
            payload: submissionEventPayload(sub),
          } satisfies DecisionEventInput;
        }),
    );
    return json({ outcomes: result.outcomes, ...decisionSetOpenState(result.set) });
  }
  const decisionsDeferMatch = /^\/api\/decisions\/([^/]+)\/defer$/.exec(path);
  if (req.method === "POST" && decisionsDeferMatch) {
    const setId = decodePathSegment(decisionsDeferMatch[1]);
    if (setId === null) return json({ error: "bad setId" }, 400);
    let body: {
      token?: unknown;
      lean?: unknown;
      ledgerIds?: unknown;
      allRemaining?: unknown;
      submissions?: unknown;
      comment?: unknown;
    };
    try {
      body = (await readJsonBody(req)) as typeof body;
    } catch {
      return json({ error: "invalid defer body" }, 400);
    }
    // No bearer: the per-set token is the authority.
    const set = await getDecisionSet(cfg.decisionsFile, setId).catch(() => null);
    if (!set || typeof body.token !== "string" || body.token !== set.token) {
      return json({ error: "unknown set or bad token" }, 403);
    }
    if (!isDeferLean(body.lean)) {
      return json({ error: "lean must be none|skeptical|approve" }, 400);
    }
    const all = body.allRemaining === true;
    const ledgerIds = Array.isArray(body.ledgerIds)
      ? body.ledgerIds.filter((x): x is string => typeof x === "string")
      : [];
    if (!all && ledgerIds.length === 0) {
      return json({ error: "ledgerIds or allRemaining required" }, 400);
    }
    // Filled answers win over the bulk defer (#8766): submissions the
    // page gathered from filled-in fields are recorded FIRST, and only
    // the still-unanswered questions in scope are deferred.
    let submissions: DecisionSubmission[] = [];
    if (body.submissions !== undefined) {
      const parsedSubs = parseDecisionSubmissions(body.submissions);
      if (!parsedSubs.ok) return json({ error: parsedSubs.error }, 400);
      submissions = parsedSubs.value;
    }
    const result = await deferWithSubmissions(
      cfg.decisionsFile,
      setId,
      submissions,
      all ? { allRemaining: true as const } : { ledgerIds },
      body.lean,
      typeof body.comment === "string" ? body.comment : undefined,
    );
    if (!result) return json({ error: "unknown set" }, 404);
    await logDecisionEvents([
      ...result.outcomes
        .filter((o) => o.applied)
        .map((o) => {
          const sub = submissions.find((x) => x.qid === o.qid)!;
          return {
            setId,
            type: eventTypeForKind(sub.kind),
            questionRef: questionRefFor(set, sub.ledgerId, sub.qid),
            payload: submissionEventPayload(sub),
          } satisfies DecisionEventInput;
        }),
      ...(result.deferredCount > 0
        ? [
            {
              setId,
              type: "defer" as const,
              payload: {
                bulk: true,
                deferredCount: result.deferredCount,
                lean: body.lean as string,
                scope: all ? "allRemaining" : "checked",
              },
            } satisfies DecisionEventInput,
          ]
        : []),
    ]);
    return json({
      outcomes: result.outcomes,
      deferredCount: result.deferredCount,
      ...decisionSetOpenState(result.set),
    });
  }
  const decisionsReqInfoMatch = /^\/api\/decisions\/([^/]+)\/request-info$/.exec(path);
  if (req.method === "POST" && decisionsReqInfoMatch) {
    const setId = decodePathSegment(decisionsReqInfoMatch[1]);
    if (setId === null) return json({ error: "bad setId" }, 400);
    let body: { token?: unknown; note?: unknown; ledgerIds?: unknown; allRemaining?: unknown };
    try {
      body = (await readJsonBody(req)) as typeof body;
    } catch {
      return json({ error: "invalid request-info body" }, 400);
    }
    // No bearer: the per-set token is the authority.
    const set = await getDecisionSet(cfg.decisionsFile, setId).catch(() => null);
    if (!set || typeof body.token !== "string" || body.token !== set.token) {
      return json({ error: "unknown set or bad token" }, 403);
    }
    const all = body.allRemaining === true;
    const ledgerIds = Array.isArray(body.ledgerIds)
      ? body.ledgerIds.filter((x): x is string => typeof x === "string")
      : [];
    if (!all && ledgerIds.length === 0) {
      return json({ error: "ledgerIds or allRemaining required" }, 400);
    }
    const result = await requestInfoOpenQuestions(
      cfg.decisionsFile,
      setId,
      all ? { allRemaining: true as const } : { ledgerIds },
      typeof body.note === "string" ? body.note : undefined,
    );
    if (!result) return json({ error: "unknown set" }, 404);
    if (result.requestedCount > 0) {
      await logDecisionEvents([
        {
          setId,
          type: "request-info",
          payload: {
            bulk: true,
            requestedCount: result.requestedCount,
            scope: all ? "allRemaining" : "checked",
            ...(typeof body.note === "string" && body.note ? { note: body.note.slice(0, 2000) } : {}),
          },
        },
      ]);
    }
    return json({ requestedCount: result.requestedCount, ...decisionSetOpenState(result.set) });
  }

  const decisionsDraftsMatch = /^\/api\/decisions\/([^/]+)\/drafts$/.exec(path);
  if ((req.method === "GET" || req.method === "POST") && decisionsDraftsMatch) {
    const setId = decodePathSegment(decisionsDraftsMatch[1]);
    if (setId === null) return json({ error: "bad setId" }, 400);
    const set = await getDecisionSet(cfg.decisionsFile, setId).catch(() => null);
    if (req.method === "GET") {
      const token = url.searchParams.get("t") ?? "";
      // Unknown set or token mismatch: 403, nothing about the set's
      // drafts is shown without the correct link.
      if (!set || !token || token !== set.token) {
        return json({ error: "unknown set or bad token" }, 403);
      }
      const drafts = await getDecisionDrafts(cfg.decisionsFile, setId).catch(() => null);
      return json({
        drafts: drafts ?? { questions: {}, bulk: [], bulkLean: "", bulkNote: "", updatedAt: "" },
      });
    }
    let body: { token?: unknown; drafts?: unknown };
    try {
      body = (await readJsonBody(req)) as typeof body;
    } catch {
      return json({ error: "invalid drafts body" }, 400);
    }
    // No bearer: the per-set token is the authority.
    if (!set || typeof body.token !== "string" || body.token !== set.token) {
      return json({ error: "unknown set or bad token" }, 403);
    }
    const parsedDrafts = validateDecisionDraftUpsert(body.drafts);
    if (!parsedDrafts.ok) return json({ error: parsedDrafts.error }, 400);
    const result = await upsertDecisionDrafts(cfg.decisionsFile, setId, parsedDrafts.value);
    if (!result) return json({ error: "unknown set" }, 404);
    await logDecisionEvents(
      result.applied.map((qid) => {
        const entry = result.drafts.questions[qid];
        const ledgerId = qid.slice(0, qid.lastIndexOf("#")) || qid;
        return {
          setId,
          type: "draft" as const,
          questionRef: questionRefFor(set, ledgerId, qid),
          payload: {
            ...(entry?.opt ? { opt: entry.opt } : {}),
            ...(entry?.opts ? { opts: entry.opts } : {}),
            ...(entry?.ft ? { freeText: entry.ft.slice(0, 2000) } : {}),
            ...(entry?.lean ? { lean: entry.lean } : {}),
            ...(entry?.note ? { note: entry.note.slice(0, 2000) } : {}),
          },
        } satisfies DecisionEventInput;
      }),
    );
    return json({ drafts: result.drafts, applied: result.applied, skipped: result.skipped });
  }
  return null;
}
