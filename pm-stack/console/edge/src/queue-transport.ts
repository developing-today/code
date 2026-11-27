// The console's own watch on the queue transport's health (#13503
// slice 2, #16242). Slice 1 made the console the notifier: it read the
// queue-transport-alert Worker's /status and told the owner itself.
// This slice retires that Worker: the console's scheduled tick now
// does the probe itself, from off the broker's host, and keeps the
// incident semantics the Worker had — find-or-file one GitHub
// incident, comment on recovery, never close (#10136), and enqueue
// one triage task, anchored to the incident, once the broker answers
// again.
//
// The probe/state machine below is a port of the retired Worker's
// pkgs/queue-transport-alert/worker/src/{logic,run,github,queue,
// status}.js, so the two implementations agree on one contract while
// both exist: threshold consecutive failed probes alert once per
// outage window; the first passing probe after an alerted window
// recovers once; a failing window that ends below the threshold ends
// quietly. The Worker's Telegram and Web Push channels are not
// ported: the console's own notifications are the one place that
// decides what reaches the owner's phone (#13503), and they already
// speak from the verdict kept here (see the alarm in edge.ts).
//
// Oracle: the retired Worker's publicStatus shape
// (pkgs/queue-transport-alert/worker/src/status.js) and its runPass
// semantics, exercised in the console tests with stand-in fetchers
// and clients.
import { timed } from "../../src/timed.ts";

/** The transport verdict the console keeps, as its probe stated it. */
export interface QueueTransport {
  /** up | degraded (failing below threshold) | down (alerted) | stale | unknown */
  status: string;
  consecutiveFailures: number;
  failingSince: string | null;
  lastFailure: string | null;
  lastCheck: string | null;
  lastOk: string | null;
  incident: { number: number; url: string } | null;
  checkedAt: number;
}

/** The storage key the object keeps the last verdict under. */
export const QUEUE_TRANSPORT_KEY = "queue-transport";
/** The storage key for the once-per-outage notification flag. */
export const QUEUE_TRANSPORT_TOLD_KEY = "queue-transport-told";
/** The storage key for the probe's own state (the retired Worker's
 * Durable Object state, kept here now). */
export const QUEUE_TRANSPORT_PROBE_KEY = "queue-transport-probe";

/** The repository incidents are filed in, unless a run says another. */
export const QUEUE_TRANSPORT_REPO = "developing-today/code";
/** The broker's public route: a Cloudflare tunnel to the broker's
 * Tailscale listener, reached through Cloudflare Access. */
export const QUEUE_BROKER_URL = "https://queue-broker.security.cab";
/** Failed probes in a row before an outage window alerts. */
export const QUEUE_TRANSPORT_THRESHOLD = 2;
/** No probe for this long and the verdict reads stale: more than two
 * missed ten-minute ticks (the retired Worker's STALE_AFTER_MS). */
export const QUEUE_TRANSPORT_STALE_MS = 25 * 60 * 1000;

export const INCIDENT_TITLE_PREFIX = "Queue transport incident:";
export const INCIDENT_LABELS = ["kind/bug", "priority/high", "area/infra", "source/fleet-monitor"];

/** Read a stored verdict back, or null when what is stored is not one. */
export function parseQueueTransport(raw: unknown): QueueTransport | null {
  if (typeof raw !== "object" || raw === null) return null;
  const record = raw as Record<string, unknown>;
  if (typeof record.status !== "string") return null;
  if (typeof record.consecutiveFailures !== "number") return null;
  if (typeof record.checkedAt !== "number") return null;
  const strOrNull = (value: unknown): string | null =>
    value === null || value === undefined ? null : typeof value === "string" ? value : null;
  let incident: QueueTransport["incident"] = null;
  if (record.incident !== null && record.incident !== undefined) {
    if (typeof record.incident !== "object") return null;
    const issue = record.incident as Record<string, unknown>;
    if (typeof issue.number !== "number" || typeof issue.url !== "string") return null;
    incident = { number: issue.number, url: issue.url };
  }
  return {
    status: record.status,
    consecutiveFailures: record.consecutiveFailures,
    failingSince: strOrNull(record.failingSince),
    lastFailure: strOrNull(record.lastFailure),
    lastCheck: strOrNull(record.lastCheck),
    lastOk: strOrNull(record.lastOk),
    incident,
    checkedAt: record.checkedAt,
  };
}

