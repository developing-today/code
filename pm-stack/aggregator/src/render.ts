// Pure HTML rendering. Every view function takes already-assembled data and
// returns a full HTML string — nothing here does I/O, which keeps the smoke
// tests cheap (render from fixtures and assert on output).
import {
  allRefs,
  attentionSorted,
  commentStream,
  executionStrip,
  timeline as buildTimeline,
  windowPercent,
} from "./derive.ts";
import type {
  AttentionEntry,
  BoardItem,
  Comment,
  GhItem,
  Provenance,
  QuotaView,
  RefKey,
  RoadmapRow,
  SessionFeedEntry,
} from "./model.ts";

export function esc(s: unknown): string {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function fmtDate(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d
    .toISOString()
    .replace("T", " ")
    .replace(/\.\d+Z$/, "Z");
}

const CSS = `
:root { color-scheme: light dark; }
* { box-sizing: border-box; }
body { margin: 0; font: 14px/1.45 system-ui, -apple-system, sans-serif; background: #0e1117; color: #e6edf3; }
nav { display: flex; gap: 4px; padding: 10px 16px; border-bottom: 1px solid #30363d; background: #161b22; position: sticky; top: 0; }
nav a { color: #58a6ff; text-decoration: none; padding: 4px 10px; border-radius: 6px; }
nav a.active, nav a:hover { background: #21262d; }
main { max-width: 1180px; margin: 0 auto; padding: 20px 16px 80px; }
h1 { font-size: 18px; } h2 { font-size: 15px; }
a { color: #58a6ff; }
.card { border: 1px solid #30363d; border-radius: 8px; padding: 10px 12px; margin: 8px 0; background: #161b22; }
.muted { color: #8b949e; font-size: 12px; }
.badge { display: inline-block; font-size: 11px; padding: 1px 7px; border-radius: 999px; border: 1px solid #30363d; margin: 1px 2px 1px 0; white-space: nowrap; }
.badge.open { background: #1f6feb33; border-color: #1f6feb; color: #58a6ff; }
.badge.merged { background: #2ea04333; border-color: #2ea043; color: #3fb950; }
.badge.closed { background: #8b949e22; border-color: #8b949e; color: #8b949e; }
.badge.implementing { background: #d2992233; border-color: #d29922; color: #e3b341; }
.badge.review { background: #bc8cff33; border-color: #bc8cff; color: #bc8cff; }
.badge.healthy { background: #2ea04333; border-color: #2ea043; color: #3fb950; }
.badge.near { background: #d2992233; border-color: #d29922; color: #e3b341; }
.badge.walled { background: #f8514933; border-color: #f85149; color: #f85149; }
.strip { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 6px; }
.board { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 12px; align-items: start; }
.col { border: 1px solid #30363d; border-radius: 8px; padding: 8px; background: #0d1117; }
.col h2 { margin: 2px 0 8px; font-size: 13px; text-transform: uppercase; letter-spacing: .04em; color: #8b949e; }
table { border-collapse: collapse; width: 100%; }
th, td { border: 1px solid #30363d; padding: 6px 8px; vertical-align: top; text-align: left; }
th { background: #161b22; }
.tl-cell { min-width: 110px; }
.banner { border: 1px solid #d29922; background: #d2992211; border-radius: 8px; padding: 8px 12px; margin: 12px 0; color: #e3b341; }
.pool { display: grid; grid-template-columns: 220px 1fr; gap: 8px 16px; }
meter { width: 140px; height: 14px; }
.foot { margin-top: 40px; border-top: 1px solid #30363d; padding-top: 10px; color: #8b949e; font-size: 12px; }
`;

const NAV = [
  ["/", "Board"],
  ["/roadmap", "Roadmap"],
  ["/quota", "Quota"],
  ["/attention", "Needs you"],
] as const;

function layout(title: string, body: string, provenance: Provenance, active: string): string {
  const links = NAV.map(
    ([href, label]) => `<a href="${href}"${href === active ? ' class="active"' : ""}>${label}</a>`,
  ).join("");
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)} · aggregator</title><style>${CSS}</style></head>
<body><nav>${links}</nav><main>${body}
<div class="foot">board: ${esc(provenance.board)} · roadmap: ${esc(provenance.roadmap)} · items: ${esc(provenance.items)} · comments: ${esc(provenance.comments)} · quota: ${esc(provenance.quota)} · attention: ${esc(provenance.attention)}</div>
</main></body></html>`;
}

function itemBadge(item: GhItem): string {
  const cls = item.state === "merged" ? "merged" : item.state === "open" ? "open" : "closed";
  const kind = item.kind === "pr" ? (item.state === "merged" ? "PR merged" : "PR") : "issue";
  const draft = item.draft ? " · draft" : "";
  return `<a class="badge ${cls}" href="${esc(item.url)}" title="${esc(item.title)}">${kind} ${esc(item.ref)}${draft}</a>`;
}

function stripHtml(items: Map<RefKey, GhItem>, links: BoardItem["links"]): string {
  const strip = executionStrip(links, items);
  const buckets: [string, string, GhItem[]][] = [
    ["implementing", "implementing", strip.implementing],
    ["review", "review", strip.review],
    ["merged", "merged", strip.merged],
    ["open", "queued", strip.queued],
    ["closed", "closed", strip.closed],
  ];
  const parts: string[] = [];
  for (const [cls, label, list] of buckets) {
    if (list.length === 0) continue;
    parts.push(`<span class="badge ${cls}">${label}: ${list.length}</span>` + list.map(itemBadge).join(" "));
  }
  if (strip.unknown.length) {
    parts.push(`<span class="badge closed">unresolved: ${strip.unknown.map(esc).join(", ")}</span>`);
  }
  return `<div class="strip">${parts.join(" ")}</div>`;
}

// --- Board view -----------------------------------------------------------

export function renderBoard(board: BoardItem[], items: Map<RefKey, GhItem>, provenance: Provenance): string {
  const cols = new Map<string, BoardItem[]>();
  for (const item of board) {
    const key = item.status === "done" ? "done" : item.status;
    const k = key === "to-do" ? "to-do" : key;
    cols.set(k, [...(cols.get(k) ?? []), item]);
  }
  const order = ["to-do", "in-progress", "in-review", "done"];
  const extra = [...cols.keys()].filter((k) => !order.includes(k));
  const bodies = [...order, ...extra]
    .filter((k) => cols.has(k))
    .map((k) => {
      const cards = (cols.get(k) ?? [])
        .map((item) => {
          const repo = item.repo ? `<span class="muted">${esc(item.repo)}</span><br>` : "";
          const src = item.source === "github" ? `<span class="badge open">gh-only</span> ` : "";
          return `<div class="card"><a href="/item/${encodeURIComponent(item.id)}">${src}${esc(item.title)}</a><br>${repo}${stripHtml(items, item.links)}</div>`;
        })
        .join("");
      return `<div class="col"><h2>${esc(k)} (${(cols.get(k) ?? []).length})</h2>${cards}</div>`;
    })
    .join("");
  return layout(
    "Cross-repo WIP board",
    `<h1>Cross-repo WIP board</h1><p class="muted">Curated in-flight set across the four repos. Each card's execution strip is derived from its linked issues/PRs: implementing → review → merged.</p><div class="board">${bodies}</div>`,
    provenance,
    "/",
  );
}

