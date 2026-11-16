// Integrated queue/PM view — human face (#7078, #2072 slice 3, second piece).
//
// The second face over the SAME read model as the machine face
// (src/queue-view.ts): `buildQueueView` output, rendered as one
// standalone HTML page/report. One read model, two faces — this module
// adds no join, no ingestion, and no write path: it renders exactly
// what the machine face emits (queued in claim order, claimed with the
// claiming system/lane, snapshot counts, hot-list consumption summary)
// and its trend panels LINK the existing aggregator/projector stores
// through the view's own `links` — never copies of their data.
//
// Pure: the same QueueView always renders the same page (modulo
// nothing at all — even `generated_at` comes from the view), so the
// rendering is unit-testable without a database or a live broker.
// Every interpolated value is HTML-escaped: titles and refs are
// GitHub/broker data, not markup.

import type { ClaimedEntry, QueuedEntry, QueueView } from "./queue-view";

/** Escape a value for HTML text/attribute context. */
export function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

const dash = (v: string | number | null | undefined): string =>
  v === null || v === undefined || v === "" ? "—" : escapeHtml(String(v));

function entryCells(e: QueuedEntry): string {
  const hot = e.hot_events.length
    ? `<ul class="hot">${e.hot_events
        .map(
          (h) =>
            `<li>#${h.seq} ${dash(h.event)}${h.action ? `/${dash(h.action)}` : ""}` +
            (h.actor ? ` by ${dash(h.actor)}` : "") +
            (h.source_url ? ` — <a href="${escapeHtml(h.source_url)}">source</a>` : "") +
            `</li>`,
        )
        .join("")}</ul>`
    : "";
  return (
    `<td>${e.task_id}</td><td>${dash(e.title)}${hot}</td><td>${dash(e.ref)}</td>` +
    `<td>${dash(e.kind)}</td><td>${dash(e.priority)}</td><td>${dash(e.created_at)}</td>`
  );
}

/**
 * Render a QueueView as a standalone HTML page. Sections, in order:
 * header (generated_at + window), status counts, queued (in the
 * view's claim order — the renderer never re-sorts), picked up
 * (claimed, with claiming system/lane), trend panels (one per link
 * the machine face emitted, href = the link's ref verbatim), and the
 * hot-list consumption summary.
 */
export function renderQueueViewHtml(view: QueueView): string {
  const counts = Object.entries(view.counts)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([status, n]) => `<tr><td>${escapeHtml(status)}</td><td>${n}</td></tr>`)
    .join("\n          ");

  const queued = view.queued.map((e, i) => `        <tr><td>${i + 1}</td>${entryCells(e)}</tr>`).join("\n");

  const claimed = view.claimed
    .map(
      (e: ClaimedEntry) =>
        `        <tr>${entryCells(e)}<td>${dash(e.claimed_at)}</td>` +
        `<td>${dash(e.claimed_by.worker_id)}</td><td>${dash(e.claimed_by.lane_id)}</td>` +
        `<td>${dash(e.claimed_by.platform)}</td></tr>`,
    )
    .join("\n");

  const trends = view.links
    .map(
      (l) =>
        `      <section class="trend-panel" data-store="${escapeHtml(l.store)}">\n` +
        `        <h3>${escapeHtml(l.label)}</h3>\n` +
        `        <p>Store: ${escapeHtml(l.store)} — <a href="${escapeHtml(l.ref)}">${escapeHtml(l.ref)}</a></p>\n` +
        `      </section>`,
    )
    .join("\n");

  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Queue/PM view — ${escapeHtml(view.window.since)} to ${escapeHtml(view.window.until)}</title>
<style>
  body { font-family: system-ui, sans-serif; margin: 2rem; }
  table { border-collapse: collapse; margin-bottom: 1.5rem; }
  th, td { border: 1px solid #ccc; padding: 0.25rem 0.6rem; text-align: left; vertical-align: top; }
  .trend-panel { border: 1px solid #ccc; border-radius: 0.5rem; padding: 0.5rem 1rem; margin-bottom: 1rem; }
  ul.hot { margin: 0.25rem 0 0; padding-left: 1.2rem; }
</style>
</head>
<body>
<header>
  <h1>Integrated queue/PM view</h1>
  <p>Generated at ${escapeHtml(view.generated_at)} — window ${escapeHtml(view.window.since)} to ${escapeHtml(view.window.until)}.
  Human face over the queue-view read model (#7078): the same join the machine face emits as JSON, rendered. It writes nothing and collects nothing new.</p>
</header>

<section id="counts">
  <h2>Status counts</h2>
  <table>
    <thead><tr><th>Status</th><th>Tasks</th></tr></thead>
    <tbody>
          ${counts}
    </tbody>
  </table>
</section>

<section id="queued">
  <h2>Queued — in claim order (${view.queued.length})</h2>
  <table>
    <thead><tr><th>#</th><th>Task</th><th>Title</th><th>Ref</th><th>Kind</th><th>Priority</th><th>Created</th></tr></thead>
    <tbody>
${queued}
    </tbody>
  </table>
</section>

<section id="claimed">
  <h2>Picked up in window (${view.claimed.length})</h2>
  <table>
    <thead><tr><th>Task</th><th>Title</th><th>Ref</th><th>Kind</th><th>Priority</th><th>Created</th><th>Claimed at</th><th>Worker (system)</th><th>Lane</th><th>Platform</th></tr></thead>
    <tbody>
${claimed}
    </tbody>
  </table>
</section>

<section id="trends">
  <h2>Trend panels — linked stores (not copies)</h2>
${trends}
</section>

<section id="hot-list">
  <h2>Hot list (slice-2 event stream)</h2>
  <p>Consumer ${dash(view.hot_list.consumer)} — consumed ${view.hot_list.events_consumed} event(s)
  from seq ${view.hot_list.from_seq} to high-water ${view.hot_list.high_water}.</p>
</section>

</body>
</html>
`;
}
