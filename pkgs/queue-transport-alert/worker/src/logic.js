// Pure decision logic for the queue transport alert (code#6369).
//
// Ported from scripts/fleet/queue-transport-alert/check_transport.py
// so the Worker and the local probe agree on one contract:
//   - `threshold` consecutive failed probes alert ONCE per outage window;
//   - the first passing probe after an alerted window recovers ONCE;
//   - a failing window that ends below the threshold ends quietly.
// No I/O here: the Durable Object feeds probe results and a clock in,
// and performs whatever GitHub writes the result asks for.

export const INCIDENT_TITLE_PREFIX = "Queue transport incident:";
export const INCIDENT_LABELS = ["kind/bug", "priority/high", "area/infra", "source/fleet-monitor"];

export function emptyState() {
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
    // Carried across windows:
    last_outage: null, // {start, end, incident} of the last alerted window
    pending: [], // side effects still to deliver (see run.js)
  };
}

/** A fresh window that keeps what outlives one (last outage, pending). */
function freshWindow(prev, nowMs, lastOutage) {
  return {
    ...emptyState(),
    last_ok: iso(nowMs),
    last_probe: iso(nowMs),
    last_outage: lastOutage === undefined ? (prev.last_outage ?? null) : lastOutage,
    pending: Array.isArray(prev.pending) ? prev.pending : [],
  };
}

export function iso(ms) {
  return new Date(ms).toISOString().replace(/\.\d{3}Z$/, "Z");
}

export function span(startMs, endMs) {
  const minutes = Math.max(0, Math.floor((endMs - startMs) / 60000));
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h ? `${h}h${String(m).padStart(2, "0")}m` : `${m}m`;
}

/** Advance the state by one probe result. Returns {state, event}. */
export function step(prev, ok, detail, nowMs, threshold) {
  const state = { ...emptyState(), ...prev, last_probe: iso(nowMs) };
  if (ok) {
    if (state.alerted) {
      return {
        state: { ...state, consecutive_failures: 0, last_ok: iso(nowMs) },
        event: "recovered",
      };
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
export function finishRecovery(state, nowMs) {
  return freshWindow(state, nowMs, {
    start: state.window_start,
    end: iso(nowMs),
    incident: state.incident ?? null,
  });
}

export function incidentTitle(state) {
  return `${INCIDENT_TITLE_PREFIX} repo-queue broker unreachable since ${state.window_start}`;
}

export function alertMessage(state, nowMs) {
  const start = Date.parse(state.window_start);
  return (
    `The repo-queue broker has been unreachable for ${span(start, nowMs)} ` +
    `(${state.consecutive_failures} failed probes since ${state.window_start}), ` +
    `so worker lanes are starving at their health gate.`
  );
}

export function recoveryMessage(state, nowMs) {
  const start = Date.parse(state.window_start);
  return (
    `The repo-queue broker is reachable again after about ${span(start, nowMs)} ` +
    `down (${state.window_start} to ${iso(nowMs)}).`
  );
}

export function incidentBody(state, nowMs, probeDesc) {
  return [
    "Filed automatically by the queue transport alert Worker " +
      "(`pkgs/queue-transport-alert/worker`, code#6369), which probes the " +
      "broker every 10 minutes from Cloudflare, off the broker host.",
    "",
    `- **Probe:** ${probeDesc}`,
    `- **Failing since:** ${state.window_start}`,
    `- **Consecutive failed probes:** ${state.consecutive_failures}`,
    `- **Last failure:** ${state.last_failure_detail || "n/a"}`,
    "",
    alertMessage(state, nowMs),
    "",
    "No queue task is filed now, because the queue is the thing that is " +
      "down. When the broker answers again the Worker enqueues one triage " +
      "task anchored to this issue to find the cause.",
    "",
    "The Worker comments here once when the transport recovers. Close this " +
      "issue by hand once the outage is understood; automation never closes " +
      "issues (#10136).",
  ].join("\n");
}

/**
 * Classify one /healthz response. Anything but a 200 whose JSON says
 * ok:true is a failed probe; the detail names what was seen so an
 * Access misconfiguration (302/401/403) reads differently from the
 * tunnel being down (502/530) in the incident.
 */
export function classifyHealthz(status, bodyText) {
  if (status === 200) {
    try {
      const body = JSON.parse(bodyText);
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

function clip(text) {
  const t = String(text || "").replace(/\s+/g, " ").trim();
  return t.length <= 200 ? t : `${t.slice(0, 197)}...`;
}