/** Whether this verdict is worth waking the owner for. */
export function transportNeedsAttention(transport: QueueTransport | null): boolean {
  if (!transport) return false;
  return transport.status === "down" || transport.status === "degraded" || transport.status === "stale";
}

/**
 * Whether the owner should be told now: attention-worthy and not told
 * for this outage yet (told names the failing-since it was told for),
 * or recovered after having been told. Pure, so the tests hold it
 * without devices.
 */
export function transportNotifyDecision(
  transport: QueueTransport | null,
  told: string | null,
): "notify" | "recovered" | "quiet" {
  if (!transport || !transportNeedsAttention(transport)) {
    // A missing or unknown verdict is not evidence the broker recovered.
    return transport?.status === "up" && told ? "recovered" : "quiet";
  }
  const marker = transport.failingSince ?? transport.status;
  return told === marker ? "quiet" : "notify";
}

/** The notification said when the transport needs attention. */
export function transportMessage(transport: QueueTransport): { title: string; body: string; tag: string } {
  const incident = transport.incident ? ` Incident #${transport.incident.number}.` : "";
  const failing = transport.consecutiveFailures
    ? ` (${transport.consecutiveFailures} failed checks${transport.failingSince ? ` since ${transport.failingSince}` : ""})`
    : "";
  const last = transport.lastFailure ? ` Last failure: ${transport.lastFailure}` : "";
  if (transport.status === "stale") {
    return {
      title: "Queue broker status is stale",
      body: `The queue transport alert has not checked in.${failing}${incident}`,
      tag: "queue-transport",
    };
  }
  if (transport.status === "degraded") {
    return {
      title: "Queue broker is struggling",
      body: `The broker is failing checks but below the alert threshold.${failing}${last}${incident}`,
      tag: "queue-transport",
    };
  }
  return {
    title: "Queue broker is down",
    body: `The repo-queue broker is unreachable.${failing}${last}${incident}`,
    tag: "queue-transport",
  };
}

/** The marker stored once the owner has been told of this outage. */
export function transportToldMarker(transport: QueueTransport): string {
  return transport.failingSince ?? transport.status;
}

// ---------------------------------------------------------------------------
// The probe itself: the retired Worker's logic.js / run.js, ported.
// ---------------------------------------------------------------------------

/** One recovery triage task still to enqueue. */
export interface TransportPendingEnqueue {
  type: "enqueue";
  incident: number;
  start: string | null;
  end: string;
}

/** The probe's kept state: the retired Worker's Durable Object state
 * (its `state` key), field for field, minus the push subscriptions
 * and last push, which were the Worker's own notification plumbing. */
export interface TransportProbeState {
  consecutive_failures: number;
  window_start: string | null;
  last_failure_detail: string | null;
  alerted: boolean;
  incident: number | null;
  incident_filed_by_probe: boolean;
  alert_notified: boolean;
  recovery_notified: boolean;
  last_ok: string | null;
  last_probe: string | null;
  last_outage: { start: string | null; end: string; incident: number | null } | null;
  pending: TransportPendingEnqueue[];
  last_errors?: string[];
}

export function emptyProbeState(): TransportProbeState {
  return {
    consecutive_failures: 0,
    window_start: null,
    last_failure_detail: null,
    alerted: false,
    incident: null,
    incident_filed_by_probe: false,
    alert_notified: false,
    recovery_notified: false,
    last_ok: null,
    last_probe: null,
    last_outage: null,
    pending: [],
  };
}