// --- Roadmap view ----------------------------------------------------------

export function renderRoadmap(rows: RoadmapRow[], items: Map<RefKey, GhItem>, provenance: Provenance): string {
  const tl = buildTimeline(rows);
  const head = `<tr><th>Theme</th>${tl.lanes.map((l) => `<th>${esc(l.label)}</th>`).join("")}</tr>`;
  const body = tl.themes
    .map((theme) => {
      const cells = tl.lanes
        .map((lane) => {
          const rs = tl.cells.get(theme)?.get(lane.key) ?? [];
          const chips = rs
            .map((row) => {
              const refs = allRefs([row]);
              const refBadges = refs
                .map((r) => items.get(r))
                .filter((x): x is GhItem => Boolean(x))
                .map(itemBadge)
                .join(" ");
              return `<div class="card"><a href="/item/${encodeURIComponent(row.subject)}">${esc(row.title)}</a><br><span class="badge ${esc(row.status.toLowerCase().replace(/[^a-z]/g, "-"))}">${esc(row.status)}</span>${refBadges}</div>`;
            })
            .join("");
          return `<td class="tl-cell">${chips}</td>`;
        })
        .join("");
      return `<tr><th>${esc(theme)}</th>${cells}</tr>`;
    })
    .join("");
  return layout(
    "Roadmap",
    `<h1>Roadmap</h1><p class="muted">Grist's <em>Program roadmap</em> table as a timeline; each row drills down to its linked GitHub issues and their live state.</p><table>${head}${body}</table>`,
    provenance,
    "/roadmap",
  );
}

