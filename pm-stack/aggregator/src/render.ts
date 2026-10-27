// Pure HTML rendering. Every view function takes already-assembled data and
// returns a full HTML string — nothing here does I/O, which keeps the smoke
// tests cheap (render from fixtures and assert on output).
import {
  type AttentionView,
  type RoadmapGrain,
  type TodayView,
  allRefs,
  commentStream,
  executionStrip,
  freshness,
  monthTimeline,
  projectPace,
  resetMarkers,
  roadmapDayGrain,
  sessionChildren,
  sessionRoots,
  sourceOf,
  STATE_LABELS,
  timeline as buildTimeline,
  windowPercent,
} from "./derive.ts";
import { githubCommentUrl } from "./comments.ts";
import { summarizeDns } from "./dns.ts";
import {
  type BoardItem,
  type ChargeRow,
  type Comment,
  type DashAuthor,
  type DashComment,
  type DnsEntry,
  type DnsSection,
  type GhItem,
  type OrchestrationSection,
  type PaygLine,
  type ProjectPoolItem,
  type Provenance,
  type QuotaView,
  type RefKey,
  type RoadmapRow,
  type SaasService,
  type SessionRecord,
  type SnapshotRecord,
  type SourceCoverage,
  type TieredEntry,
  type TierItem,
  type UsageMetric,
  type UsageView,
  SAAS_GROUP_LABELS,
  SESSION_SOURCE_LABELS,
  countedEstimate,
  refUrl,
} from "./model.ts";
import { PAGES } from "./pages.ts";
import type { ProgramData, ProgramEntry } from "./program.ts";
import { derivedProjectState, projectDisagreement, PROJECT_URL } from "./projects.ts";
import { type AuthorIdentity } from "./registry.ts";

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
nav { display: flex; gap: 4px; padding: 10px 16px; border-bottom: 1px solid #30363d; background: #161b22; position: sticky; top: 0; overflow-x: auto; -webkit-overflow-scrolling: touch; }
nav a { color: #58a6ff; text-decoration: none; padding: 6px 10px; border-radius: 6px; white-space: nowrap; }
nav a.active, nav a:hover { background: #21262d; }
main { max-width: 1180px; margin: 0 auto; padding: 20px 16px 80px; }
h1 { font-size: 18px; } h2 { font-size: 15px; }
a { color: #58a6ff; }
.card { border: 1px solid #30363d; border-radius: 8px; padding: 10px 12px; margin: 8px 0; background: #161b22; overflow-wrap: anywhere; }
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
.badge.src { background: #21262d; color: #e6edf3; }
.badge.drew { background: #f8514933; border-color: #f85149; color: #f85149; }
.badge.orch { background: #1f6feb33; border-color: #1f6feb; color: #58a6ff; }
.badge.moving { background: #2ea04333; border-color: #2ea043; color: #3fb950; }
.badge.stalled { background: #d2992233; border-color: #d29922; color: #e3b341; }
.badge.blocked { background: #f8514933; border-color: #f85149; color: #f85149; }
.strip { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 6px; }
.board { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 12px; align-items: start; }
.col { border: 1px solid #30363d; border-radius: 8px; padding: 8px; background: #0d1117; }
.col h2 { margin: 2px 0 8px; font-size: 13px; text-transform: uppercase; letter-spacing: .04em; color: #8b949e; }
.tblwrap { overflow-x: auto; -webkit-overflow-scrolling: touch; }
table { border-collapse: collapse; width: 100%; }
th, td { border: 1px solid #30363d; padding: 6px 8px; vertical-align: top; text-align: left; }
th { background: #161b22; }
.tl-cell { min-width: 110px; }
.banner { border: 1px solid #d29922; background: #d2992211; border-radius: 8px; padding: 8px 12px; margin: 12px 0; color: #e3b341; }
.banner.drew { border-color: #f85149; background: #f8514911; color: #f85149; }
.pool { display: grid; grid-template-columns: 220px 1fr; gap: 8px 16px; }
meter { width: 140px; height: 14px; }
.chips { display: flex; flex-wrap: wrap; gap: 6px; margin: 10px 0; }
.chips a { border: 1px solid #30363d; border-radius: 999px; padding: 5px 12px; text-decoration: none; color: #e6edf3; background: #161b22; }
.chips a.active { border-color: #1f6feb; background: #1f6feb33; color: #58a6ff; }
.reason { border-left: 3px solid #f85149; padding: 6px 10px; margin: 8px 0 2px; background: #f851490d; }
.orch-note { border-left: 3px solid #1f6feb; padding: 6px 10px; margin: 8px 0 2px; background: #1f6feb0d; }
.cov { border: 1px solid #30363d; border-radius: 8px; padding: 8px 12px; margin: 12px 0; background: #0d1117; }
.cov li { margin: 3px 0; }
.kids { margin: 6px 0 2px 14px; border-left: 2px solid #30363d; padding-left: 10px; }
.kids summary { cursor: pointer; color: #58a6ff; font-size: 12px; padding: 4px 0; }
.obj { display: flex; gap: 10px; align-items: flex-start; }
.obj .n { font-size: 16px; font-weight: 700; color: #58a6ff; min-width: 22px; }
.badge.Muse { background: #bc8cff33; border-color: #bc8cff; color: #bc8cff; }
.badge.agent { background: #1f6feb33; border-color: #1f6feb; color: #58a6ff; }
.badge.msg { background: #d2992233; border-color: #d29922; color: #e3b341; }
.badge.gh { background: #2ea04333; border-color: #2ea043; color: #3fb950; }
.cmt { border-left: 3px solid #30363d; padding: 6px 10px; margin: 6px 0; background: #161b22; border-radius: 0 6px 6px 0; overflow-wrap: anywhere; }
.cmt.msg { border-left-color: #d29922; }
.cmt.github { border-left-color: #2ea043; }
.cform { border: 1px solid #30363d; border-radius: 8px; padding: 10px 12px; margin: 10px 0; background: #0d1117; }
.cform label { display: block; font-size: 12px; color: #8b949e; margin: 8px 0 3px; }
.cform textarea, .cform select, .cform input[type="text"] { width: 100%; background: #161b22; color: #e6edf3; border: 1px solid #30363d; border-radius: 6px; padding: 7px 9px; font: inherit; }
.cform .kinds { display: flex; flex-wrap: wrap; gap: 12px; margin-top: 4px; }
.cform .kinds label { display: inline-flex; gap: 5px; align-items: center; margin: 0; font-size: 13px; color: #e6edf3; }
.cform button { margin-top: 10px; background: #1f6feb; color: #fff; border: 0; border-radius: 6px; padding: 8px 16px; font: inherit; cursor: pointer; }
.cform .err { color: #f85149; font-size: 12px; margin-top: 6px; }
nav select { background: #161b22; color: #e6edf3; border: 1px solid #30363d; border-radius: 6px; padding: 6px 8px; font: inherit; margin-left: auto; max-width: 45vw; }
.foot { margin-top: 40px; border-top: 1px solid #30363d; padding-top: 10px; color: #8b949e; font-size: 12px; }
.card.stale { opacity: .55; }
.badge.stale { background: #d2992233; border-color: #d29922; color: #e3b341; }
.badge.conflict { background: #bc8cff33; border-color: #bc8cff; color: #bc8cff; }
.flag { border: 1px solid #f85149; background: #f8514911; border-radius: 8px; padding: 8px 12px; margin: 8px 0; }
.flag.payg { border-color: #d29922; background: #d2992211; }
.svcgrid { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 12px; align-items: start; }
.chart { background: #0d1117; border: 1px solid #30363d; border-radius: 8px; padding: 8px; margin: 8px 0; overflow-x: auto; }
@media (max-width: 720px) {
  body { font-size: 13.5px; }
  main { padding: 12px 10px 60px; }
  nav { padding: 8px 10px; }
  h1 { font-size: 16px; }
  .board { grid-template-columns: 1fr; }
  .pool { grid-template-columns: 1fr; }
  th, td { padding: 5px 6px; }
  meter { width: 96px; }
  .card { padding: 8px 10px; }
}
`;

/** Canonical tailnet URL for this dashboard (MagicDNS — never the raw IP). */
export const TAILNET_URL = "http://amd-6.raptor-pumpkinseed.ts.net:4870";

/**
 * The client side of the comment form (one delegated submit handler;
 * pages without a form never notice it). Kept dependency-free and
 * inline, like everything else in this app.
 */
const COMMENT_SCRIPT = `<script>
document.addEventListener("submit", async (ev) => {
  const form = ev.target;
  if (!(form instanceof HTMLFormElement) || !form.classList.contains("cform")) return;
  ev.preventDefault();
  const err = form.querySelector(".err");
  const data = new FormData(form);
  const kind = String(data.get("kind") || "note");
  const [akind, aid, asession] = String(data.get("author") || "drew|drew|").split("|");
  const labelEl = form.querySelector('option[value="' + CSS.escape(String(data.get("author"))) + '"]');
  const payload = {
    objectRef: form.dataset.object,
    kind,
    author: { kind: akind, id: aid, label: labelEl ? labelEl.textContent : aid, ...(asession ? { sessionId: asession } : {}) },
    body: String(data.get("body") || ""),
    ...(kind === "github" ? { githubRef: String(data.get("githubRef") || "") } : {}),
  };
  try {
    const res = await fetch("/api/comments", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(payload) });
    if (!res.ok) { const b = await res.json().catch(() => ({})); throw new Error(b.error || ("HTTP " + res.status)); }
    location.reload();
  } catch (e) { if (err) err.textContent = "Not posted: " + e.message; }
});
document.addEventListener("change", (ev) => {
  const t = ev.target;
  if (!(t instanceof HTMLInputElement) || t.name !== "kind") return;
  const form = t.closest(".cform");
  const wrap = form && form.querySelector(".ghtarget");
  if (wrap) wrap.style.display = t.value === "github" && t.checked ? "" : "none";
});
</script>`;

function layout(title: string, body: string, provenance: Provenance, active: string, extra = ""): string {
  // Nav + the page dropdown both render from the manifest (#906 §1):
  // registering a page in src/pages.ts is the only step that puts it
  // in either.
  const links = PAGES.filter((p) => p.id !== "pages")
    .map((p) => `<a href="${p.route}"${p.route === active ? ' class="active"' : ""}>${esc(p.title)}</a>`)
    .join("");
  const options = PAGES.map(
    (p) =>
      `<option value="${p.route}"${p.route === active ? " selected" : ""}>${esc(p.title)} — ${esc(p.blurb)}</option>`,
  ).join("");
  const dropdown = `<select aria-label="Go to page" onchange="if(this.value)location.href=this.value"><option value="">Pages ▾</option>${options}</select>`;
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)} · aggregator</title><style>${CSS}</style></head>
<body><nav>${links}${dropdown}</nav><main>${body}
<div class="foot">board: ${esc(provenance.board)} · roadmap: ${esc(provenance.roadmap)} · items: ${esc(provenance.items)} · comments: ${esc(provenance.comments)} · quota: ${esc(provenance.quota)} · usage: ${esc(provenance.usage)} · attention: ${esc(provenance.attention)} · orchestration: ${esc(provenance.orchestration)} · dns: ${esc(provenance.dns)} · registry: ${esc(provenance.registry)}<br>tailnet: <a href="${TAILNET_URL}">${TAILNET_URL.replace("http://", "")}</a></div>
</main>${extra}</body></html>`;
}

// --- Dashboard comments (#906 §2) ------------------------------------------

/** Object refs: the keys dashboard comment threads hang on. */
export const sessionObjectRef = (id: string): string => `session:${id}`;
export const ghObjectRef = (ref: RefKey): string => `gh:${ref}`;
export function tierItemObjectRef(item: TierItem): string {
  return item.ref ? ghObjectRef(item.ref) : `orch:${item.id}`;
}

/** A 💬 link to an object's comment thread, with its comment count. */
export function commentBadge(objectRef: string, counts?: Map<string, number>): string {
  const n = counts?.get(objectRef) ?? 0;
  return ` <a class="badge" href="/discuss/${encodeURIComponent(objectRef)}" title="Dashboard comments on this item">💬${n ? ` ${n}` : ""}</a>`;
}

export function authorBadge(a: DashAuthor): string {
  const cls = a.kind === "drew" ? "drew" : a.kind === "muse" ? "Muse" : "agent";
  const link = a.sessionId ? ` <a href="/sessions#${encodeURIComponent(a.sessionId)}" class="muted">session</a>` : "";
  return `<span class="badge ${cls}">${esc(a.label)}</span>${link}`;
}

export function dashCommentHtml(c: DashComment, showObject = false): string {
  const kindChip =
    c.kind === "message"
      ? `<span class="badge msg">✉ message to Muse</span> <span class="muted">${c.triageState === "triaged" ? "triaged by the orchestrator" : "awaiting triage"}</span>`
      : c.kind === "github"
        ? `<span class="badge gh">GitHub comment</span> <span class="muted">${
            c.githubState === "posted"
              ? `posted${c.githubUrl ? ` — <a href="${esc(c.githubUrl)}">view</a>` : ""}`
              : c.githubState === "failed"
                ? "posting failed — a fleet poster will retry"
                : `pending — will post to ${esc(c.githubRef ?? "")}`
          }</span>`
        : `<span class="badge">note</span>`;
  const obj = showObject ? ` <span class="muted">on <code>${esc(c.objectRef)}</code></span>` : "";
  return `<div class="cmt ${esc(c.kind)}">${authorBadge(c.author)} ${kindChip}${obj}<br>${esc(c.body)}<br><span class="muted">${fmtDate(c.createdAt)}</span></div>`;
}

export interface CommentFormOpts {
  /** Refs the "post as GitHub comment" kind may target (explicit select). */
  githubRefs?: RefKey[];
  /** Author identities for the picker (standing + registry sessions). */
  identities: AuthorIdentity[];
}

export function commentFormHtml(objectRef: string, opts: CommentFormOpts): string {
  const ghRefs = opts.githubRefs ?? [];
  const authorOptions = opts.identities
    .map(
      (i) =>
        `<option value="${esc(`${i.kind}|${i.id}|${i.sessionId ?? ""}`)}"${i.id === "drew" ? " selected" : ""}>${esc(i.label)}${i.kind === "agent" ? " (agent)" : ""}</option>`,
    )
    .join("");
  const ghTarget = ghRefs.length
    ? `<div class="ghtarget" style="display:none"><label>Post to GitHub issue / PR (explicit target)</label><select name="githubRef">${ghRefs.map((r) => `<option value="${esc(r)}">${esc(r)}</option>`).join("")}</select></div>`
    : "";
  const ghKind = ghRefs.length
    ? `<label><input type="radio" name="kind" value="github"> Post as GitHub comment</label>`
    : "";
  return `<form class="cform" data-object="${esc(objectRef)}">
<label>Comment as</label><select name="author">${authorOptions}</select>
<label>Kind</label><div class="kinds">
<label><input type="radio" name="kind" value="note" checked> Note — read when this is next picked up</label>
<label><input type="radio" name="kind" value="message"> ✉ Message to Muse — the orchestrator triages it on its next pass</label>
${ghKind}</div>
${ghTarget}
<label>Comment</label><textarea name="body" rows="3" required placeholder="Write a comment…"></textarea>
<button type="submit">Post comment</button><div class="err"></div></form>`;
}

/** A full thread + form for one object ref (discuss page, item page). */
export function commentThreadHtml(
  objectRef: string,
  comments: DashComment[],
  opts: CommentFormOpts,
  heading = "Dashboard comments",
): string {
  const list = comments.length
    ? comments.map((c) => dashCommentHtml(c)).join("")
    : `<p class="muted">No dashboard comments yet.</p>`;
  return `<h2>${esc(heading)}</h2>${list}${commentFormHtml(objectRef, opts)}`;
}

export function coverageHtml(coverage: SourceCoverage[]): string {
  const rows = coverage
    .map(
      (c) =>
        `<li>${c.wired ? "✓" : "⚠"} <strong>${esc(c.label)}</strong> — ${c.wired ? "wired" : "<strong>not wired</strong>"} · <span class="muted">${esc(c.detail)}</span></li>`,
    )
    .join("");
  return `<div class="cov"><strong>Session coverage</strong><ul>${rows}</ul></div>`;
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

export function renderBoard(
  board: BoardItem[],
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  counts?: Map<string, number>,
): string {
  const cols = new Map<string, BoardItem[]>();
  for (const item of board) {
    const key = item.status === "done" ? "done" : item.status;
    const k = key === "to-do" ? "to-do" : key;
    cols.set(k, [...(cols.get(k) ?? []), item]);
  }
  const order = ["in-flight", "to-do", "in-progress", "in-review", "done"];
  const extra = [...cols.keys()].filter((k) => !order.includes(k));
  const bodies = [...order, ...extra]
    .filter((k) => cols.has(k))
    .map((k) => {
      const colItems = cols.get(k) ?? [];
      const cards = colItems
        .map((item) => {
          const repo = item.repo ? `<span class="muted">${esc(item.repo)}</span><br>` : "";
          const cmt = counts ? commentBadge(item.id, counts) : "";
          if (item.source === "orchestrator") {
            // Synthetic in-flight entry: visibly not a Kaneo card. Its
            // GitHub refs link directly (no execution strip — the
            // projector may not know these refs yet).
            const status = item.orchStatus ? `<span class="badge src">${esc(item.orchStatus)}</span> ` : "";
            const goal = item.goal ? `<br><span class="muted">goal: ${esc(item.goal)}</span>` : "";
            const blocked = item.blockedBy?.length
              ? `<br><strong>Blocked by:</strong> ${item.blockedBy.map(esc).join("; ")}`
              : "";
            const refs = item.links.length
              ? `<div class="strip">${item.links.map((l) => `<a class="badge open" href="${esc(refUrl(l.ref))}">${esc(l.ref)}</a>`).join(" ")}</div>`
              : "";
            return `<div class="card"><a href="/item/${encodeURIComponent(item.id)}"><span class="badge implementing">orchestrator · in flight</span> ${esc(item.title)}</a>${cmt}<br>${status}${repo}${goal}${blocked}${refs}</div>`;
          }
          const src = item.source === "github" ? `<span class="badge open">gh-only</span> ` : "";
          return `<div class="card"><a href="/item/${encodeURIComponent(item.id)}">${src}${esc(item.title)}</a>${cmt}<br>${repo}${stripHtml(items, item.links)}</div>`;
        })
        .join("");
      // The in-flight lane is dated by the orchestration section that
      // produced it (stamped on every synthetic entry as updatedAt).
      const heading =
        k === "in-flight"
          ? `in flight — orchestrator · in flight · section ${fmtDate(colItems[0]?.updatedAt)} (${colItems.length})`
          : `${esc(k)} (${colItems.length})`;
      return `<div class="col"><h2>${heading}</h2>${cards}</div>`;
    })
    .join("");
  return layout(
    "Cross-repo WIP board",
    `<h1>Cross-repo WIP board</h1><p class="muted">What is being worked on now is the <strong>in-flight</strong> lane: synthetic entries from the orchestrator's lined-up tasks (the fleet feed's orchestration section), not Kaneo cards. The other columns are the curated Kaneo board across the four repos, including its completion history — on their own they are a record of finished and in-review work, not a live WIP list. Each Kaneo card's execution strip is derived from its linked issues/PRs: implementing → review → merged.</p><div class="board">${bodies}</div>`,
    provenance,
    "/",
  );
}

// --- Roadmap view ----------------------------------------------------------

const GRAINS: [RoadmapGrain, string][] = [
  ["quarter", "Quarter"],
  ["month", "Month"],
  ["week", "Week"],
  ["3d", "Next 3 days"],
  ["today", "Today"],
];

function roadmapCard(row: RoadmapRow, items: Map<RefKey, GhItem>, counts?: Map<string, number>): string {
  const refs = allRefs([row]);
  const refBadges = refs
    .map((r) => items.get(r))
    .filter((x): x is GhItem => Boolean(x))
    .map(itemBadge)
    .join(" ");
  const cmt = counts ? commentBadge(row.subject, counts) : "";
  return `<div class="card"><a href="/item/${encodeURIComponent(row.subject)}">${esc(row.title)}</a>${cmt}<br><span class="badge ${esc(row.status.toLowerCase().replace(/[^a-z]/g, "-"))}">${esc(row.status)}</span> <span class="muted">${esc(row.area)} · ${esc(row.horizon)}</span>${refBadges}</div>`;
}

function timelineTable(
  tl: ReturnType<typeof buildTimeline>,
  items: Map<RefKey, GhItem>,
  counts?: Map<string, number>,
): string {
  const head = `<tr><th>Theme</th>${tl.lanes.map((l) => `<th>${esc(l.label)}</th>`).join("")}</tr>`;
  const body = tl.themes
    .map((theme) => {
      const cells = tl.lanes
        .map((lane) => {
          const rs = tl.cells.get(theme)?.get(lane.key) ?? [];
          return `<td class="tl-cell">${rs.map((r) => roadmapCard(r, items, counts)).join("")}</td>`;
        })
        .join("");
      return `<tr><th>${esc(theme)}</th>${cells}</tr>`;
    })
    .join("");
  return `<div class="tblwrap"><table>${head}${body}</table></div>`;
}

export function renderRoadmap(
  rows: RoadmapRow[],
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  grain: RoadmapGrain = "quarter",
  counts?: Map<string, number>,
): string {
  const chips = GRAINS.map(
    ([g, label]) => `<a href="/roadmap?h=${g}"${g === grain ? ' class="active"' : ""}>${label}</a>`,
  ).join("");
  let body: string;
  if (grain === "quarter") {
    body = timelineTable(buildTimeline(rows), items, counts);
  } else if (grain === "month") {
    body = timelineTable(monthTimeline(rows), items, counts);
  } else {
    const dg = roadmapDayGrain(rows, grain);
    const sections = dg.lanes
      .map(
        (lane) =>
          `<h2>${esc(lane.label)}</h2>${lane.rows.length ? lane.rows.map((r) => roadmapCard(r, items, counts)).join("") : '<p class="muted">Nothing dated this day.</p>'}`,
      )
      .join("");
    const notes: string[] = [];
    if (dg.outside > 0) notes.push(`${dg.outside} dated row${dg.outside === 1 ? "" : "s"} fall outside this window`);
    if (dg.coarse.length > 0) {
      notes.push(
        `${dg.coarse.length} row${dg.coarse.length === 1 ? "" : "s"} have no calendar date (quarter grain, ongoing, or unscheduled) — see the Quarter view`,
      );
    }
    body = `${sections}${notes.length ? `<p class="muted">${notes.join(" · ")}</p>` : ""}`;
  }
  return layout(
    "Roadmap",
    `<h1>Roadmap</h1><p class="muted">Grist's <em>Program roadmap</em> table at every grain — one data model, switchable horizon. Rows with a calendar date land on their day; quarter-labelled rows stay at quarter grain.</p><div class="chips">${chips}</div>${body}`,
    provenance,
    "/roadmap",
  );
}

// --- Item drill-down -------------------------------------------------------

export interface ItemExtras {
  /** Dashboard comments (#906 §2) across this item's object refs. */
  dashComments: DashComment[];
  /** The object ref the inline form posts to (the item's subject). */
  formRef: string;
  formOpts: CommentFormOpts;
  /** Registry sessions loosely linked to this item's refs (#906 §3). */
  sessions: SessionRecord[];
}

export function renderItem(
  title: string,
  subject: string,
  links: BoardItem["links"],
  notes: BoardItem["notes"],
  items: Map<RefKey, GhItem>,
  comments: Comment[],
  provenance: Provenance,
  extras?: ItemExtras,
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
  const dashSection = extras
    ? `<h2>Dashboard comments</h2>${
        extras.dashComments.length
          ? extras.dashComments.map((c) => dashCommentHtml(c, true)).join("")
          : `<p class="muted">No dashboard comments yet — notes are read when this is picked up; messages are triaged by the orchestrator.</p>`
      }${commentFormHtml(extras.formRef, extras.formOpts)}`
    : "";
  const sessionSection =
    extras && extras.sessions.length
      ? `<h2>Sessions that worked this</h2>${extras.sessions.map((r) => sessionRecordLine(r)).join("")}`
      : "";
  return layout(
    title,
    `<h1>${esc(title)}</h1><p class="muted">subject: <code>${esc(subject)}</code></p>${stripHtml(items, links)}
<h2>Linked issues / PRs</h2><div class="tblwrap"><table><tr><th>Ref</th><th>Title</th><th>Labels</th><th>Updated</th></tr>${refRows || '<tr><td colspan="4" class="muted">none resolved</td></tr>'}</table></div>
${noteList ? `<h2>Planning notes</h2><ul>${noteList}</ul>` : ""}
<h2>Activity stream</h2><div class="tblwrap"><table><tr><th>When</th><th>Kind</th><th>Author</th><th>Body</th></tr>${commentRows || '<tr><td colspan="4" class="muted">no comments</td></tr>'}</table></div>
${sessionSection}${dashSection}`,
    provenance,
    "/",
    extras ? COMMENT_SCRIPT : "",
  );
}

/** One registry record as a compact line (item page, sessions page). */
export function sessionRecordLine(r: SessionRecord): string {
  const tokens = r.tokensTotal != null ? ` · ${r.tokensTotal.toLocaleString()} tok` : "";
  const parent = r.parentId
    ? ` · sub of <a href="/sessions#${encodeURIComponent(r.parentId)}">${esc(r.parentId)}</a>`
    : "";
  const refs = r.taskRefs.length
    ? ` · ${r.taskRefs.map((ref) => `<a href="${esc(refUrl(ref))}">${esc(ref)}</a>`).join(" ")}`
    : "";
  return `<div class="card" id="${esc(r.id)}"><strong>${esc(r.label)}</strong> <span class="badge src">${esc(r.source)}</span> <span class="badge agent">${esc(r.agentLabel)}</span><br><span class="muted">${esc(r.id)} · ${esc(r.status)}${r.model ? ` · ${esc(r.model)}` : ""} · started ${fmtDate(r.startedAt)} · last seen ${fmtDate(r.lastSeenAt)}${tokens}${parent}${refs}</span></div>`;
}

// --- Quota view (#851) + Usage & billing hub (design 2026-10-25) -----------

const SOURCE_CLASS_LABELS: Record<string, string> = {
  api: "API read",
  console: "console read",
  manual: "manual read",
  estimate: "estimate",
  invoice: "invoice",
};

export function fmtMetricValue(v: number | null, unit: string): string {
  if (v == null) return "not measured";
  if (unit === "usd") return `$${v.toFixed(2)}`;
  if (unit === "gb") return `${v} GB`;
  if (unit === "pct") return `${v}%`;
  return v.toLocaleString();
}

/** The freshness stamp every hub card carries (§1.1): measured-at + source + age. */
export function freshnessStamp(measuredAt: string | null, sourceClass: string, cadenceDays: number): string {
  const f = freshness(measuredAt, cadenceDays);
  const cls = SOURCE_CLASS_LABELS[sourceClass] ?? sourceClass;
  if (!measuredAt)
    return `<span class="badge stale">no dated read</span> <span class="muted">${esc(cls)} · ${esc("never measured")}</span>`;
  const age = f.ageDays == null ? "" : f.ageDays === 0 ? " · today" : ` · ${f.ageDays}d ago`;
  const stale = f.stale ? ` <span class="badge stale">stale — as of ${esc(measuredAt.slice(0, 10))}</span>` : "";
  return `<span class="muted">measured ${esc(measuredAt.slice(0, 10))}${age} · ${esc(cls)}</span>${stale}`;
}

function metricRow(m: UsageMetric): string {
  const pct = m.percent;
  const meter =
    pct != null
      ? `<meter min="0" max="100" value="${Math.min(100, pct).toFixed(0)}"></meter> ${pct.toFixed(0)}% · `
      : "";
  const limit = m.limit == null ? "" : ` / ${fmtMetricValue(m.limit, m.unit)}`;
  return `<div>${esc(m.label)}: ${meter}${fmtMetricValue(m.used, m.unit)}${limit}${m.note ? ` <span class="muted">— ${esc(m.note)}</span>` : ""}</div>`;
}

function attentionStripHtml(usage: UsageView): string {
  if (!usage.attention.length) {
    return `<div class="card"><strong>Needs attention</strong> — nothing over threshold. <span class="muted">(Bubble rules: any metric over 80% of its limit, or projected unbilled PAYG over $5.)</span></div>`;
  }
  const rows = usage.attention
    .map(
      (f) =>
        `<div class="flag${f.kind === "payg-unbilled" ? " payg" : ""}"><strong>${esc(f.label)}</strong> — ${esc(f.detail)} <a href="${esc(f.href)}">details</a></div>`,
    )
    .join("");
  return `<h2>Needs attention</h2>${rows}`;
}

function paygCardHtml(usage: UsageView): string {
  if (!usage.payg.length) {
    return `<div class="card"><div><strong>PAYG dollars</strong><br><span class="muted">pay-as-you-go agent spend</span></div><div class="muted">No metered spill recorded in local records.</div></div>`;
  }
  const lines = usage.payg
    .map((l: PaygLine) => {
      const figures =
        l.accruedUsd != null
          ? `$${l.accruedUsd.toFixed(2)} accrued${l.collectedUsd != null ? ` · $${l.collectedUsd.toFixed(2)} collected` : ""} · $${(l.unbilledUsd ?? 0).toFixed(2)} unbilled`
          : l.estimate
            ? `counted $${countedEstimate(l.estimate).toFixed(2)} as used (1.2 × high) · estimate $${l.estimate.low.toFixed(2)}–$${l.estimate.high.toFixed(2)} ${esc(l.estimate.period)}`
            : "not estimable from local records";
      return `<div><strong>${esc(l.label)}</strong>: ${figures}<br><span class="muted">${esc(l.note)}</span><br>${freshnessStamp(l.measuredAt, l.sourceClass, 7)}</div>`;
    })
    .join("");
  const total = usage.payg.reduce((sum, l) => sum + (l.unbilledUsd ?? 0), 0);
  return `<div class="card"><div><strong>PAYG dollars</strong><br><span class="muted">metered spill, counted per Decision 004 — estimates at 1.2 × high</span><br><span class="badge ${total > 5 ? "near" : "healthy"}">$${total.toFixed(2)} projected unbilled</span></div><div>${lines}</div></div>`;
}

function serviceCardHtml(svc: SaasService, history: SnapshotRecord[]): string {
  const f = freshness(svc.measuredAt, svc.cadenceDays);
  const highlights = svc.metrics.filter((m) => m.highlight);
  const metrics = highlights.length
    ? highlights.map(metricRow).join("")
    : `<div class="muted">${esc(svc.usageNote ?? "No highlight metrics recorded.")}</div>`;
  const billed = svc.billedUsd != null ? `<div>billed this period: $${svc.billedUsd.toFixed(2)}</div>` : "";
  const payg = svc.paygEstimate
    ? `<div>PAYG estimate: counted $${countedEstimate(svc.paygEstimate).toFixed(2)} (${esc(svc.paygEstimate.period)}; range $${svc.paygEstimate.low.toFixed(2)}–$${svc.paygEstimate.high.toFixed(2)} in detail)</div>`
    : svc.paygNote
      ? `<div class="muted">PAYG: ${esc(svc.paygNote)}</div>`
      : "";
  const conflict = svc.conflicts.length
    ? ` <a class="badge conflict" href="/quota/service/${esc(svc.id)}" title="Sources disagree — the dated history is on the detail page">conflict</a>`
    : "";
  const pace = paceLineHtml(svc.id, history);
  return `<div class="card${f.stale ? " stale" : ""}"><div><strong><a href="/quota/service/${esc(svc.id)}">${esc(svc.name)}</a></strong>${conflict}<br><span class="muted">${esc(svc.plan)}</span><br>${freshnessStamp(svc.measuredAt, svc.sourceClass, svc.cadenceDays)}${svc.nextDate ? `<br><span class="muted">next: ${esc(svc.nextDate)}</span>` : ""}</div><div>${metrics}${billed}${payg}${pace}<div><a href="/quota/service/${esc(svc.id)}">show all →</a></div></div></div>`;
}

/** A one-line pace projection on a card, when the history supports one (§1.5). */
function paceLineHtml(serviceId: string, history: SnapshotRecord[]): string {
  const metricIds = [...new Set(history.filter((r) => r.unit === "pct").map((r) => r.metricId))];
  for (const metricId of metricIds) {
    const pace = projectPace(history, metricId);
    if (pace?.projectedExhaustionAt) {
      return `<div class="muted">pace (${esc(metricId)}): +${pace.slopePerDay} pts/day → projected to exhaust ${esc(pace.projectedExhaustionAt.slice(0, 10))}${pace.projectedPercentAtReset != null ? ` · ${pace.projectedPercentAtReset}% at reset` : ""}</div>`;
    }
  }
  return "";
}

function chargesStripHtml(usage: UsageView): string {
  if (!usage.charges.length) {
    return `<h2>Charges this period</h2><div class="card"><span class="muted">${esc(usage.chargesNote ?? "No charge evidence in the local records yet — rows appear as Plaid/Gmail receipt evidence is recorded.")}</span></div>`;
  }
  const rows = usage.charges
    .map(
      (c: ChargeRow) =>
        `<tr><td>${esc(c.at)}</td><td>${esc(c.label)}</td><td>$${c.amountUsd.toFixed(2)}</td><td>${esc(c.method)}</td><td class="muted">${esc(c.source)}</td></tr>`,
    )
    .join("");
  const total = usage.charges.reduce((sum, c) => sum + c.amountUsd, 0);
  return `<h2>Charges this period</h2><p class="muted">Evidence, not accounting — charges observed on cards/receipts in the local records (full balances stay in the finance tooling). Total observed: $${total.toFixed(2)}.</p><div class="tblwrap"><table><tr><th>Date</th><th>Charge</th><th>Amount</th><th>Method</th><th>Evidence</th></tr>${rows}</table></div>`;
}

export function renderQuota(view: QuotaView, provenance: Provenance, usage?: UsageView): string {
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
  const burnAndChoices = `<h2>Burn between resets</h2><div class="tblwrap"><table><tr><th>Provider</th><th>Pool</th><th>From</th><th>To</th><th>Tokens</th></tr>${burn || '<tr><td colspan="5" class="muted">none</td></tr>'}</table></div>
<h2>Model choice</h2><div class="tblwrap"><table><tr><th>Rule</th><th>Pools</th></tr>
<tr><td>Headroom for a new session</td><td>${choices(view.choices.hasHeadroom)}</td></tr>
<tr><td>Burn-it-before-reset (unused headroom)</td><td>${choices(view.choices.burnBeforeReset)}</td></tr>
<tr><td>Do not start a second session</td><td>${choices(view.choices.doNotStart)}</td></tr></table></div>`;
  if (!usage) {
    return layout(
      "Quota + throughput",
      `<h1>Quota + throughput</h1><p class="muted">All pooled quotas, resets, and burn rates. Machine-first: the same data is at <a href="/api/quota.json">/api/quota.json</a>.</p>
<h2>Pools</h2>${pools}
${burnAndChoices}`,
      provenance,
      "/quota",
    );
  }
  const groupOrder: (keyof typeof SAAS_GROUP_LABELS)[] = [
    "database",
    "object-storage",
    "compute",
    "dev-platform",
    "domains",
    "account-tooling",
  ];
  const groups = groupOrder
    .map((g) => {
      const svcs = usage.services.filter((s) => s.group === g);
      const cards = svcs.map((s) => serviceCardHtml(s, usage.history[s.id] ?? [])).join("");
      const domainsCard =
        g === "domains"
          ? usage.domains
            ? `<div class="card"><div><strong><a href="/dns">Domains</a></strong><br><span class="muted">Porkbun register — summary only; the full register lives on the DNS page</span></div><div><div>renewal exposure: $${usage.domains.totalAnnualUsd.toFixed(2)}/yr across ${usage.domains.domainCount} domains</div><div>next renewal: ${esc(usage.domains.nextRenewal ?? "—")}</div><div><a href="/dns">the full register →</a></div></div></div>`
            : `<div class="card"><span class="muted">Domains: the register has not reached the feed yet — see <a href="/dns">/dns</a>.</span></div>`
          : "";
      const subs =
        g === "account-tooling" && usage.subscriptions.length
          ? `<div class="card"><div><strong>Subscriptions (flat)</strong><br><span class="muted">renewal-only rows — no usage meter</span></div><div>${usage.subscriptions
              .map(
                (s) =>
                  `<div>${esc(s.name)}: ${esc(s.plan)}${s.renewsAt ? ` · renews ${esc(s.renewsAt)}` : " · renewal date unknown"}</div>`,
              )
              .join("")}</div></div>`
          : "";
      if (!cards && !domainsCard && !subs) return "";
      return `<h3>${esc(SAAS_GROUP_LABELS[g])}</h3><div class="svcgrid">${cards}${domainsCard}${subs}</div>`;
    })
    .join("");
  return layout(
    "Usage & billing",
    `<h1>Usage &amp; billing</h1><p class="muted">Token quotas, PAYG dollars, and SaaS usage vs limits — from dated local records; every card carries its measured-at stamp, and stale cards dim. Machine-first: <a href="/api/quota.json">/api/quota.json</a>.</p>
${attentionStripHtml(usage)}
<h2>Token quotas</h2>${pools}
${paygCardHtml(usage)}
<h2>SaaS &amp; services</h2>${groups || '<p class="muted">No services extracted yet.</p>'}
${chargesStripHtml(usage)}
${burnAndChoices}`,
    provenance,
    "/quota",
  );
}

// --- Service detail view (hub v1.1, §1.5) -------------------------------------

/** An inline SVG history chart for one metric, with reset markers (§4). */
export function historyChartHtml(records: SnapshotRecord[], metricId: string): string {
  const recs = records
    .filter((r) => r.metricId === metricId && typeof r.value === "number")
    .sort((a, b) => a.ts.localeCompare(b.ts)) as (SnapshotRecord & { value: number })[];
  if (!recs.length) return "";
  const W = 640;
  const H = 130;
  const PAD = 28;
  const t0 = Date.parse(recs[0].ts);
  const t1 = Math.max(Date.parse(recs[recs.length - 1].ts), t0 + 1);
  const vmax = Math.max(...recs.map((r) => r.value), ...(recs.map((r) => r.limit ?? 0) as number[]), 1) * 1.1;
  const x = (ts: string) => PAD + ((Date.parse(ts) - t0) / (t1 - t0)) * (W - PAD - 8);
  const y = (v: number) => H - 18 - (v / vmax) * (H - 34);
  const markers = resetMarkers(records, metricId)
    .filter((m) => Date.parse(m.ts) >= t0 && Date.parse(m.ts) <= t1)
    .map(
      (m) =>
        `<line x1="${x(m.ts).toFixed(1)}" y1="8" x2="${x(m.ts).toFixed(1)}" y2="${H - 18}" stroke="#d29922" stroke-dasharray="3 3"><title>reset ${esc(m.resetsAt.slice(0, 10))}</title></line>`,
    )
    .join("");
  const limitLine = recs.find((r) => r.limit != null)
    ? `<line x1="${PAD}" y1="${y((recs.find((r) => r.limit != null) as SnapshotRecord).limit as number).toFixed(1)}" x2="${W - 8}" y2="${y((recs.find((r) => r.limit != null) as SnapshotRecord).limit as number).toFixed(1)}" stroke="#f85149" stroke-dasharray="2 4"></line>`
    : "";
  const points = recs
    .map(
      (r) =>
        `<circle cx="${x(r.ts).toFixed(1)}" cy="${y(r.value).toFixed(1)}" r="3" fill="#58a6ff"><title>${esc(r.ts.slice(0, 10))}: ${r.value} ${esc(r.unit)} (${esc(r.source)})</title></circle>`,
    )
    .join("");
  const line = recs.map((r, i) => `${i === 0 ? "M" : "L"}${x(r.ts).toFixed(1)},${y(r.value).toFixed(1)}`).join(" ");
  const first = recs[0];
  const last = recs[recs.length - 1];
  return `<div class="chart"><div class="muted">${esc(metricId)} · ${recs.length} snapshot${recs.length === 1 ? "" : "s"} · ${esc(first.ts.slice(0, 10))} → ${esc(last.ts.slice(0, 10))} · dashed amber = window reset, red = limit</div><svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="history chart for ${esc(metricId)}">${markers}${limitLine}<path d="${line}" fill="none" stroke="#58a6ff" stroke-width="1.5"></path>${points}</svg></div>`;
}

export function renderServiceDetail(svc: SaasService, history: SnapshotRecord[], provenance: Provenance): string {
  const f = freshness(svc.measuredAt, svc.cadenceDays);
  const metricRows = svc.metrics.length
    ? svc.metrics
        .map(
          (m) =>
            `<tr><td>${esc(m.label)}${m.highlight ? ' <span class="badge src">highlight</span>' : ""}</td><td>${fmtMetricValue(m.used, m.unit)}</td><td>${m.limit == null ? "—" : fmtMetricValue(m.limit, m.unit)}</td><td>${m.percent != null ? `${m.percent.toFixed(0)}%` : "—"}</td><td class="muted">${esc(m.note ?? "")}</td></tr>`,
        )
        .join("")
    : '<tr><td colspan="5" class="muted">No metrics recorded for this service yet.</td></tr>';
  const payg = svc.paygEstimate
    ? `<h2>PAYG estimate</h2><div class="card">low $${svc.paygEstimate.low.toFixed(2)} / high $${svc.paygEstimate.high.toFixed(2)} ${esc(svc.paygEstimate.period)} — counted as used: <strong>$${countedEstimate(svc.paygEstimate).toFixed(2)}</strong> (1.2 × high, Decision 004).${svc.paygEstimate.note ? `<br><span class="muted">${esc(svc.paygEstimate.note)}</span>` : ""}</div>`
    : svc.paygNote
      ? `<h2>PAYG estimate</h2><div class="card"><span class="muted">${esc(svc.paygNote)} — no number is invented where the records carry no volume.</span></div>`
      : "";
  const metricIds = [...new Set(history.map((r) => r.metricId))];
  const charts = metricIds
    .map((id) => historyChartHtml(history, id))
    .filter(Boolean)
    .join("");
  const paces = metricIds
    .map((id) => ({ id, pace: projectPace(history, id) }))
    .filter((p) => p.pace)
    .map(
      (p) =>
        `<div>${esc(p.id)}: +${p.pace!.slopePerDay} pts/day over ${p.pace!.points} reads${p.pace!.projectedExhaustionAt ? ` → projected to exhaust ${esc(p.pace!.projectedExhaustionAt.slice(0, 10))}` : ""}${p.pace!.projectedPercentAtReset != null ? ` · projected ${p.pace!.projectedPercentAtReset}% at reset` : ""}</div>`,
    )
    .join("");
  const reads = svc.reads.length
    ? svc.reads.map((r) => `<div class="cmt"><span class="muted">${esc(r.at)}</span><br>${esc(r.text)}</div>`).join("")
    : '<p class="muted">No dated reads recorded.</p>';
  const conflicts = svc.conflicts.length
    ? `<h2>Conflicts (shown, not resolved)</h2>${svc.conflicts.map((c) => `<div class="reason">${esc(c)}</div>`).join("")}`
    : "";
  const body = `<p class="muted"><a href="/quota">← Usage &amp; billing</a></p>
<h1>${esc(svc.name)}</h1>
<div class="card${f.stale ? " stale" : ""}"><strong>${esc(svc.plan)}</strong> <span class="badge src">${esc(svc.status)}</span><br>${freshnessStamp(svc.measuredAt, svc.sourceClass, svc.cadenceDays)}<br><span class="muted">source: ${esc(svc.sourceNote)}${svc.nextDate ? ` · next: ${esc(svc.nextDate)}` : ""}${svc.billedUsd != null ? ` · billed this period: $${svc.billedUsd.toFixed(2)}` : ""}</span>${svc.usageNote ? `<br>${esc(svc.usageNote)}` : ""}</div>
<h2>All metrics</h2><div class="tblwrap"><table><tr><th>Metric</th><th>Used</th><th>Limit</th><th>%</th><th>Note</th></tr>${metricRows}</table></div>
${payg}
<h2>Snapshot history</h2>${charts || '<p class="muted">No snapshots in the store for this service yet — history starts with the §4 backfill and accumulates as writers append.</p>'}
${paces ? `<h2>Pace projections</h2><div class="card">${paces}</div>` : ""}
${conflicts}
<h2>Dated source reads</h2>${reads}
<p class="muted">Machine-first: <a href="/api/quota/service.json?id=${encodeURIComponent(svc.id)}">/api/quota/service.json?id=${esc(svc.id)}</a></p>`;
  return layout(`${svc.name} · usage`, body, provenance, "/quota");
}

// --- Attention view (#852, tiered in slice 2) -------------------------------

function sessionCard(
  e: TieredEntry,
  children: Map<string, TieredEntry[]>,
  depth = 0,
  counts?: Map<string, number>,
): string {
  const src = SESSION_SOURCE_LABELS[sourceOf(e)];
  const sub =
    e.parentId && depth === 0
      ? ` <span class="badge src">sub-session${e.agent ? ` · ${esc(e.agent)}` : ""} ↳ ${esc(e.parentId)}</span>`
      : e.agent
        ? ` <span class="badge src">${esc(e.agent)}</span>`
        : "";
  const tierBadge =
    e.tier === "needs-drew"
      ? `<span class="badge drew">needs Drew</span>`
      : e.tier === "needs-orchestrator"
        ? `<span class="badge orch">needs orchestrator</span>`
        : "";
  const evidence = e.supervisionNote ? `${e.evidence} Keeper: ${e.supervisionNote}` : e.evidence;
  const reason = e.drewReason ? `<div class="reason"><strong>Why you:</strong> ${esc(e.drewReason)}</div>` : "";
  const orch = e.orchestratorNote ? `<div class="orch-note">${esc(e.orchestratorNote)}</div>` : "";
  const kids = children.get(e.sessionId) ?? [];
  const kidsHtml = kids.length
    ? `<details class="kids"><summary>${kids.length} sub-session${kids.length === 1 ? "" : "s"}</summary>${kids.map((k) => sessionCard(k, children, depth + 1, counts)).join("")}</details>`
    : "";
  const cmt = counts ? commentBadge(sessionObjectRef(e.sessionId), counts) : "";
  return `<div class="card"><strong>${esc(e.title ?? e.sessionId)}</strong> <span class="badge src">${esc(src)}</span> <span class="badge ${esc(e.state)}">${esc(STATE_LABELS[e.state])}</span>${tierBadge}${sub}${cmt}<br><span class="muted">${esc(e.sessionId)} · ${esc(e.directory ?? "—")}${e.branch ? ` · ${esc(e.branch)}` : ""} · ${esc(e.model ?? "—")}${e.provider ? ` / ${esc(e.provider)}` : ""} · last activity ${fmtDate(e.lastActivityAt)} · ${esc(e.messageCount ?? "—")} msgs${e.outcome ? ` · outcome: ${esc(e.outcome)}` : ""}</span><br>${esc(evidence)}${reason}${orch}${kidsHtml}</div>`;
}

/** A #899 tier item that is not a live session — a decision, task, or gap. */
function tierItemCard(item: TierItem, whyLabel: string, counts?: Map<string, number>): string {
  const blocked = item.blockedBy?.length
    ? `<br><strong>Blocked by:</strong> ${item.blockedBy
        .map((d) => esc(`${d.task} (${d.status}${d.workerLabel ? `, ${d.workerLabel}` : ""})`))
        .join("; ")}`
    : "";
  const question = item.question ? `<br><span class="q">“${esc(item.question)}”</span>` : "";
  const action = item.recommendedAction
    ? `<br><span class="muted">Recommended: ${esc(item.recommendedAction)}</span>`
    : "";
  const ref = item.ref ? `<br><a href="${esc(refUrl(item.ref))}">${esc(item.ref)}</a>` : "";
  const cmt = counts ? commentBadge(tierItemObjectRef(item), counts) : "";
  return `<div class="card"><span class="badge src">${esc(item.source)}</span> <span class="badge">${esc(item.kind)}</span> <strong>${esc(item.title)}</strong>${cmt}<br><strong>${esc(whyLabel)}:</strong> ${esc(item.reason)}${question}${blocked}${action}${ref}</div>`;
}

export function renderAttention(
  view: AttentionView,
  coverage: SourceCoverage[],
  provenance: Provenance,
  counts?: Map<string, number>,
): string {
  const children = sessionChildren(view.all);
  const roots = sessionRoots(view.all);
  const drewTotal = view.drew.length + view.drewExtra.length;
  const orchTotal = view.orchestrator.length + view.orchestratorExtra.length;
  const chips = `<div class="chips"><a href="#drew">Needs Drew (${drewTotal})</a><a href="#orchestrator">Needs orchestrator (${orchTotal})</a><a href="#sessions">All sessions (${roots.length})</a></div>`;
  const drewCards =
    [
      ...view.drew.map((e) => sessionCard(e, children, 0, counts)),
      ...view.drewExtra.map((i) => tierItemCard(i, "Why you", counts)),
    ].join("") || `<p class="muted">Nothing needs you right now.</p>`;
  const orchCards =
    [
      ...view.orchestrator.map((e) => sessionCard(e, children, 0, counts)),
      ...view.orchestratorExtra.map((i) => tierItemCard(i, "Why the fleet", counts)),
    ].join("") || `<p class="muted">The fleet's queue is empty.</p>`;
  const sessionCards = roots.length
    ? roots.map((e) => sessionCard(e, children, 0, counts)).join("")
    : `<p class="muted">No current sessions.</p>`;
  return layout(
    "Needs you",
    `<h1>Needs you</h1><p class="muted">Only what genuinely requires Drew lands in NEEDS DREW — each with its reason. Everything else the fleet can work is the orchestrator's queue. Finished runs and stale items expire out (2 h after a run finishes, 24 h at the latest). Machine-first: <a href="/api/attention.json">/api/attention.json</a> · <a href="/api/coverage.json">/api/coverage.json</a>.</p>
${chips}
${coverageHtml(coverage)}
<h2 id="drew">Needs Drew (${drewTotal})</h2>${drewCards}
<h2 id="orchestrator">Needs orchestrator (${orchTotal})</h2><p class="muted">The orchestration job (#899) reviews this queue and pushes work back into motion; it escalates here only what truly needs Drew.</p>${orchCards}
<h2 id="sessions">All sessions (${roots.length})</h2><p class="muted">Every current session with its source and state; sub-sessions collapse under their parent.</p>${sessionCards}
${view.expiredCount ? `<p class="muted">${view.expiredCount} finished or stale ${view.expiredCount === 1 ? "entry" : "entries"} expired out of this view.</p>` : ""}`,
    provenance,
    "/attention",
  );
}

// --- Today view (slice 2, #898 §4) -------------------------------------------

export function renderToday(
  today: TodayView,
  attention: AttentionView,
  quota: QuotaView,
  coverage: SourceCoverage[],
  provenance: Provenance,
): string {
  const walled = quota.pools.filter((p) => p.state === "walled");
  const alerts: string[] = [];
  const drewTotal = attention.drew.length + attention.drewExtra.length;
  if (drewTotal) {
    alerts.push(
      `<div class="banner drew"><strong>${drewTotal} item${drewTotal === 1 ? "" : "s"} need Drew</strong> — <a href="/attention#drew">review them</a></div>`,
    );
  }
  if (walled.length) {
    alerts.push(
      `<div class="banner"><strong>Walled pools:</strong> ${walled.map((p) => esc(`${p.provider} (${p.plan})`)).join(", ")} — <a href="/quota">quota</a></div>`,
    );
  }
  const provenanceLine = today.fromOrchestrator
    ? `<p class="muted">From the orchestration job's goal measurements (section written ${fmtDate(today.generatedAt)}). Machine-first: <a href="/api/today.json">/api/today.json</a> · <a href="/api/orchestration.json">/api/orchestration.json</a>.</p>`
    : `<div class="banner">Derived from the board, roadmap, and item activity — the orchestration job (#899) has not published a goal-measurement section yet, so this is the aggregator's own derivation, not the orchestrator's measurement.</div>`;
  const actions = today.actions?.length
    ? `<h2>Pushed back into motion</h2>${today.actions.map((a) => `<div class="card"><span class="badge">${esc(a.kind)}</span> <strong>${esc(a.target)}</strong><br>${esc(a.summary)}<br><span class="muted">${esc(fmtDate(a.at))}</span></div>`).join("")}`
    : "";
  const objectives = today.objectives.length
    ? today.objectives
        .map(
          (o, i) =>
            `<div class="card obj"><div class="n">${i + 1}</div><div><strong>${esc(o.title)}</strong> <span class="badge src">${esc(o.chip)}</span>${o.detail ? `<br><span class="muted">${esc(o.detail)}</span>` : ""}${o.ref ? `<br><a href="${esc(refUrl(o.ref))}">${esc(o.ref)}</a>` : ""}</div></div>`,
        )
        .join("")
    : `<p class="muted">No objectives lined up for today.</p>`;
  const momentum = today.momentum.length
    ? today.momentum
        .map((m) => {
          // Defensive: a TodayView built outside derive.ts may still
          // carry a non-array `accomplished` (e.g. a bare string) —
          // coerce instead of crashing the whole /today render.
          const rawAccomplished: unknown = m.accomplished;
          const accomplished = (Array.isArray(rawAccomplished)
            ? rawAccomplished
            : rawAccomplished
              ? [rawAccomplished]
              : []
          ).map((a) => String(a));
          const done = accomplished.length
            ? `<br>Recently accomplished:<ul>${accomplished.map((a) => `<li>${esc(a)}</li>`).join("")}</ul>`
            : "";
          const blocked = m.blockedBy ? `<br><strong>Blocked by:</strong> ${esc(m.blockedBy)}` : "";
          const detail = m.detail ? `<br><span class="muted">${esc(m.detail)}</span>` : "";
          return `<div class="card"><strong>${esc(m.project)}</strong> <span class="badge ${esc(m.state)}">${esc(m.state)}</span>${done}${blocked}${detail}</div>`;
        })
        .join("")
    : `<p class="muted">No projects to measure yet.</p>`;
  return layout(
    "Today",
    `<h1>Today</h1>${provenanceLine}${alerts.join("")}
<h2>Top objectives</h2>${objectives}
<h2>Project momentum</h2>${momentum}
${actions}${coverageHtml(coverage)}`,
    provenance,
    "/today",
  );
}

// --- DNS register page (#906 §1, data from #902) -----------------------------

function dnsTable(entries: DnsEntry[]): string {
  const rows = entries
    .map(
      (e) =>
        `<tr><td><strong>${esc(e.domain)}</strong></td><td>${esc(e.renewal)}</td><td>${e.priceUsd != null ? `$${e.priceUsd.toFixed(2)}` : "—"}</td><td>${esc(e.autoRenew)}</td><td>${esc(e.plan)}</td><td>${esc(e.review)}</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table><tr><th>Domain</th><th>Next renewal</th><th>Price (list)</th><th>Auto-renew</th><th>Plan / status</th><th>Review?</th></tr>${rows || '<tr><td colspan="6" class="muted">none</td></tr>'}</table></div>`;
}

export function renderDns(section: DnsSection | null, provenance: Provenance): string {
  if (!section) {
    return layout(
      "DNS register",
      `<h1>DNS / domain register</h1><div class="banner">The register has not reached the dashboard yet — the fleet-feed collector embeds it from #902's register file, and no section is in the current feed. The register of record is the file the #902 pipeline maintains; this page only renders it.</div>`,
      provenance,
      "/dns",
    );
  }
  const s = summarizeDns(section);
  const next = s.nextRenewal;
  const soonTotal = Math.round(s.renewingSoon.reduce((sum, e) => sum + (e.priceUsd ?? 0), 0) * 100) / 100;
  const body = `<h1>DNS / domain register</h1>
<p class="muted">${section.entries.length} domains · $${section.totalAnnualListUsd.toFixed(2)}/yr at list renewal prices · register parsed ${fmtDate(section.generatedAt)} (${esc(section.source)}). The register is maintained by the #902 pipeline — this page renders it, never edits it. Machine-first: <a href="/api/dns.json">/api/dns.json</a>.</p>
<div class="board">
<div class="card"><strong>Next renewal</strong><br>${
    next
      ? `<strong>${esc(next.domain)}</strong> — ${esc(next.renewal)}${next.priceUsd != null ? ` ($${next.priceUsd.toFixed(2)})` : ""}<br><span class="muted">${esc(next.plan)}</span>`
      : "—"
  }</div>
<div class="card"><strong>Renewing in 45 days</strong><br>${s.renewingSoon.length} domain${s.renewingSoon.length === 1 ? "" : "s"} · $${soonTotal.toFixed(2)}<br><span class="muted">Auto-renew is ON across the account portfolio — this is upcoming spend, not lapse risk.</span></div>
<div class="card"><strong>Portfolio shape</strong><br>${s.active.length} in use · ${s.earmarked.length} unused but earmarked · ${s.unused.length} completely unused</div>
</div>
<h2>Next renewals</h2>${dnsTable(section.entries.slice(0, 10))}
<h2>In use (${s.active.length})</h2>${dnsTable(s.active)}
<h2>Unused but earmarked (${s.earmarked.length})</h2><p class="muted">No public-facing use, but the name or plan ties them to a project family (earmark as recorded in the register).</p>${dnsTable(s.earmarked)}
<h2>Completely unused (${s.unused.length})</h2><p class="muted">Nothing but registrar defaults — each still auto-renews until someone decides otherwise.</p>${dnsTable(s.unused)}
<details><summary>Full register (${section.entries.length})</summary>${dnsTable(section.entries)}</details>`;
  return layout("DNS register", body, provenance, "/dns");
}

// --- Sessions / identity registry page (#906 §3) ------------------------------

const REGISTRY_SOURCE_ORDER = ["opencode-laptop", "opencode-sandbox", "jules", "claude-code", "cron"];

export function renderSessions(records: SessionRecord[], provenance: Provenance): string {
  const bySource = new Map<string, SessionRecord[]>();
  for (const r of records) bySource.set(r.source, [...(bySource.get(r.source) ?? []), r]);
  const totalTokens = records.reduce((sum, r) => sum + (r.tokensTotal ?? 0), 0);
  const groups = [...bySource.entries()]
    .sort((a, b) => REGISTRY_SOURCE_ORDER.indexOf(a[0]) - REGISTRY_SOURCE_ORDER.indexOf(b[0]))
    .map(
      ([source, list]) => `<h2>${esc(source)} (${list.length})</h2>${list.map((r) => sessionRecordLine(r)).join("")}`,
    )
    .join("");
  const body = `<h1>Sessions — identity registry</h1>
<p class="muted">Every agent/session the fleet can see, with its stable id, the agent identity it ran as, its parent session, and its token total where the source reports one. Task links are loose (from the session's PR, title, or launch record) and improve over time. ${records.length} records · ${totalTokens.toLocaleString()} tokens recorded. Machine-first: <a href="/api/registry.json">/api/registry.json</a>.</p>
<p class="muted">Coverage honesty: OpenCode sessions register from their servers; Jules from the keeper's state; Muse sessions only when a launch record was appended at spawn (<code>scripts/record-claude-launch.ts</code>) — a Muse session that never wrote one is invisible here, and the coverage note on Needs you says which sources are wired.</p>
${groups || '<p class="muted">No sessions recorded yet.</p>'}`;
  return layout("Sessions", body, provenance, "/sessions");
}

// --- All pages (the manifest as a page) ---------------------------------------

export function renderPages(provenance: Provenance): string {
  const rows = PAGES.map(
    (p) =>
      `<tr><td><a href="${p.route}">${esc(p.title)}</a></td><td><code>${esc(p.route)}</code></td><td>${p.api ? `<a href="${p.api}"><code>${esc(p.api)}</code></a>` : '<span class="muted">—</span>'}</td><td>${esc(p.blurb)}</td></tr>`,
  ).join("");
  return layout(
    "All pages",
    `<h1>All pages</h1><p class="muted">Every registered page, from the manifest in <code>src/pages.ts</code> — the nav bar and the page dropdown render from the same list. Adding a page is one manifest entry + a render function + a route (recipe in the README).</p><div class="tblwrap"><table><tr><th>Page</th><th>Route</th><th>JSON twin</th><th>What it is</th></tr>${rows}</table></div>`,
    provenance,
    "/pages",
  );
}

// --- Discuss page (one object's dashboard comments) -----------------------------

export function renderDiscuss(
  objectRef: string,
  title: string,
  comments: DashComment[],
  opts: CommentFormOpts,
  provenance: Provenance,
): string {
  const back =
    objectRef.startsWith("gh:") || objectRef.startsWith("kaneo:") || objectRef.startsWith("grist:")
      ? `<p class="muted"><a href="/item/${encodeURIComponent(objectRef)}">← the item page</a></p>`
      : objectRef.startsWith("session:")
        ? `<p class="muted"><a href="/sessions">← the session registry</a> · <a href="/attention">Needs you</a></p>`
        : `<p class="muted"><a href="/attention">← Needs you</a></p>`;
  const body = `<h1>${esc(title)}</h1><p class="muted">object: <code>${esc(objectRef)}</code></p>${back}
<p class="muted">Two kinds, visibly distinct: a <strong>note</strong> is read when this is next picked up and triggers nothing; a <strong>message to Muse</strong> is triaged by the orchestrator on its next pass and may create work.${
    opts.githubRefs?.length
      ? ` This item is GitHub-backed, so a comment can also be posted as a <strong>GitHub comment</strong> on an explicit target.`
      : ""
  }</p>
${commentThreadHtml(objectRef, comments, opts)}`;
  return layout(title, body, provenance, "", COMMENT_SCRIPT);
}

// --- Projects page (#910 stage 5: the live GitHub Project pool) ------------
// New function only (parallel agents own the other render functions).
// Renders the feed's Project pool; where the Project Status and the
// derived state (see src/projects.ts derivedProjectState) disagree,
// BOTH are shown — neither side is silently preferred.

export function renderProjects(
  pool: ProjectPoolItem[],
  provenance: Provenance,
  opts: { area?: string | null; orchestration?: OrchestrationSection | null } = {},
): string {
  const areas = [...new Set(pool.map((p) => p.area).filter((a): a is string => Boolean(a)))].sort();
  const area = opts.area && areas.includes(opts.area) ? opts.area : null;
  const shown = area ? pool.filter((p) => p.area === area) : pool;
  const chips = [
    `<a href="/projects"${!area ? ' class="active"' : ""}>all (${pool.length})</a>`,
    ...areas.map(
      (a) =>
        `<a href="/projects?area=${encodeURIComponent(a)}"${a === area ? ' class="active"' : ""}>${esc(a)} (${pool.filter((p) => p.area === a).length})</a>`,
    ),
  ].join("");
  if (!pool.length) {
    return layout(
      "Projects",
      `<h1>Projects — GitHub Project pool</h1><div class="banner">No Project pool in the current fleet feed — the collector embeds it from the org Project <a href="${PROJECT_URL}">Developing Today (#4)</a> when a Projects token is available on the sandbox. Items on the board/roadmap fall back to the projector/fixtures chain until the pool lands.</div>`,
      provenance,
      "/projects",
    );
  }
  const cards = shown
    .map((p) => {
      const derived = derivedProjectState(p, opts.orchestration);
      const clash = projectDisagreement(p, opts.orchestration);
      const clashBadge = clash
        ? ` <span class="badge stale" title="The Project Status and the state the rest of the dashboard derives disagree — both are shown, neither is preferred">⚠ project: ${esc(clash.project)} · derived: ${esc(clash.derived)}</span>`
        : "";
      const statusBadge = p.status
        ? `<span class="badge ${p.status === "Done" ? "merged" : p.status === "Todo" ? "closed" : "implementing"}">${esc(p.status)}</span>`
        : `<span class="badge">no status</span>`;
      const meta = [
        p.area ? `<span class="badge src">${esc(p.area)}</span>` : "",
        p.horizon ? `<span class="badge src">${esc(p.horizon)}</span>` : "",
        p.kind ? `<span class="badge src">${esc(p.kind)}</span>` : "",
        `<span class="badge ${p.state === "open" ? "open" : "closed"}">issue ${esc(p.state)}</span>`,
        `<span class="badge src">derived: ${esc(derived)}</span>`,
      ].join(" ");
      return `<div class="card"><strong>${esc(p.title)}</strong>${clashBadge}<br>${statusBadge} ${meta}<br><span class="muted"><a href="${esc(p.issueUrl)}">${esc(p.repo)}#${p.number}</a> · <a href="${esc(p.url)}">Project item</a> · <a href="${esc(p.projectUrl)}">Project #4</a>${p.updatedAt ? ` · updated ${fmtDate(p.updatedAt)}` : ""}</span></div>`;
    })
    .join("");
  const body = `<h1>Projects — GitHub Project pool</h1>
<p class="muted">${pool.length} items from the org Project <a href="${PROJECT_URL}">Developing Today (#4)</a>, read live by the fleet-feed collector (GraphQL, read-only) and carried in the feed. This pool is the resolution source for board/roadmap GitHub items. Where the Project Status and the derived state disagree, the card shows ⚠ with both values. Machine-first: <a href="/api/projects.json">/api/projects.json</a>.</p>
<div class="chips">${chips}</div>
${cards || '<p class="muted">No items in this area.</p>'}`;
  return layout("Projects", body, provenance, "/projects");
}

// --- Program status page (#1046: the dashboard documents itself) ------------
// Editorial sections come from src/program.ts (the PROGRAM object — the
// one file to edit when the program changes); the live-status section is
// derived from the Source at request time by buildProgramLive, so its
// claims cannot drift from the tracked issues / Project pool /
// orchestrator they describe. Unresolvable refs render as "unresolved".

function programEntryHtml(e: ProgramEntry, items: Map<RefKey, GhItem>): string {
  const refs = (e.refs ?? [])
    .map((r) => {
      const item = items.get(r);
      return item ? itemBadge(item) : `<span class="badge closed" title="The current source cannot resolve this ref">unresolved ${esc(r)}</span>`;
    })
    .join(" ");
  const links = (e.links ?? []).map((l) => `<a href="${esc(l.url)}">${esc(l.label)}</a>`).join(" · ");
  const meta = [refs, links].filter(Boolean).join(" ");
  return `<div class="card"><strong>${esc(e.title)}</strong><br>${esc(e.detail)}${meta ? `<br>${meta}` : ""}</div>`;
}

function programEntries(entries: ProgramEntry[], items: Map<RefKey, GhItem>): string {
  return entries.map((e) => programEntryHtml(e, items)).join("");
}

export function renderProgram(data: ProgramData, items: Map<RefKey, GhItem>, provenance: Provenance): string {
  const { program, live } = data;
  const trackedRows = live.tracked
    .map(({ ref, item }) =>
      item
        ? `<tr><td>${itemBadge(item)}</td><td>${esc(item.title)}</td><td><span class="badge ${item.state === "open" ? "open" : item.state === "merged" ? "merged" : "closed"}">${esc(item.state)}</span></td><td>${fmtDate(item.updatedAt)}</td></tr>`
        : `<tr><td><span class="badge closed">unresolved</span></td><td><code>${esc(ref)}</code></td><td><span class="muted">unresolved — the current source cannot resolve this ref</span></td><td>—</td></tr>`,
    )
    .join("");
  const countList = (list: { name: string; count: number }[]) =>
    list.length ? list.map((c) => `${esc(c.name)}: ${c.count}`).join(" · ") : '<span class="muted">none</span>';
  const poolHtml = live.pool.total
    ? `<div class="card"><strong><a href="/projects">${live.pool.total} items</a></strong> in the org Project pool<br><span class="muted">By status — ${countList(live.pool.byStatus)}</span><br><span class="muted">By area — ${countList(live.pool.byArea)}</span></div>`
    : `<div class="banner">No Project pool in the current feed — the collector embeds it from the org Project when a Projects token is available. The pool summary appears here when it lands; the tracked issues above still resolve through the projector/fixtures chain.</div>`;
  const orchHtml = live.orchestration
    ? `<div class="card"><strong>Orchestrator</strong> — latest run <code>${esc(live.orchestration.runId)}</code>, published ${fmtDate(live.orchestration.generatedAt)}.<br><span class="muted">The tiers and goal measurements it publishes are on <a href="/attention">Needs you</a> and <a href="/today">Today</a>.</span></div>`
    : `<div class="card"><strong>Orchestrator</strong> — <span class="muted">no orchestration section in the current feed yet (not measured). The Today view derives and labels itself until one lands.</span></div>`;
  const body = `<h1>Program status — what this dashboard is</h1>
<p class="muted">${esc(program.intro)}</p>
<p class="muted">Source of truth: the <a href="${esc(program.designRecordUrl)}">design record</a> in developing-today/code · the org Project <a href="${esc(program.projectUrl)}">Developing Today (#4)</a> · Machine-first: <a href="/api/program.json">/api/program.json</a>.</p>
<h2>What this is — architecture</h2>
${programEntries(program.architecture, items)}
<h2>Live status</h2>
<p class="muted">Derived at request time from the tracked issues, the Project pool, and the latest orchestration section — not written by hand. A ref the current source cannot resolve says so.</p>
<div class="tblwrap"><table><tr><th>Tracked issue</th><th>Title</th><th>State</th><th>Updated</th></tr>${trackedRows}</table></div>
${poolHtml}
${orchHtml}
<h2>What's built</h2>
${programEntries(program.done, items)}
<h2>What's next</h2>
${programEntries(program.next, items)}
<h2>Long-term roadmap</h2>
${programEntries(program.longTerm, items)}
<h2>Known gaps</h2>
${programEntries(program.gaps, items)}`;
  return layout("Program status", body, provenance, "/program");
}