/** Read kept probe state back, or null when what is kept is not one:
 * an object that outlives a change of shape starts the watch afresh
 * rather than handing the pass a shape it cannot advance. */
export function parseProbeState(raw: unknown): TransportProbeState | null {
  if (typeof raw !== "object" || raw === null) return null;
  const record = raw as Record<string, unknown>;
  if (typeof record.consecutive_failures !== "number") return null;
  if (typeof record.alerted !== "boolean") return null;
  const strOrNull = (value: unknown): string | null => (typeof value === "string" ? value : null);
  const pending: TransportPendingEnqueue[] = [];
  if (Array.isArray(record.pending)) {
    for (const item of record.pending) {
      if (typeof item !== "object" || item === null) continue;
      const entry = item as Record<string, unknown>;
      if (entry.type !== "enqueue" || typeof entry.incident !== "number" || typeof entry.end !== "string") continue;
      pending.push({ type: "enqueue", incident: entry.incident, start: strOrNull(entry.start), end: entry.end });
    }
  }
  let lastOutage: TransportProbeState["last_outage"] = null;
  if (typeof record.last_outage === "object" && record.last_outage !== null) {
    const outage = record.last_outage as Record<string, unknown>;
    if (typeof outage.end === "string") {
      lastOutage = {
        start: strOrNull(outage.start),
        end: outage.end,
        incident: typeof outage.incident === "number" ? outage.incident : null,
      };
    }
  }
  return {
    consecutive_failures: record.consecutive_failures,
    window_start: strOrNull(record.window_start),
    last_failure_detail: strOrNull(record.last_failure_detail),
    alerted: record.alerted,
    incident: typeof record.incident === "number" ? record.incident : null,
    incident_filed_by_probe: record.incident_filed_by_probe === true,
    alert_notified: record.alert_notified === true,
    recovery_notified: record.recovery_notified === true,
    last_ok: strOrNull(record.last_ok),
    last_probe: strOrNull(record.last_probe),
    last_outage: lastOutage,
    pending,
    ...(Array.isArray(record.last_errors)
      ? { last_errors: record.last_errors.filter((e): e is string => typeof e === "string") }
      : {}),
  };
}

export function iso(ms: number): string {
  return new Date(ms).toISOString().replace(/\.\d{3}Z$/, "Z");
}

export function span(startMs: number, endMs: number): string {
  const minutes = Math.max(0, Math.floor((endMs - startMs) / 60000));
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h ? `${h}h${String(m).padStart(2, "0")}m` : `${m}m`;
}

/** A fresh window that keeps what outlives one (last outage, pending). */
function freshWindow(
  prev: TransportProbeState,
  nowMs: number,
  lastOutage?: TransportProbeState["last_outage"],
): TransportProbeState {
  return {
    ...emptyProbeState(),
    last_ok: iso(nowMs),
    last_probe: iso(nowMs),
    last_outage: lastOutage === undefined ? (prev.last_outage ?? null) : lastOutage,
    pending: Array.isArray(prev.pending) ? prev.pending : [],
  };
}

/** Advance the state by one probe result. Returns {state, event}. */
export function stepProbe(
  prev: TransportProbeState,
  ok: boolean,
  detail: string,
  nowMs: number,
  threshold: number,
): { state: TransportProbeState; event: "alert" | "recovered" | "ok" | "down" | "failing" } {
  const state: TransportProbeState = { ...emptyProbeState(), ...prev, last_probe: iso(nowMs) };
  if (ok) {
    if (state.alerted) {
      return { state: { ...state, consecutive_failures: 0, last_ok: iso(nowMs) }, event: "recovered" };
    }
    return { state: freshWindow(state, nowMs), event: "ok" };
  }
  state.consecutive_failures = Number(state.consecutive_failures || 0) + 1;
  state.last_failure_detail = detail;
  if (!state.window_start) state.window_start = iso(nowMs);
  if (state.alerted) return { state, event: "down" };
  if (state.consecutive_failures >= threshold) {
    state.alerted = true;
    return { state, event: "alert" };
  }
  return { state, event: "failing" };
}