// --- Item drill-down -------------------------------------------------------

export function renderItem(
  title: string,
  subject: string,
  links: BoardItem["links"],
  notes: BoardItem["notes"],
  items: Map<RefKey, GhItem>,
  comments: Comment[],
  provenance: Provenance,
): string {
  const strip = executionStrip(links, items);
  const all = [...strip.implementing, ...strip.review, ...strip.merged, ...strip.queued, ...strip.closed];
  const refRows = all
    .map(
      (i) =>
        `<tr><td>${itemBadge(i)}</td><td>${esc(i.title)}</td><td>${esc(i.labels.join(", "))}</td><td>${fmtDate(i.updatedAt)}</td></tr>`,
    )
    .join("");
  const stream = commentStream(links, comments);
  const commentRows = stream
    .map(
      (c) =>
        `<tr><td class="muted">${fmtDate(c.createdAt)}</td><td><span class="badge ${esc(c.kind)}">${esc(c.kind)}</span></td><td>${esc(c.author)}</td><td>${esc(c.body)}${c.path ? ` <span class="muted">(${esc(c.path)}${c.line ? `:${c.line}` : ""})</span>` : ""}</td></tr>`,
    )
    .join("");
  const noteList = (notes ?? []).map((n) => `<li><a href="${esc(n.url)}">${esc(n.title)}</a></li>`).join("");
  return layout(
    title,
    `<h1>${esc(title)}</h1><p class="muted">subject: <code>${esc(subject)}</code></p>${stripHtml(items, links)}
<h2>Linked issues / PRs</h2><table><tr><th>Ref</th><th>Title</th><th>Labels</th><th>Updated</th></tr>${refRows || '<tr><td colspan="4" class="muted">none resolved</td></tr>'}</table>
${noteList ? `<h2>Planning notes</h2><ul>${noteList}</ul>` : ""}
<h2>Activity stream</h2><table><tr><th>When</th><th>Kind</th><th>Author</th><th>Body</th></tr>${commentRows || '<tr><td colspan="4" class="muted">no comments</td></tr>'}</table>`,
    provenance,
    "/",
  );
}

// --- Quota view (#851) -----------------------------------------------------

