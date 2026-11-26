// C6 incident pages (#15019, design §6): assemble one evidence-backed
// incident page as a §1.1 page-data envelope and render it phone-first.
//
// Evidence discipline (design §6.1): the facts come from the canonical
// C3 healthHook records stored on the alerts ledger's opened/closed
// event details (#12602) — never from the thin audit projection.
// Identity joins on healthHook.incidentId / healthHook.recoveryOf;
// alerts-ledger event IDs are preserved separately because they are
// not incident IDs. Anything the records do not show is labelled
// unknown or provisional — never filled in.
import { createHash } from "node:crypto";
import type { Incident, IncidentEvent } from "./alerts.ts";
import { validateHealthHook, type HealthHookRecord } from "./health-hook.ts";

export const C6_PAGE_SCHEMA = "c6.page-data/v1";
export const INCIDENT_TEMPLATE = { id: "c6-incident", version: 1 } as const;

/** The §6.1 template sections, in render order. IDs are stable (§4.2):
 * renames and reorders of future templates must preserve them. */
export const INCIDENT_SECTION_IDS = [
  "current-state",
  "story",
  "timeline-attempts",
  "findings",
  "related",
  "root-cause",
  "remediation",
  "comments-questions",
] as const;

export type C6SectionKind = "facts" | "narrative" | "chart" | "questions" | "comments";

export interface C6Section {
  id: string;
  title: string;
  kind: C6SectionKind;
  data: Record<string, unknown>;
}

export interface C6PageEnvelope {
  schema: string;
  page_id: string;
  record_id: string;
  page_class: "incident";
  content_version: string;
  generated_at: string;
  as_of: string;
  freshness: {
    budget_seconds: number;
    last_source_event_at: string;
    state: "fresh" | "stale" | "unavailable";
    reason: string | null;
  };
  sources: { id: string; cursor: string; observed_at: string }[];
  template: { id: string; version: number };
  sections: C6Section[];
  links: { rel: string; href: string }[];
}

/** Health-hook targets that probe a core service / routing tier (§6.2a). */
const ROUTING_TIER_TARGETS = new Set(["opencode-working", "pm-stack-reachable"]);
const MIN_CORE_OUTAGE_SECONDS = 30 * 60;
/** Freshness budgets (§1.2): active incident 15 min, closed 24 h. */
const ACTIVE_INCIDENT_BUDGET_SECONDS = 900;
const CLOSED_INCIDENT_BUDGET_SECONDS = 86_400;

const ISSUE_15019 = "https://github.com/developing-today/code/issues/15019";
const ISSUE_9574 = "https://github.com/developing-today/code/issues/9574";

/** The target segment of a health-hook incident key
 * (`health-hook:<target>:<healthHook incidentId>`), or null. */
export function incidentTarget(incident: Pick<Incident, "incident_key">): string | null {
  const parts = incident.incident_key.split(":");
  if (parts.length < 3 || parts[0] !== "health-hook") return null;
  return parts.slice(1, -1).join(":");
}

function healthHookIdOf(incident: Pick<Incident, "incident_key">): string | null {
  const parts = incident.incident_key.split(":");
  return parts.length >= 3 ? parts[parts.length - 1] : null;
}

function impactSeconds(incident: Incident): number {
  const end = incident.closed_at ?? incident.last_seen_at;
  return Math.max(0, (Date.parse(end) - Date.parse(incident.first_seen_at)) / 1000);
}

/**
 * §6.2 page threshold. Clauses (b) multi-provider wall and (c)
 * user-visible coordinated outage are not derivable from ledger
 * records alone, so only (a) and (d) can qualify a page from this
 * evidence; the rest is a human call recorded on the issue.
 */
export function incidentPageThreshold(
  incident: Incident,
  _events: IncidentEvent[],
  related: Incident[],
): { qualifies: boolean; clauses: string[] } {
  const clauses: string[] = [];
  const target = incidentTarget(incident);
  if (target && ROUTING_TIER_TARGETS.has(target) && impactSeconds(incident) >= MIN_CORE_OUTAGE_SECONDS) {
    clauses.push("a");
  }
  if (target && related.filter((r) => r.id !== incident.id && incidentTarget(r) === target).length >= 2) {
    clauses.push("d");
  }
  return { qualifies: clauses.length > 0, clauses };
}