/** The state after a recovery has been announced: a fresh window that
 * records the outage just ended. */
export function finishProbeRecovery(state: TransportProbeState, nowMs: number): TransportProbeState {
  return freshWindow(state, nowMs, {
    start: state.window_start,
    end: iso(nowMs),
    incident: state.incident ?? null,
  });
}

export function incidentTitle(state: TransportProbeState): string {
  return `${INCIDENT_TITLE_PREFIX} repo-queue broker unreachable since ${state.window_start}`;
}

export function alertMessage(state: TransportProbeState, nowMs: number): string {
  const start = Date.parse(state.window_start ?? "") || nowMs;
  return (
    `The repo-queue broker has been unreachable for ${span(start, nowMs)} ` +
    `(${state.consecutive_failures} failed probes since ${state.window_start}), ` +
    `so worker lanes are starving at their health gate.`
  );
}

export function recoveryMessage(state: TransportProbeState, nowMs: number): string {
  const start = Date.parse(state.window_start ?? "") || nowMs;
  return (
    `The repo-queue broker is reachable again after about ${span(start, nowMs)} ` +
    `down (${state.window_start} to ${iso(nowMs)}).`
  );
}

export function incidentBody(state: TransportProbeState, nowMs: number, probeDesc: string): string {
  return [
    "Filed automatically by the PM console's queue transport watch " +
      "(pm-stack/console/edge, #13503 slice 2), which probes the " +
      "broker from Cloudflare, off the broker host. It replaces the " +
      "retired queue-transport-alert Worker (code#6369).",
    "",
    `- **Probe:** ${probeDesc}`,
    `- **Failing since:** ${state.window_start}`,
    `- **Consecutive failed probes:** ${state.consecutive_failures}`,
    `- **Last failure:** ${state.last_failure_detail || "n/a"}`,
    "",
    alertMessage(state, nowMs),
    "",
    "No queue task is filed now, because the queue is the thing that is " +
      "down. When the broker answers again the console enqueues one triage " +
      "task anchored to this issue to find the cause.",
    "",
    "The console comments here once when the transport recovers. Close this " +
      "issue by hand once the outage is understood; automation never closes " +
      "issues (#10136).",
  ].join("\n");
}

function clip(text: unknown): string {
  const t = String(text || "").replace(/\s+/g, " ").trim();
  return t.length <= 200 ? t : `${t.slice(0, 197)}...`;
}

/**
 * Classify one /healthz response. Anything but a 200 whose JSON says
 * ok:true is a failed probe; the detail names what was seen so an
 * Access misconfiguration (302/401/403) reads differently from the
 * tunnel being down (502/530) in the incident.
 */
export function classifyHealthz(status: number, bodyText: string): { ok: boolean; detail: string } {
  if (status === 200) {
    try {
      const body = JSON.parse(bodyText) as { ok?: unknown } | null;
      if (body && body.ok === true) return { ok: true, detail: "" };
      return { ok: false, detail: `HTTP 200 but body was not ok:true: ${clip(bodyText)}` };
    } catch {
      return { ok: false, detail: `HTTP 200 with a non-JSON body: ${clip(bodyText)}` };
    }
  }
  let hint = "";
  if ([301, 302, 303, 307, 401, 403].includes(status)) {
    hint = " (Cloudflare Access refused the probe's service token, or it is unset)";
  } else if ([502, 503, 504, 530].includes(status)) {
    hint = " (the tunnel could not reach the broker)";
  }
  return { ok: false, detail: `HTTP ${status}${hint}: ${clip(bodyText)}` };
}

const PROBE_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 pm-console-queue-transport";

/** What the probe needs: the broker's public route and the Access
 * service token that may ask it. The token is a Worker secret, put
 * there by infra/pm-cloudflare's apply (by name from the vault: the
 * same cloudflare.access.queue-broker-client-id/-secret the retired
 * Worker's deploy used — provisioned by name, never copied off the
 * retiring Worker). */