export function renderQuota(view: QuotaView, provenance: Provenance): string {
  const pools = view.pools
    .map((p) => {
      const windows = p.windows
        .map((w) => {
          const pct = windowPercent(w);
          const used = w.unit === "usd" ? `$${w.used.toFixed(2)}` : w.used.toLocaleString();
          const limit = w.limit == null ? "∞" : w.unit === "usd" ? `$${w.limit.toFixed(2)}` : w.limit.toLocaleString();
          return `<div>${esc(w.name)}: <meter min="0" max="100" value="${pct.toFixed(0)}"></meter> ${pct.toFixed(0)}% · ${used}/${limit} ${esc(w.unit)} · resets ${fmtDate(w.resetsAt)}</div>`;
        })
        .join("");
      const sessions = (p.activeSessions ?? [])
        .map(
          (s) =>
            `<div class="muted">burning: ${esc(s.session)} (${esc(s.model)}) ${s.tokens.toLocaleString()} tok</div>`,
        )
        .join("");
      return `<div class="card pool"><div><strong>${esc(p.provider)}</strong><br><span class="muted">${esc(p.plan)}</span><br><span class="badge ${esc(p.state)}">${esc(p.state)}${p.wallLiftsAt ? ` · lifts ${fmtDate(p.wallLiftsAt)}` : ""}</span>${p.burnTokensPerMin ? ` <span class="muted">${p.burnTokensPerMin} tok/min</span>` : ""}</div><div>${windows}${sessions}${p.note ? `<div class="muted">${esc(p.note)}</div>` : ""}</div></div>`;
    })
    .join("");
  const burn = view.burn
    .map(
      (b) =>
        `<tr><td>${esc(b.provider)}</td><td>${esc(b.pool)}</td><td>${fmtDate(b.from)}</td><td>${fmtDate(b.to)}</td><td>${b.tokens.toLocaleString()}</td></tr>`,
    )
    .join("");
  const choices = (list: string[]) => (list.length ? list.map(esc).join(", ") : "—");
  return layout(
    "Quota + throughput",
    `<h1>Quota + throughput</h1><p class="muted">All pooled quotas, resets, and burn rates. Machine-first: the same data is at <a href="/api/quota.json">/api/quota.json</a>.</p>
<h2>Pools</h2>${pools}
<h2>Burn between resets</h2><table><tr><th>Provider</th><th>Pool</th><th>From</th><th>To</th><th>Tokens</th></tr>${burn || '<tr><td colspan="5" class="muted">none</td></tr>'}</table>
<h2>Model choice</h2><table><tr><th>Rule</th><th>Pools</th></tr>
<tr><td>Headroom for a new session</td><td>${choices(view.choices.hasHeadroom)}</td></tr>
<tr><td>Burn-it-before-reset (unused headroom)</td><td>${choices(view.choices.burnBeforeReset)}</td></tr>
<tr><td>Do not start a second session</td><td>${choices(view.choices.doNotStart)}</td></tr></table>`,
    provenance,
    "/quota",
  );
}

// --- Attention view (#852) --------------------------------------------------

export function renderAttention(feed: SessionFeedEntry[], provenance: Provenance): string {
  const entries: AttentionEntry[] = attentionSorted(feed);
  const rows = entries
    .map((e) => {
      const extra =
        e.awaitingQuestion ??
        e.lastTurnError ??
        (e.repeatedErrorCount ? `${e.repeatedErrorCount} repeated errors` : null) ??
        (e.longRunningCall ? `CLI: ${e.longRunningCall.command ?? "?"}` : null);
      return `<tr><td><span class="badge ${esc(e.state)}">${esc(e.state)}</span></td><td><strong>${esc(e.title ?? e.sessionId)}</strong><br><span class="muted">${esc(e.sessionId)} · ${esc(e.directory ?? "—")} · ${esc(e.branch ?? "—")}</span></td><td>${esc(e.model ?? "—")} / ${esc(e.provider ?? "—")}</td><td>${fmtDate(e.lastActivityAt)}</td><td>${esc(e.messageCount ?? "—")}${e.messageDelta ? ` (+${e.messageDelta})` : ""}</td><td>${esc(e.evidence)}</td></tr>`;
    })
    .join("");
  return layout(
    "Needs you",
    `<h1>Needs you</h1><p class="muted">Every live agent session, classified and attention-ordered. Machine-first: <a href="/api/attention.json">/api/attention.json</a>.</p>
<table><tr><th>State</th><th>Session</th><th>Model / provider</th><th>Last activity</th><th>Msgs</th><th>Evidence</th></tr>${rows || '<tr><td colspan="6" class="muted">no sessions</td></tr>'}</table>`,
    provenance,
    "/attention",
  );
}
