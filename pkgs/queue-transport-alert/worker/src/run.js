// One alert pass: probe, advance the state, perform the GitHub writes
// the transition asks for. Pure apart from the injected probe, github
// and clock, so tests drive it without a Worker runtime.
import {
  alertMessage,
  emptyState,
  finishRecovery,
  incidentBody,
  incidentTitle,
  recoveryMessage,
  step,
} from "./logic.js";

/**
 * @param {object} prev            stored state (or null)
 * @param {{ok:boolean, detail:string}} probe
 * @param {object|null} github     null = observe only
 * @param {number} nowMs
 * @param {number} threshold
 * @param {string} probeDesc
 * @returns {Promise<{state, event, message, incident, errors:string[]}>}
 */
export async function runPass(prev, probe, github, nowMs, threshold, probeDesc) {
  const { state, event } = step(prev || emptyState(), probe.ok, probe.detail, nowMs, threshold);
  const errors = [];
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
        errors.push(String(err && err.message ? err.message : err));
        // Leave the window un-alerted so the next tick retries the write.
        state.alerted = false;
        state.incident = null;
        state.incident_filed_by_probe = false;
      }
    }
    return { state, event, message, incident: state.incident, errors };
  }

  if (event === "recovered") {
    message = recoveryMessage(state, nowMs);
    if (github && state.incident) {
      try {
        // Comment only. Automation may label and comment but must never
        // close an issue (#10136), even one it filed; a human closes it.
        await github.comment(state.incident, message);
      } catch (err) {
        errors.push(String(err && err.message ? err.message : err));
        // Stay alerted: the next passing tick retries what is left.
        return { state, event, message, incident: state.incident, errors };
      }
    }
    const incident = state.incident;
    return { state: finishRecovery(nowMs), event, message, incident, errors };
  }

  return { state, event, message, incident: state.incident, errors };
}
