// Alert throat (#3676) routes: push subscriptions, test sends, the
// bearer-gated incident intake, and the alerts/notifications pages.
// Split out of server.ts (#13052).
import { healthHookAuditEvent } from "./health-hook-audit-event.ts";
import { selectNeedsUserExtension } from "./alert-extension-compat.ts";
import { raiseNeedsDrew, validateNeedsDrew } from "./needs-drew.ts";
import { getDecisionSet, raiseDecisionSet, validateDecisionSet } from "./decisions.ts";
import { setStackHealth, validateStackHealth } from "./stack-health.ts";
import { validateHealthHook, type HealthHookRecord } from "./health-hook.ts";
import { timingSafeEqual } from "node:crypto";
import {
  getAlertsStore,
  requestAlertTick,
  sendClassTestPush,
  sendTestPush,
  toPublicSubscription,
  validateNewIncident,
  validateNewSubscription,
} from "./alerts.ts";
import { requestNotifyRelaySubscriptionSync } from "./notify-relay.ts";
import { onQuestionRaised } from "./needs-user.ts";
import { VAPID_PUBLIC_KEY } from "./push.ts";
import { resolveBwsSecret } from "./secrets.ts";
import { renderAlerts, renderNotificationPreferences, renderNotifications } from "./render.ts";
import {
  isNotificationTestClass,
  NOTIFICATION_CLASS_DEFAULTS,
  NOTIFICATION_CLASSES,
  rollupNotificationSends,
} from "./notifications.ts";
import { readJsonBody } from "./request-body.ts";
import type { Source } from "./source.ts";
import { cfg, eventsStore, source } from "./server-wiring.ts";
import { json, html } from "./server-respond.ts";
import { needsDrewAnswerUrl, decisionsPageUrl, logDecisionEvents } from "./server-pages.ts";

// --- Alert throat (#3676) -------------------------------------------------

/** Bearer check for POST /api/alerts: the intake token is resolved
 * by name from Bitwarden (pm-aggregator.alerts-intake-token) and
 * compared in constant time; its value is never logged. */
export async function alertsIntakeAuthorized(req: Request): Promise<boolean> {
  const header = req.headers.get("authorization") ?? "";
  const presented = header.startsWith("Bearer ") ? header.slice("Bearer ".length).trim() : "";
  if (!presented) return false;
  const expected = await resolveBwsSecret("pm-aggregator.alerts-intake-token");
  if (!expected) return false;
  const a = Buffer.from(presented);
  const b = Buffer.from(expected);
  return a.length === b.length && timingSafeEqual(a, b);
}