/** The validated healthHook record carried on an event's detail, if any. */
function healthHookOf(event: IncidentEvent): HealthHookRecord | null {
  const raw = event.detail?.healthHook;
  if (raw === undefined) return null;
  const v = validateHealthHook(raw);
  return v.ok ? v.value : null;
}

function canonical(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value) ?? "null";
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  const entries = Object.entries(value as Record<string, unknown>)
    .filter(([, v]) => v !== undefined)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${canonical(v)}`).join(",")}}`;
}

const sha256 = (text: string): string => createHash("sha256").update(text).digest("hex");

export interface BuildIncidentEnvelopeInput {
  incident: Incident;
  events: IncidentEvent[];
  related: Incident[];
  generatedAt: string;
}

export function buildIncidentEnvelope(input: BuildIncidentEnvelopeInput): C6PageEnvelope {
  const { incident, events, related, generatedAt } = input;
  const sortedEvents = [...events].sort((a, b) => a.id - b.id);
  const opened = sortedEvents.find((e) => e.kind === "opened");
  const closed = sortedEvents.find((e) => e.kind === "closed");
  const down = opened ? healthHookOf(opened) : null;
  const up = closed ? healthHookOf(closed) : null;
  const target = down?.target ?? incidentTarget(incident) ?? "unknown";
  const serviceName = down?.stackHealth?.name ?? target;
  const hookId = down?.incidentId ?? healthHookIdOf(incident) ?? incident.id;
  const pageId = `incident:${target}:${hookId}`;
  const asOf = sortedEvents.length ? sortedEvents[sortedEvents.length - 1].at : incident.last_seen_at;
  const ledgerCursor = sortedEvents.length ? String(sortedEvents[sortedEvents.length - 1].id) : "";
  const isClosed = incident.status === "closed";
  const budget = isClosed ? CLOSED_INCIDENT_BUDGET_SECONDS : ACTIVE_INCIDENT_BUDGET_SECONDS;
  const ageSeconds = (Date.parse(generatedAt) - Date.parse(asOf)) / 1000;
  const fresh = ageSeconds <= budget;
  const freshness = {
    budget_seconds: budget,
    last_source_event_at: asOf,
    state: (fresh ? "fresh" : "stale") as "fresh" | "stale",
    reason: fresh
      ? null
      : isClosed
        ? "closed incident: no new source events within the 24h budget; the record is immutable and updates arrive as new versions"
        : "incident open and no source events within the 15-minute budget",
  };
  const sources = [
    { id: "alerts-ledger", cursor: ledgerCursor, observed_at: asOf },
    {
      id: `health-hook:${target}`,
      cursor: hookId,
      observed_at: up?.firedAt ?? down?.firedAt ?? asOf,
    },
  ];
  const durationSeconds = Math.floor(impactSeconds(incident));
  const downEvidence = (down?.evidence ?? {}) as Record<string, any>;
  const upEvidence = (up?.evidence ?? {}) as Record<string, any>;

  const sections: C6Section[] = [
    {
      id: "current-state",
      title: "Current state",
      kind: "facts",
      data: {
        status: incident.status,
        affected_service: serviceName,
        target,
        severity: incident.severity,
        impact_interval: {
          start: incident.first_seen_at,
          end: incident.closed_at,
          duration_seconds: durationSeconds,
        },
        last_updated: incident.last_seen_at,
        occurrences: incident.occurrences,
        push_delivered_at: incident.delivered_at,
        escalated_at: incident.escalated_at,
      },
    },
    {
      id: "story",
      title: "Story",
      kind: "narrative",
      data: {
        start: down?.firedAt ?? incident.first_seen_at,
        detection: opened?.at ?? incident.first_seen_at,
        response:
          `The health hook re-confirmed the failure ${incident.occurrences} times while the incident was open. ` +
          (incident.acked_at
            ? `Acknowledged at ${incident.acked_at}.`
            : "No acknowledgement or escalation is recorded in the ledger."),
        acknowledged_at: incident.acked_at,
        resolution: closed?.at ?? null,
        end: incident.closed_at,
      },
    },
    {
      id: "timeline-attempts",
      title: "Progress timeline and fix attempts",
      kind: "facts",
      data: {
        timeline: sortedEvents.map((e) => ({
          at: e.at,
          kind: e.kind,
          event_id: e.id,
          source: `alerts ledger event #${e.id}`,
        })),
        attempts:
          up && closed
            ? [
                {
                  action: `Restart of the ${upEvidence.serviceState?.unit ?? target} unit`,
                  outcome: "succeeded",
                  at_reported: upEvidence.serviceState?.activeEnterTimestamp ?? null,
                  source: `alerts ledger event #${closed.id} (healthHook recovery record)`,
                },
              ]
            : [],
        // The ledger carries no failed-attempt records for this
        // incident; saying so beats inventing trials.
        no_failed_attempts_recorded: true,
      },
    },
    {
      id: "findings",
      title: "Findings and evidence",
      kind: "chart",
      data: {
        series: [
          ...(down
            ? [
                {
                  label: "At detection (down record)",
                  at: down.firedAt,
                  sessions_last_hour: downEvidence.sessionLiveness?.sessionsLastHour ?? null,
                },
              ]
            : []),
          ...(up
            ? [
                {
                  label: "At recovery (up record)",
                  at: up.firedAt,
                  sessions_last_hour: upEvidence.sessionLiveness?.sessionsLastHour ?? null,
                },
              ]
            : []),
        ],
        evidence: sortedEvents
          .map((e) => {
            const record = healthHookOf(e);
            if (!record) return null;
            return {
              source: `alerts ledger event #${e.id} (${e.kind} detail, healthHook record)`,
              at: record.firedAt,
              reachability: record.evidence.reachability ?? null,
              service_state: record.evidence.serviceState ?? null,
              auth_health: record.evidence.authHealth ?? null,
            };
          })
          .filter((e) => e !== null),
      },
    },
    {
      id: "related",
      title: "Related incidents",
      kind: "facts",
      data: {
        incidents: related
          .filter((r) => r.id !== incident.id)
          .map((r) => ({
            ledger_id: r.id,
            health_hook_id: healthHookIdOf(r),
            title: r.title,
            status: r.status,
            first_seen_at: r.first_seen_at,
            closed_at: r.closed_at,
            occurrences: r.occurrences,
          })),
      },
    },
    {
      id: "root-cause",
      title: "Root cause",
      kind: "narrative",
      data: {
        // Provisional by construction: the records show the unit was
        // SIGTERM-stopped and later restarted, but never who or what
        // stopped it. Verified would require a source that names it.
        status: "provisional",
        summary:
          `The ${downEvidence.serviceState?.unit ?? target} unit was stopped with SIGTERM ` +
          `(systemd result ${downEvidence.serviceState?.result ?? "unknown"}; state ` +
          `${downEvidence.serviceState?.activeState ?? "unknown"}/${downEvidence.serviceState?.subState ?? "unknown"}, ` +
          `exit at ${downEvidence.serviceState?.exitTimestamp ?? "unknown"} as reported) and stayed down until the ` +
          `restart recorded in the recovery record (active at ${upEvidence.serviceState?.activeEnterTimestamp ?? "unknown"} ` +
          `as reported). The source records do not identify what stopped the unit, so the cause is unverified.`,
      },
    },
    {
      id: "remediation",
      title: "Remediation",
      kind: "facts",
      data: {
        horizons: [
          {
            horizon: "short",
            action: `Restart the ${upEvidence.serviceState?.unit ?? target} unit and confirm reachability`,
            status: closed ? "done (evidenced by the recovery record)" : "pending",
            issue: null,
            owner: null,
            due: null,
          },
          {
            horizon: "medium",
            action: "Keep the recurrence under a durable record: this incident page and the C6 parent",
            status: "in progress",
            issue: ISSUE_15019,
            owner: null,
            due: null,
          },
          {
            horizon: "long",
            action: "Determine what stopped the unit (the SIGTERM source) and prevent recurrence",
            status: "not started",
            issue: null,
            owner: null,
            due: null,
          },
        ],
      },
    },
    {
      id: "comments-questions",
      title: "Comments and follow-up questions",
      kind: "comments",
      data: {
        object_ref: `incident:${incident.id}`,
        alerts_url: `/alerts#incident-${incident.id}`,
        note: "Comments and follow-up questions attach through the dashboard comment machinery on this object ref; they are not embedded in the page content hash (C6 design §4.1).",
      },
    },
  ];

  const links = [
    { rel: "incident", href: `/alerts#incident-${incident.id}` },
    { rel: "issue", href: ISSUE_15019 },
    { rel: "issue", href: ISSUE_9574 },
  ];

  // §1.1: the hash covers the rendered-source content (section IDs and
  // values, as_of, source cursors) but excludes transport-only
  // timestamps (generated_at) and storage identifiers (record_id);
  // freshness state is reader-computed (§1.2), so it is excluded too.
  const contentVersion = `sha256:${sha256(
    canonical({
      schema: C6_PAGE_SCHEMA,
      page_id: pageId,
      page_class: "incident",
      template: INCIDENT_TEMPLATE,
      as_of: asOf,
      freshness_budget_seconds: budget,
      last_source_event_at: asOf,
      sources,
      sections,
      links,
    }),
  )}`;
  // §1.3: stable record ID derived from (page_id, template_version,
  // source_cursor), in the reportRecordId house style.
  const recordId = `c6page:${sha256(
    JSON.stringify([pageId, `${INCIDENT_TEMPLATE.id}@${INCIDENT_TEMPLATE.version}`, ledgerCursor]),
  )}`;

  return {
    schema: C6_PAGE_SCHEMA,
    page_id: pageId,
    record_id: recordId,
    page_class: "incident",
    content_version: contentVersion,
    generated_at: generatedAt,
    as_of: asOf,
    freshness,
    sources,
    template: { ...INCIDENT_TEMPLATE },
    sections,
    links,
  };
}