export interface TransportProbeConfig {
  brokerUrl: string;
  accessId?: string;
  accessSecret?: string;
  timeoutMs?: number;
}

export type BrokerProbe = { ok: boolean; detail: string } | { skipped: string };

/** Ask the broker's /healthz through Cloudflare Access. A probe that
 * cannot authenticate itself is skipped, never an alert: that posture
 * is the retired Worker's, kept deliberately. The deadline is
 * timed()'s, not AbortSignal.timeout's: a timer left running keeps
 * the object in memory (see src/timed.ts). */
export async function probeBroker(
  fetchImpl: typeof fetch,
  config: TransportProbeConfig,
): Promise<BrokerProbe> {
  if (!config.accessId || !config.accessSecret) {
    return { skipped: "Access service token not configured" };
  }
  const url = `${String(config.brokerUrl).replace(/\/+$/, "")}/healthz`;
  try {
    return await timed(config.timeoutMs ?? 10000, async (signal) => {
      const res = await fetchImpl(url, {
        headers: {
          "CF-Access-Client-Id": config.accessId,
          "CF-Access-Client-Secret": config.accessSecret,
          "User-Agent": PROBE_UA,
        },
        redirect: "manual",
        signal,
      });
      return classifyHealthz(res.status, await res.text());
    });
  } catch (err) {
    const name = (err as { name?: string } | null)?.name;
    if (name === "TimeoutError" || name === "AbortError") {
      return { ok: false, detail: `no response from ${url} within the probe timeout` };
    }
    return {
      ok: false,
      detail: `request to ${url} failed: ${String(err instanceof Error ? err.message : err)}`,
    };
  }
}

/** The incident half of GitHub, as the probe uses it. Deliberately no
 * close(): automation must never close an issue (#10136). */
export interface IncidentGitHub {
  /** Number of an open incident issue, or null. */
  find(): Promise<number | null>;
  file(title: string, body: string): Promise<number>;
  comment(number: number, body: string): Promise<void>;
}

/** The incident client over GitHub's REST API, with the console's own
 * token (a Worker secret, like the retired Worker's own). Each method
 * throws on a non-2xx response so the caller can keep the window
 * un-alerted (or un-recovered) and retry on the next tick. */
export function makeIncidentGitHub(
  token: string,
  repo: string,
  fetchImpl: typeof fetch,
  apiOrigin = "https://api.github.com",
): IncidentGitHub {
  async function call(method: string, path: string, body?: unknown): Promise<unknown> {
    const res = await fetchImpl(`${apiOrigin}${path}`, {
      method,
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "pm-console-queue-transport",
        ...(body ? { "Content-Type": "application/json" } : {}),
      },
      body: body ? JSON.stringify(body) : undefined,
    });
    const text = await res.text();
    if (!res.ok) {
      throw new Error(`GitHub ${method} ${path} -> HTTP ${res.status}: ${text.slice(0, 200)}`);
    }
    return text ? JSON.parse(text) : null;
  }

  return {
    async find() {
      const q = `repo:${repo} is:issue is:open in:title "${INCIDENT_TITLE_PREFIX}"`;
      const data = (await call("GET", `/search/issues?q=${encodeURIComponent(q)}&per_page=20`)) as {
        items?: { title?: unknown; number?: unknown }[];
      } | null;
      const hit = (data?.items || []).find((i) => String(i.title || "").startsWith(INCIDENT_TITLE_PREFIX));
      return hit && typeof hit.number === "number" ? hit.number : null;
    },
    async file(title, body) {
      const issue = (await call("POST", `/repos/${repo}/issues`, {
        title,
        body,
        labels: INCIDENT_LABELS,
      })) as { number: number };
      return issue.number;
    },
    async comment(number, body) {
      await call("POST", `/repos/${repo}/issues/${number}/comments`, { body });
    },
  };
}

