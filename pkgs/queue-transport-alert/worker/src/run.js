// One alert pass: probe result in, state advanced, side effects done.
// Pure apart from the injected clients and clock, so tests drive it
// without a Worker runtime.
//
// Side effects per outage window:
//   alert     -> GitHub incident (file, or comment on an open one)
//                + one phone notification
//   recovered -> GitHub comment on the incident (never a close, #10136)
//                + one phone notification
//                + one fleet-queue triage task anchored to the incident
// Notifications and the queue task go through `state.pending`, so a
// failed delivery is retried on later ticks. The queue task is only
// attempted while the broker answers, which during an outage it does
// not: that is why it is filed on recovery.
import {
  alertMessage,
  emptyState,
  finishRecovery,
  incidentBody,
  incidentTitle,
  iso,
  recoveryMessage,
  step,
} from "./logic.js";

const MAX_PENDING = 20;

function errText(err) {
  return String(err && err.message ? err.message : err);
}

/**
 * @param {object} prev                 stored state (or null)
 * @param {{ok:boolean, detail:string}} probe
 * @param {object|null} github          null = no GitHub writes
 * @param {number} nowMs
 * @param {number} threshold
 * @param {string} probeDesc
 * @param {object} [extra]
 * @param {((text:string)=>Promise<void>)|null} [extra.notify]   phone push
 * @param {((item:object)=>Promise<void>)|null} [extra.enqueue]  fleet queue task
 * @param {string} [extra.repo]
 * @param {string} [extra.statusUrl]
 */
export async function runPass(prev, probe, github, nowMs, threshold, probeDesc, extra = {}) {
  const { notify = null, enqueue = null, repo = "developing-today/code", statusUrl = "" } = extra;
  let { state, event } = step(prev || emptyState(), probe.ok, probe.detail, nowMs, threshold);
  const errors = [];
  let message = "";
  const queue = (item) => {
    state.pending = [...(state.pending || []), item].slice(-MAX_PENDING);
  };
  const issueUrl = (n) => `https://github.com/${repo}/issues/${n}`;
  const statusLine = statusUrl ? `\nStatus: ${statusUrl}` : "";

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
    if (!state.alert_notified) {
      const link = state.incident ? `\n${issueUrl(state.incident)}` : "";
      queue({ type: "notify", text: `Queue broker DOWN. ${alertMessage(state, nowMs)}${link}${statusLine}` });
      state.alert_notified = true;
    }
  } else if (event === "recovered") {
    message = recoveryMessage(state, nowMs);
    if (!state.recovery_notified) {
      const link = state.incident ? `\n${issueUrl(state.incident)}` : "";
      queue({ type: "notify", text: `Queue broker back UP. ${message}${link}` });
      state.recovery_notified = true;
    }
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
        queue({ type: "enqueue", incident: state.incident, start: state.window_start, end: iso(nowMs) });
      }
      state = finishRecovery(state, nowMs);
    }
  }

  // Deliver what is pending. A delivery without a configured client is
  // dropped (observe mode); a failed one stays for the next tick.
  const left = [];
  for (const item of state.pending || []) {
    try {
      if (item.type === "notify") {
        if (notify) await notify(item.text);
      } else if (item.type === "enqueue") {
        if (enqueue && !probe.ok) {
          left.push(item); // the broker is down; try once it answers
          continue;
        }
        if (enqueue) await enqueue(item);
      }
    } catch (err) {
      errors.push(`${item.type}: ${errText(err)}`);
      left.push(item);
    }
  }
  state.pending = left;

  return { state, event, message, incident: state.incident ?? state.last_outage?.incident ?? null, errors };
}