/** §1.1 server-side validation: required fields, known schema version,
 * and — for incident pages — §6.1 section completeness and order.
 * Unknown fields are preserved; unknown major schema versions are
 * rejected. */
export function validatePageEnvelope(value: unknown): { ok: true } | { ok: false; error: string } {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ok: false, error: "envelope must be an object" };
  }
  const e = value as Record<string, unknown>;
  if (e.schema !== C6_PAGE_SCHEMA) {
    return { ok: false, error: `unknown schema ${String(e.schema)} (expected ${C6_PAGE_SCHEMA})` };
  }
  for (const field of ["page_id", "record_id", "content_version", "generated_at", "as_of"] as const) {
    if (typeof e[field] !== "string" || !(e[field] as string).length) {
      return { ok: false, error: `envelope.${field} is required` };
    }
  }
  if (!Array.isArray(e.sources) || e.sources.length === 0) {
    return { ok: false, error: "envelope.sources must be a non-empty array" };
  }
  if (!e.freshness || typeof e.freshness !== "object") {
    return { ok: false, error: "envelope.freshness is required" };
  }
  if (!Array.isArray(e.sections)) return { ok: false, error: "envelope.sections must be an array" };
  if (e.page_class === "incident") {
    const ids = (e.sections as { id?: unknown }[]).map((s) => s.id);
    if (JSON.stringify(ids) !== JSON.stringify([...INCIDENT_SECTION_IDS])) {
      return { ok: false, error: "incident sections must be the §6.1 template sections in order" };
    }
  }
  return { ok: true };
}