export const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export async function handleAlertIntakeRoutes(req: Request, path: string): Promise<Response | null> {
  // --- Alert throat (#3676): subscription API -------------------------
  // #13979: method gating precedes the store check so a removed GET
  // stays a 405 even when the alerts store is unavailable (fixtures /
  // store outage would otherwise mask it as a 503).
  if (path === "/api/push/subscriptions") {
    if (req.method !== "POST" && req.method !== "DELETE") {
      return json({ error: "method not allowed" }, 405);
    }
    const store = await getAlertsStore();
    if (!store) return json({ error: "alerts store unavailable" }, 503);
    if (req.method === "POST") {
      try {
        const parsed = validateNewSubscription(await readJsonBody(req));
        if (!parsed.ok) return json({ error: parsed.error }, 400);
        const row = await store.upsertSubscription(parsed.value);
        requestNotifyRelaySubscriptionSync();
        return json({ subscription: toPublicSubscription(row) }, 201);
      } catch (err) {
        return json({ error: `invalid subscription: ${err instanceof Error ? err.message : err}` }, 400);
      }
    }
    if (req.method === "DELETE") {
      try {
        const body = (await readJsonBody(req)) as { endpoint?: unknown };
        if (typeof body.endpoint !== "string") return json({ error: "endpoint required" }, 400);
        const deleted = await store.deleteSubscription(body.endpoint);
        if (deleted) requestNotifyRelaySubscriptionSync();
        return deleted ? json({ deleted: true }) : json({ error: "no such subscription" }, 404);
      } catch (err) {
        return json({ error: `invalid delete: ${err instanceof Error ? err.message : err}` }, 400);
      }
    }
    return json({ error: "method not allowed" }, 405);
  }

  if (path === "/api/push/test") {
    if (req.method !== "POST") return json({ error: "method not allowed" }, 405);
    const store = await getAlertsStore();
    if (!store) return json({ error: "alerts store unavailable" }, 503);
    const summary = await sendTestPush(store, cfg.publicOrigin);
    return json(summary);
  }

  // Per-class test sends (#3318): one test per preference class,
  // honouring the stored class toggles, landing per design §3.
  if (path === "/api/push/test-class") {
    if (req.method !== "POST") return json({ error: "method not allowed" }, 405);
    let cls: unknown;
    try {
      cls = ((await readJsonBody(req)) as { class?: unknown })?.class;
    } catch {
      return json({ error: "invalid test-class body" }, 400);
    }
    if (!isNotificationTestClass(cls)) return json({ error: "unknown notification class" }, 400);
    const store = await getAlertsStore();
    if (!store) return json({ error: "alerts store unavailable" }, 503);
    const summary = await sendClassTestPush(store, cfg.publicOrigin, cls);
    return json(summary);
  }

  // --- Alert throat (#3676): incident intake + ack/close --------------
  if (req.method === "POST" && path === "/api/alerts") {
    if (!(await alertsIntakeAuthorized(req))) return json({ error: "unauthorized" }, 401);
    const store = await getAlertsStore();
    if (!store) return json({ error: "alerts store unavailable" }, 503);
    try {
      const rawBody: unknown = await readJsonBody(req);
      const parsed = validateNewIncident(rawBody);
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      const ext =
        rawBody && typeof rawBody === "object" && !Array.isArray(rawBody) ? (rawBody as Record<string, unknown>) : {};
      const needsUserExtension = selectNeedsUserExtension(ext);
      if (needsUserExtension.kind === "ambiguous") {
        return json({ error: "needsUser and needsDrew are aliases; supply only one" }, 400);
      }
      const hasNeedsUserQuestion = needsUserExtension.kind === "selected";
      const hasStackHealth = ext.stackHealth !== undefined;
      const hasDecisionSet = ext.decisionSet !== undefined;
      const hasHealthHook = ext.healthHook !== undefined;
      // #12602 (#12599 decision b): the healthHook extension carries the
      // full C3 incident record to the alerts ledger. It is gated and
      // validated BEFORE the incident insert, unlike the extensions
      // below, because a valid record rides the insert onto the opened
      // event's detail; a malformed one is a 400 naming the field,
      // before anything is stored.
      let healthHook: HealthHookRecord | undefined;
      if (hasHealthHook) {
        if (parsed.value.source !== "health-hook") {
          return json({ error: 'healthHook extension is only accepted for source "health-hook"' }, 400);
        }
        const hv = validateHealthHook(ext.healthHook);
        if (!hv.ok) return json({ error: hv.error }, 400);
        healthHook = hv.value;
      }
      const { incident, created } = await store.postIncident(parsed.value, healthHook ? { healthHook } : undefined);
      const status = created ? 201 : 200;
      // #8568/#8766 producer extensions: exactly one needsUser or
      // legacy needsDrew / stackHealth / decisionSet may ride an incident
      // post. #12602 adds healthHook, which may co-ride with stackHealth
      // (health-hook source only, gated above) and with nothing else. An
      // extension validation failure is a 400; a store failure is logged
      // and never fails the incident post.
      if ([hasNeedsUserQuestion, hasStackHealth, hasDecisionSet].filter(Boolean).length > 1) {
        return json(
          { error: "needsUser/needsDrew, stackHealth and decisionSet extensions are mutually exclusive" },
          400,
        );
      }
      if (hasHealthHook && (hasNeedsUserQuestion || hasDecisionSet)) {
        return json({ error: "healthHook extension may only co-ride with stackHealth" }, 400);
      }
      if (needsUserExtension.kind === "selected") {
        const v = validateNeedsDrew(needsUserExtension.value);
        if (!v.ok) return json({ error: v.error }, 400);
        try {
          const item = await raiseNeedsDrew(cfg.needsDrewFile, v.value, parsed.value.source, parsed.value.key);
          // #14084 S1: the question enters the staged pipeline at
          // needs-user. Best-effort: a store failure never fails the raise.
          try {
            const store = await getAlertsStore();
            if (store) await onQuestionRaised(store.itemStages(), item);
          } catch (err) {
            console.error(`alerts intake: needs-user stage failed (${err instanceof Error ? err.message : "error"})`);
          }
          requestAlertTick();
          return json(
            {
              incident,
              created,
              [needsUserExtension.alias]: { ledgerId: item.ledgerId, answerUrl: needsDrewAnswerUrl(item) },
            },
            status,
          );
        } catch (err) {
          console.error(`alerts intake: needs-user raise failed (${err instanceof Error ? err.message : "error"})`);
          return json({ incident, created }, status);
        }
      }
      if (hasStackHealth) {
        const v = validateStackHealth(ext.stackHealth);
        if (!v.ok) return json({ error: v.error }, 400);
        try {
          await setStackHealth(cfg.stackHealthFile, v.value);
          const auditEvent = healthHookAuditEvent({
            title: parsed.value.title,
            source: parsed.value.source,
            stackHealth: v.value,
          });
          if (auditEvent) {
            try {
              await eventsStore.record(auditEvent);
            } catch (err) {
              console.error(
                `alerts intake: health-hook audit append failed (${err instanceof Error ? err.message : "error"})`,
              );
            }
          }
          if (v.value.state === "up") {
            // #12602: a healthHook recovery record rides the closed
            // event's detail of the incident it closes.
            await store.closeOpenIncidentByKey(
              parsed.value.source,
              parsed.value.key,
              healthHook?.state === "up" ? { healthHook } : undefined,
            );
          }
          requestAlertTick();
        } catch (err) {
          console.error(`alerts intake: stackHealth update failed (${err instanceof Error ? err.message : "error"})`);
        }
        return json({ incident, created }, status);
      }
      if (hasDecisionSet) {
        const v = validateDecisionSet(ext.decisionSet);
        if (!v.ok) return json({ error: v.error }, 400);
        try {
          const beforeSet = await getDecisionSet(cfg.decisionsFile, v.value.setId).catch(() => null);
          const set = await raiseDecisionSet(cfg.decisionsFile, v.value, parsed.value.source);
          if (!beforeSet) {
            await logDecisionEvents([
              {
                setId: set.setId,
                type: "set.created",
                payload: {
                  via: "api/alerts",
                  source: parsed.value.source,
                  title: set.title,
                  items: set.items.length,
                  questions: set.items.reduce((n, i) => n + i.questions.length, 0),
                },
              },
            ]);
          }
          requestAlertTick();
          return json({ incident, created, decisionSet: { setId: set.setId, pageUrl: decisionsPageUrl(set) } }, status);
        } catch (err) {
          console.error(`alerts intake: decisionSet raise failed (${err instanceof Error ? err.message : "error"})`);
          return json({ incident, created }, status);
        }
      }
      return json({ incident, created }, status);
    } catch (err) {
      return json({ error: `invalid incident: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  return null;
}

export async function handleAlertAction(alertActionMatch: RegExpExecArray): Promise<Response> {
  const id = alertActionMatch[1];
  if (!UUID_RE.test(id)) return json({ error: "incident id must be a uuid" }, 400);
  const store = await getAlertsStore();
  if (!store) return json({ error: "alerts store unavailable" }, 503);
  const result = alertActionMatch[2] === "ack" ? await store.ackIncident(id) : await store.closeIncident(id);
  if (!result.incident) return json({ error: "no such incident", id }, 404);
  return json({ incident: result.incident, transitioned: result.transitioned });
}

export async function handleAlertRead(alertReadMatch: RegExpExecArray): Promise<Response> {
  const id = alertReadMatch[1];
  if (!UUID_RE.test(id)) return json({ error: "incident id must be a uuid" }, 400);
  const store = await getAlertsStore();
  if (!store) return json({ error: "alerts store unavailable" }, 503);
  const incident = await store.getIncident(id);
  if (!incident) return json({ error: "no such incident", id }, 404);
  const events = await store.listIncidentEvents(id);
  return json({ incident, events });
}

export async function handleAlertsList(path: string, prov: () => ReturnType<Source["provenance"]>): Promise<Response> {
  const store = await getAlertsStore();
  if (!store) {
    return path === "/alerts"
      ? html(
          renderAlerts(
            { available: false, incidents: [], events: [], subscriptions: [], ackWindowMin: cfg.alerts.ackWindowMin },
            prov(),
            VAPID_PUBLIC_KEY,
          ),
        )
      : json({ error: "alerts store unavailable" }, 503);
  }
  try {
    const [incidents, events, subscriptions] = await Promise.all([
      store.listIncidents(),
      store.recentEvents(),
      store.listSubscriptions(),
    ]);
    if (path === "/api/alerts.json") return json({ incidents, events });
    return html(
      renderAlerts(
        { available: true, incidents, events, subscriptions, ackWindowMin: cfg.alerts.ackWindowMin },
        prov(),
        VAPID_PUBLIC_KEY,
      ),
    );
  } catch {
    return path === "/alerts"
      ? html(
          renderAlerts(
            { available: false, incidents: [], events: [], subscriptions: [], ackWindowMin: cfg.alerts.ackWindowMin },
            prov(),
            VAPID_PUBLIC_KEY,
          ),
        )
      : json({ error: "alerts store unavailable" }, 503);
  }
}

export async function handleAlertPreferences(
  path: string,
  prov: () => ReturnType<Source["provenance"]>,
): Promise<Response> {
  // #3318: subscription preferences — per-class toggles, quiet
  // hours, this-browser state, per-class test sends. Writes go
  // through the existing /api/push/subscriptions upsert; this
  // route only reads the stored subscriptions for the form.
  const store = await getAlertsStore();
  if (!store) {
    return path === "/alerts/preferences"
      ? html(renderNotificationPreferences({ available: false, subscriptions: [] }, prov(), VAPID_PUBLIC_KEY))
      : json({ error: "alerts store unavailable" }, 503);
  }
  try {
    const subscriptions = await store.listSubscriptions();
    if (path === "/api/alerts/preferences.json")
      return json({
        available: true,
        subscriptions: subscriptions.map(toPublicSubscription),
        classes: NOTIFICATION_CLASSES,
        defaults: NOTIFICATION_CLASS_DEFAULTS,
      });
    return html(renderNotificationPreferences({ available: true, subscriptions }, prov(), VAPID_PUBLIC_KEY));
  } catch {
    return path === "/alerts/preferences"
      ? html(renderNotificationPreferences({ available: false, subscriptions: [] }, prov(), VAPID_PUBLIC_KEY))
      : json({ error: "alerts store unavailable" }, 503);
  }
}

export async function handleNotificationsPage(
  path: string,
  prov: () => ReturnType<Source["provenance"]>,
): Promise<Response> {
  // #3319: the weekly sent-vs-acted roll-up over the send ledger.
  // Same reachability as every dashboard page (the tailnet origin);
  // the only alerts write surface stays the bearer-gated intake.
  const store = await getAlertsStore();
  if (!store) {
    return path === "/notifications"
      ? html(renderNotifications({ available: false, rollup: null }, prov()))
      : json({ error: "alerts store unavailable" }, 503);
  }
  try {
    const sends = await store.listSends();
    const rollup = rollupNotificationSends(sends, new Date());
    if (path === "/api/notifications.json") return json(rollup);
    return html(renderNotifications({ available: true, rollup }, prov()));
  } catch {
    return path === "/notifications"
      ? html(renderNotifications({ available: false, rollup: null }, prov()))
      : json({ error: "alerts store unavailable" }, 503);
  }
}