/** The post-outage triage task's packet: the same repo-task/v1 packet
 * the retired Worker enqueued through the broker's /enqueue verb
 * (its src/queue.js), anchored to the incident issue, so its dedupe
 * key is the canonical triage key for that issue and a retry cannot
 * double-file. Filed from here through the console's own queue door
 * (directClasses' queueEnqueue, the console's QUEUE_TOKEN). */
export function buildOutagePacket(item: TransportPendingEnqueue, repo: string, nowMs: number): unknown {
  const now = new Date(nowMs).toISOString();
  const url = `https://github.com/${repo}/issues/${item.incident}`;
  const window = `${item.start} to ${item.end}`;
  const prompt = [
    `The repo-queue broker was unreachable from ${window} (queue transport watch, #13503; formerly code#6369).`,
    `Incident: ${url}.`,
    "Find out why: broker host off, tailscaled stopped, broker process down, or the Cloudflare tunnel.",
    "Record the cause on the incident. If a fix or guard is needed, open it or file a follow-up issue.",
    "Do not close the incident; a human closes it (#10136).",
  ].join("\n");
  return {
    title: `Queue broker outage ${window}: find the cause (#${item.incident})`,
    body: prompt,
    priority: 20,
    dedupe_key: `repo-health:v1:triage:${repo}:issue:${item.incident}`,
    payload: {
      schema: "repo-task/v1",
      kind: "triage",
      repo,
      problem: `The repo-queue broker was unreachable from ${window}; lanes starved at their health gate.`,
      evidence: [
        `PM console queue transport watch: consecutive failed probes of the broker's public /healthz from ${window}`,
        `Incident issue: ${url}`,
      ],
      acceptance: [
        "The outage's cause is identified and recorded on the incident issue",
        "A fix PR or follow-up issue exists if the cause needs one",
      ],
      scope: {
        include: [`${repo}: the broker host, tailscale, the broker service and the pm-stack tunnel`],
        exclude: ["Closing the incident (a human closes it, #10136)", "Unrelated refactors"],
      },
      tests: ["None unless a fix lands; then the touched area's standard checks"],
      risk: { level: "low", flags: [] },
      priority_reason: "A queue outage starves every lane; knowing why prevents the next one.",
      timebox_minutes: 20,
      max_attempts: 1000,
      context_refs: [url],
      dedupe: { checked_at: now, basis: `Anchored to incident issue #${item.incident}; canonical triage key.` },
      issue: { number: item.incident, url, state_checked_at: now },
      origin: "queue-transport-alert",
      prompt,
    },
  };
}

export interface TransportPassClients {
  /** null = no GitHub writes (direct off, or no token): the window
   * still alerts and the owner is still told from the verdict. */
  github: IncidentGitHub | null;
  /** Files one triage packet into the queue. null = not this side's
   * to file: a pending enqueue is dropped, as the retired Worker's
   * observe mode dropped it. */
  enqueue: ((packet: unknown) => Promise<void>) | null;
}

const MAX_PENDING = 20;

function errText(err: unknown): string {
  return String(err instanceof Error ? err.message : err);
}

/**
 * One alert pass: probe result in, state advanced, side effects done.
 * A port of the retired Worker's run.js, minus its notify/push items:
 * telling the owner is the console alarm's, from the verdict this
 * pass leaves (transportNotifyDecision). What remains here is the
 * incident record and the recovery enqueue, under the same retry
 * discipline: a failed GitHub write leaves the window un-alerted (or
 * un-recovered) so the next tick retries it; the enqueue rides the
 * pending list and is only attempted while the broker answers.
 */