// --- Rendering ---------------------------------------------------------------

const esc = (value: unknown): string =>
  String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");

function fmtDuration(totalSeconds: number): string {
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  const s = totalSeconds % 60;
  return h > 0 ? `${h}h ${String(m).padStart(2, "0")}m` : m > 0 ? `${m}m ${String(s).padStart(2, "0")}s` : `${s}s`;
}

type Data = Record<string, any>;

function renderSection(section: C6Section): string {
  const d = section.data as Data;
  const head = `<h2 id="${section.id}-title">${esc(section.title)}</h2>`;
  const wrap = (inner: string) =>
    `<section id="${esc(section.id)}" aria-labelledby="${esc(section.id)}-title">${head}${inner}</section>`;
  switch (section.id) {
    case "current-state": {
      const iv = d.impact_interval;
      return wrap(
        `<dl>` +
          `<dt>Status</dt><dd>${esc(d.status)}</dd>` +
          `<dt>Affected service</dt><dd>${esc(d.affected_service)} (${esc(d.target)})</dd>` +
          `<dt>Severity</dt><dd>${esc(d.severity)}</dd>` +
          `<dt>Impact window</dt><dd>${esc(iv.start)} → ${esc(iv.end ?? "ongoing")} (${esc(fmtDuration(iv.duration_seconds))})</dd>` +
          `<dt>Last updated</dt><dd>${esc(d.last_updated)}</dd>` +
          `<dt>Hook confirmations</dt><dd>${esc(d.occurrences)}</dd>` +
          `<dt>Push delivery</dt><dd>${d.push_delivered_at ? esc(d.push_delivered_at) : "none recorded"}</dd>` +
          `</dl>`,
      );
    }
    case "story":
      return wrap(
        `<dl>` +
          `<dt>Start</dt><dd>${esc(d.start)}</dd>` +
          `<dt>Detection</dt><dd>${esc(d.detection)} (alerts ledger opened event)</dd>` +
          `<dt>Response</dt><dd>${esc(d.response)}</dd>` +
          `<dt>Resolution</dt><dd>${d.resolution ? esc(d.resolution) : "not recorded"}</dd>` +
          `</dl>`,
      );
    case "timeline-attempts": {
      const rows = (d.timeline as Data[])
        .map((t) => `<tr><td>${esc(t.at)}</td><td>${esc(t.kind)}</td><td>${esc(t.source)}</td></tr>`)
        .join("");
      const attempts = (d.attempts as Data[])
        .map(
          (a) =>
            `<li>${esc(a.action)} — <strong>${esc(a.outcome)}</strong> (unit active at ${esc(a.at_reported ?? "unknown")}, as reported; ${esc(a.source)})</li>`,
        )
        .join("");
      return wrap(
        `<div class="tblwrap"><table><caption>Progress timeline (alerts ledger events)</caption><thead><tr><th scope="col">At</th><th scope="col">Event</th><th scope="col">Source</th></tr></thead><tbody>${rows}</tbody></table></div>` +
          `<h2 class="subsection">Fix attempts</h2><ul>${attempts}</ul>` +
          (d.no_failed_attempts_recorded
            ? `<p>No failed fix attempts are recorded in the alerts ledger for this incident.</p>`
            : ""),
      );
    }
    case "findings": {
      const series = d.series as Data[];
      const bars = series
        .map((p, i) => {
          const v = typeof p.sessions_last_hour === "number" ? p.sessions_last_hour : 0;
          const y = 10 + i * 28;
          return `<rect x="10" y="${y}" width="${v * 2}" height="18" fill="#4c9aff"></rect><text x="${14 + v * 2}" y="${y + 14}" fill="currentColor">${v}</text>`;
        })
        .join("");
      const chart = `<svg viewBox="0 0 220 ${series.length * 28 + 10}" role="img" aria-label="Bar chart of sessions in the last hour at detection and at recovery" width="220">${bars}</svg>`;
      const summary =
        `<p>Sessions in the last hour: ${esc(series[0]?.sessions_last_hour ?? "unknown")} at detection` +
        (series[1]
          ? ` (${esc(series[0].at)}) and ${esc(series[1].sessions_last_hour ?? "unknown")} at recovery (${esc(series[1].at)})`
          : "") +
        `. Source: healthHook evidence sessionLiveness on the alerts ledger opened/closed event details.</p>`;
      const rows = series
        .map(
          (p) =>
            `<tr><td>${esc(p.label)}</td><td>${esc(p.at)}</td><td>${esc(p.sessions_last_hour ?? "unknown")}</td></tr>`,
        )
        .join("");
      const table =
        `<div class="tblwrap"><table><caption>Sessions in the last hour (healthHook sessionLiveness evidence)</caption>` +
        `<thead><tr><th scope="col">Point</th><th scope="col">At</th><th scope="col">Sessions last hour</th></tr></thead><tbody>${rows}</tbody></table></div>`;
      const evidence = (d.evidence as Data[])
        .map(
          (e) =>
            `<li><strong>${esc(e.source)}</strong> at ${esc(e.at)} — reachability ok: ${esc(e.reachability?.ok ?? "unknown")}` +
            ` (status ${esc(e.reachability?.status ?? "none")}); unit ${esc(e.service_state?.unit ?? "unknown")} ` +
            `${esc(e.service_state?.activeState ?? "unknown")}/${esc(e.service_state?.subState ?? "unknown")}; ` +
            `auth ${esc(e.auth_health?.status ?? "unknown")} (${esc(e.auth_health?.detail ?? "unknown")})</li>`,
        )
        .join("");
      return wrap(`${chart}${summary}${table}<ul>${evidence}</ul>`);
    }
    case "related": {
      const rows = (d.incidents as Data[])
        .map(
          (r) =>
            `<tr><td><a href="/reports/incident/${esc(r.ledger_id)}">${esc(r.title)}</a></td><td>${esc(r.status)}</td><td>${esc(r.first_seen_at)}</td><td>${esc(r.closed_at ?? "ongoing")}</td><td>${esc(r.occurrences)}</td></tr>`,
        )
        .join("");
      return wrap(
        `<p>Same target, from the alerts ledger. This recurrence is why the page exists (C6 design §6.2 clause d).</p>` +
          `<div class="tblwrap"><table><caption>Related incidents for the same target</caption><thead><tr><th scope="col">Incident</th><th scope="col">Status</th><th scope="col">First seen</th><th scope="col">Closed</th><th scope="col">Confirmations</th></tr></thead><tbody>${rows}</tbody></table></div>`,
      );
    }
    case "root-cause":
      return wrap(
        `<p><strong>Provisional root cause.</strong> ${esc(d.summary)}</p>` +
          `<p>Status: ${esc(d.status)} — it stays provisional until a source record verifies what stopped the unit.</p>`,
      );
    case "remediation": {
      const items = (d.horizons as Data[])
        .map(
          (h) =>
            `<li><strong>${esc(h.horizon)}:</strong> ${esc(h.action)} — status ${esc(h.status)}; ` +
            `${h.issue ? `<a href="${esc(h.issue)}">linked issue</a>` : "no linked issue"}; ` +
            `${h.owner ? `owner ${esc(h.owner)}` : "owner unknown"}; ${h.due ? `due ${esc(h.due)}` : "due date unknown"}</li>`,
        )
        .join("");
      return wrap(`<ul>${items}</ul>`);
    }
    case "comments-questions":
      return wrap(
        `<p>${esc(d.note)}</p><p>Object ref: <code>${esc(d.object_ref)}</code> · ` +
          `<a href="${esc(d.alerts_url)}">Open the incident card on /alerts</a></p>`,
      );
    default:
      return wrap(`<pre>${esc(JSON.stringify(d, null, 2))}</pre>`);
  }
}

/** Phone-first render (§2): title and freshness first, then the §6.1
 * sections in order. A fragment in the house style of the other report
 * renders — the server wraps it like /reports. */
export function renderIncidentPage(envelope: C6PageEnvelope): string {
  const current = envelope.sections.find((s) => s.id === "current-state")?.data as Data | undefined;
  const title = `Incident: ${current?.affected_service ?? envelope.page_id} down`;
  const freshness =
    `<p class="freshness">As of ${esc(envelope.as_of)} · freshness: ${esc(envelope.freshness.state)}` +
    (envelope.freshness.reason ? ` — ${esc(envelope.freshness.reason)}` : "") +
    ` · page <code>${esc(envelope.page_id)}</code> · version <code>${esc(envelope.content_version)}</code></p>`;
  return `<h1>${esc(title)}</h1>${freshness}${envelope.sections.map(renderSection).join("")}`;
}