export async function runTransportPass(
  prev: TransportProbeState | null,
  probe: { ok: boolean; detail: string },
  clients: TransportPassClients,
  nowMs: number,
  opts: { threshold?: number; repo?: string; probeDesc?: string } = {},
): Promise<{ state: TransportProbeState; event: string; message: string; errors: string[] }> {
  const repo = opts.repo ?? QUEUE_TRANSPORT_REPO;
  const probeDesc =
    opts.probeDesc ?? "GET the broker's /healthz through Cloudflare Access, from the PM console edge";
  const { github, enqueue } = clients;
  const stepped = stepProbe(
    prev || emptyProbeState(),
    probe.ok,
    probe.detail,
    nowMs,
    opts.threshold ?? QUEUE_TRANSPORT_THRESHOLD,
  );
  let state = stepped.state;
  const event = stepped.event;
  const errors: string[] = [];
  let message = "";

  if (event === "alert") {
    message = alertMessage(state, nowMs);
    if (github) {
      try {
        const existing = await github.find();
        if (existing) {
          await github.comment(existing, `${message}\n\nLast failure: ${state.last_failure_detail || "n/a"}`);
          state.incident = existing;
          state.incident_filed_by_probe = false;
        } else {
          state.incident = await github.file(incidentTitle(state), incidentBody(state, nowMs, probeDesc));
          state.incident_filed_by_probe = true;
        }
        message += ` Incident #${state.incident}.`;
      } catch (err) {
        errors.push(errText(err));
        // Leave the window un-alerted so the next tick retries the write.
        state.alerted = false;
        state.incident = null;
        state.incident_filed_by_probe = false;
      }
    }
    state.alert_notified = true;
  } else if (event === "recovered") {
    message = recoveryMessage(state, nowMs);
    state.recovery_notified = true;
    let commented = true;
    if (github && state.incident) {
      try {
        // Comment only. Automation may label and comment but must never
        // close an issue (#10136), even one it filed; a human closes it.
        await github.comment(state.incident, message);
      } catch (err) {
        errors.push(errText(err));
        commented = false; // stay alerted: the next passing tick retries
      }
    }
    if (commented) {
      if (state.incident) {
        state.pending = [
          ...(state.pending || []),
          { type: "enqueue" as const, incident: state.incident, start: state.window_start, end: iso(nowMs) },
        ].slice(-MAX_PENDING);
      }
      state = finishProbeRecovery(state, nowMs);
    }
  }

  // Deliver what is pending. A delivery with no client configured is
  // dropped (observe mode); a failed one stays for the next tick. The
  // enqueue is only attempted while the broker answers, which during
  // an outage it does not: that is why it is filed on recovery.
  const left: TransportPendingEnqueue[] = [];
  for (const item of state.pending || []) {
    try {
      if (item.type === "enqueue") {
        if (enqueue && !probe.ok) {
          left.push(item);
          continue;
        }
        if (enqueue) await enqueue(buildOutagePacket(item, repo, nowMs));
      }
    } catch (err) {
      errors.push(`enqueue: ${errText(err)}`);
      left.push(item);
    }
  }
  state.pending = left;
  state.last_errors = errors;
  return { state, event, message, errors };
}

/** The verdict a probe state states, in the shape the console keeps
 * and the health answer shows: the retired Worker's publicStatus,
 * mapped. Stale is derived at read time from the last probe, exactly
 * as the Worker's /status derived it. */
export function transportVerdict(state: TransportProbeState, nowMs: number, repo = QUEUE_TRANSPORT_REPO): QueueTransport {
  const lastProbeMs = state.last_probe ? Date.parse(state.last_probe) : null;
  let status: string;
  if (!lastProbeMs) status = "unknown";
  else if (nowMs - lastProbeMs > QUEUE_TRANSPORT_STALE_MS) status = "stale";
  else if (state.alerted) status = "down";
  else if ((state.consecutive_failures || 0) > 0) status = "degraded";
  else status = "up";
  return {
    status,
    consecutiveFailures: state.consecutive_failures || 0,
    failingSince: state.window_start || null,
    lastFailure: state.consecutive_failures ? state.last_failure_detail || null : null,
    lastCheck: state.last_probe || null,
    lastOk: state.last_ok || null,
    incident:
      state.alerted && state.incident
        ? { number: state.incident, url: `https://github.com/${repo}/issues/${state.incident}` }
        : null,
    checkedAt: nowMs,
  };
}
