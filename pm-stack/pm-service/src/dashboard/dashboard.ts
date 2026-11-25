/**
 * Dashboard surfaces, Workers edition (component A slice 4, #12829;
 * design doc/2026-11-23T03-22-03Z_design_pm-service-workers-edition
 * §4, the V1 parity list).
 *
 * Server-rendered HTML pages over the tenant DB, read through the
 * same async loads the read API serves (read-api/async-store.ts), so
 * a page and its JSON twin can never disagree about the data:
 *
 *   /                 board views over the full registry + per-board audit
 *   /board/<id>       one board's items
 *   /item/<ref>       item detail with the genealogy header + sentence
 *   /roadmap          per-project roadmap + the overall priorities view
 *   /releases         milestone snapshots and close-out cards
 *   /attention        needs-attention: open reconcile findings and the
 *                     collector-checked invariants over the snapshots
 *   /quota            quota/usage: named as not yet in the tenant store
 *
 * Read-only: GETs only, SELECTs only, no token read or written on this
 * path (E §2.3). Absence renders honestly: a board with no snapshot
 * says so, an item with no chain shows the chain-break sentence, and
 * nothing is invented to fill a gap (#14065).
 */
import type { ProjectPoolItem } from "../collector/pool.ts";
import type { AsyncSqlDb } from "../tenant-store/libsql-http.ts";
import { deriveGenealogy, type RoadmapRow } from "../read-api/genealogy.ts";
import type { ReleaseSnapshot } from "../read-api/releases.ts";
import {
  loadBoardSummariesAsync,
  loadPoolAsync,
  loadReleaseSnapshotsAsync,
  loadRoadmapRowsAsync,
} from "../read-api/async-store.ts";
import type { BoardSummary } from "../read-api/store.ts";

export interface DashboardDeps {
  db: AsyncSqlDb;
  /** Clock seam for the "rendered at" stamp (tests pin it). */
  now?: () => string;
}

export interface OpenFinding {
  findingId: string;
  boardId: string | null;
  kind: string;
  severity: "info" | "warning" | "error";
  subjectRef: string | null;
  summary: string;
  recordedAt: string;
}

/** A needs-attention line derived from the stored snapshots (the
 * collector-checked invariants of #9060 §3/§7). */
export interface AuditCheck {
  boardId: string;
  severity: "warning" | "error";
  summary: string;
}

const FINDINGS_SQL = `SELECT finding_id, board_id, kind, severity, subject_ref, summary, recorded_at
  FROM pm_reconcile_findings WHERE resolved_at IS NULL
  ORDER BY CASE severity WHEN 'error' THEN 0 WHEN 'warning' THEN 1 ELSE 2 END, recorded_at DESC, finding_id`;

const BOARD_ITEMS_SQL = "SELECT board_id, item_json FROM pm_items ORDER BY board_id, ref";

interface FindingRow {
  finding_id: string;
  board_id: string | null;
  kind: string;
  severity: string;
  subject_ref: string | null;
  summary: string;
  recorded_at: string;
}

export async function loadOpenFindingsAsync(db: AsyncSqlDb): Promise<OpenFinding[]> {
  const rows = await db.all<FindingRow>(FINDINGS_SQL);
  return rows.map((r) => ({
    findingId: String(r.finding_id),
    boardId: r.board_id === null ? null : String(r.board_id),
    kind: String(r.kind),
    severity: String(r.severity) as OpenFinding["severity"],
    subjectRef: r.subject_ref === null ? null : String(r.subject_ref),
    summary: String(r.summary),
    recordedAt: String(r.recorded_at),
  }));
}

async function loadBoardItemsAsync(db: AsyncSqlDb, boardId: string): Promise<ProjectPoolItem[]> {
  const rows = await db.all<{ board_id: string; item_json: string }>(BOARD_ITEMS_SQL);
  return rows.filter((r) => String(r.board_id) === boardId).map((r) => JSON.parse(String(r.item_json)) as ProjectPoolItem);
}

/** The invariants the collector checks, read back over the latest
 * snapshot per board: a board with no snapshot, a snapshot that
 * emitted fewer entries than the board reported (a truncated or
 * partial ingest), and an audit whose item total disagrees with the
 * snapshot's reported total. */
export function auditChecks(boards: readonly BoardSummary[]): AuditCheck[] {
  const checks: AuditCheck[] = [];
  for (const { board, snapshot, audit } of boards) {
    if (!snapshot) {
      checks.push({ boardId: board.id, severity: "warning", summary: "no snapshot has landed for this board yet" });
      continue;
    }
    if (snapshot.emittedEntries < snapshot.reportedTotal) {
      checks.push({
        boardId: board.id,
        severity: "error",
        summary: `latest snapshot emitted ${snapshot.emittedEntries} of ${snapshot.reportedTotal} reported items`,
      });
    }
    if (audit && audit.totalItems !== snapshot.reportedTotal) {
      checks.push({
        boardId: board.id,
        severity: "warning",
        summary: `audit counts ${audit.totalItems} items but the snapshot reported ${snapshot.reportedTotal}`,
      });
    }
  }
  return checks;
}

// ---------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------

export function escapeHtml(value: unknown): string {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

const e = escapeHtml;

/** Link targets are only ever pages of this dashboard, or https URLs
 * taken from GitHub board data; anything else renders as text. */
function externalLink(url: string | undefined | null, text: string): string {
  if (url && /^https:\/\//.test(url)) return `<a href="${e(url)}" rel="noopener noreferrer">${e(text)}</a>`;
  return e(text);
}

const itemHref = (ref: string) => `/item/${encodeURIComponent(ref)}`;
const itemLink = (ref: string, text = ref) => `<a href="${e(itemHref(ref))}">${e(text)}</a>`;

const NAV: [string, string][] = [
  ["/", "Boards"],
  ["/roadmap", "Roadmap"],
  ["/releases", "Releases"],
  ["/attention", "Needs attention"],
  ["/quota", "Quota"],
];

const CSS = `body{font:14px/1.45 system-ui,sans-serif;margin:0;color:#1f2328;background:#fff}
header{padding:10px 16px;border-bottom:1px solid #d0d7de;display:flex;gap:16px;align-items:baseline;flex-wrap:wrap}
header b{margin-right:8px}main{padding:16px;max-width:1100px}
table{border-collapse:collapse;width:100%;margin:8px 0 16px}th,td{text-align:left;padding:4px 8px;border-bottom:1px solid #eaeef2;vertical-align:top}
.card{border:1px solid #d0d7de;border-radius:6px;padding:10px 12px;margin:0 0 12px}
.muted{color:#656d76}.sev-error{color:#cf222e}.sev-warning{color:#9a6700}.sev-info{color:#0969da}
.crumbs{font-size:13px}.crumbs span{color:#656d76}
@media (prefers-color-scheme:dark){body{background:#0d1117;color:#e6edf3}header,th,td,.card{border-color:#30363d}.muted,.crumbs span{color:#8d96a0}a{color:#4493f8}}`;

function page(title: string, body: string, renderedAt: string, status = 200): Response {
  const nav = NAV.map(([href, label]) => `<a href="${href}">${e(label)}</a>`).join("");
  const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>${e(title)} · PM</title><style>${CSS}</style></head><body>
<header><b>PM</b>${nav}</header>
<main><h1>${e(title)}</h1>${body}<p class="muted">Rendered ${e(renderedAt)} from the tenant store.</p></main></body></html>`;
  return new Response(html, {
    status,
    headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" },
  });
}

function countsLine(counts: Record<string, number> | undefined): string {
  const entries = Object.entries(counts ?? {});
  if (entries.length === 0) return '<span class="muted">none</span>';
  return entries.map(([k, v]) => `${e(k)} ${e(v)}`).join(", ");
}

export function renderBoards(boards: readonly BoardSummary[], renderedAt: string): Response {
  const titleOf = new Map(boards.map((b) => [b.board.id, b.board.title]));
  const rows = boards
    .map(({ board, snapshot, audit }) => {
      const parent = board.parentBoardId ? e(titleOf.get(board.parentBoardId) ?? board.parentBoardId) : "";
      const snap = snapshot
        ? `${e(snapshot.generatedAt)}<br><span class="muted">${e(snapshot.emittedEntries)}/${e(snapshot.reportedTotal)} items, ${e(snapshot.writer)} epoch ${e(snapshot.epoch)}</span>`
        : '<span class="muted">no snapshot yet</span>';
      const status = audit ? countsLine(audit.counts.status) : '<span class="muted">no audit</span>';
      return `<tr><td><a href="/board/${encodeURIComponent(board.id)}">${e(board.title)}</a><br><span class="muted">${externalLink(board.url, `${board.org} #${board.number}`)}</span></td>
<td>${e(board.role)}</td><td>${parent}</td><td>${snap}</td><td>${status}</td></tr>`;
    })
    .join("\n");
  const body = boards.length
    ? `<table><thead><tr><th>Board</th><th>Role</th><th>Parent</th><th>Latest snapshot</th><th>Status counts</th></tr></thead><tbody>${rows}</tbody></table>`
    : '<p class="muted">The registry has no boards.</p>';
  return page("Boards", body, renderedAt);
}

export function renderBoard(summary: BoardSummary, items: readonly ProjectPoolItem[], renderedAt: string): Response {
  const { board, snapshot, audit } = summary;
  const head = `<p>${externalLink(board.url, `${board.org} project ${board.number}`)} · ${e(board.role)}</p>`;
  if (!snapshot) return page(board.title, `${head}<p class="muted">No snapshot has landed for this board yet.</p>`, renderedAt);
  const views = audit?.views.length ? `<p class="muted">Views: ${audit.views.map((v) => e(v.name)).join(", ")}</p>` : "";
  const rows = items
    .map(
      (item) => `<tr><td>${itemLink(item.ref)}</td><td>${e(item.title)}</td><td>${e(item.status ?? "")}</td>
<td>${e(item.kind ?? "")}</td><td>${e(item.area ?? "")}</td><td>${e(item.state)}</td></tr>`,
    )
    .join("\n");
  const table = items.length
    ? `<table><thead><tr><th>Item</th><th>Title</th><th>Status</th><th>Kind</th><th>Area</th><th>State</th></tr></thead><tbody>${rows}</tbody></table>`
    : '<p class="muted">The latest snapshot has no items.</p>';
  const meta = `<p class="muted">Snapshot ${e(snapshot.generatedAt)}, ${e(snapshot.emittedEntries)} of ${e(snapshot.reportedTotal)} items, written by ${e(snapshot.writer)} at epoch ${e(snapshot.epoch)}.</p>`;
  return page(board.title, `${head}${meta}${views}${table}`, renderedAt);
}

export function renderItem(item: ProjectPoolItem, pool: readonly ProjectPoolItem[], renderedAt: string): Response {
  const g = deriveGenealogy(item, pool);
  // Root first, the item last: the header reads like a path.
  const crumbs = [...g.ancestors]
    .reverse()
    .map((a) => `${itemLink(a.ref, a.title)} <span>(${e(a.relation)})</span>`)
    .concat(`<b>${e(item.title)}</b>`)
    .join(" › ");
  const release = g.release
    ? `<p>Release: ${itemLink(g.release.ref, g.release.title)}${g.release.dueOn ? ` <span class="muted">due ${e(g.release.dueOn)}</span>` : ""}</p>`
    : '<p class="muted">No release.</p>';
  const after = g.afterRefs.length ? `<p>After: ${g.afterRefs.map((r) => itemLink(r)).join(", ")}</p>` : "";
  const fields: [string, string][] = [
    ["Ref", externalLink(item.issueUrl, item.ref)],
    ["State", e(item.state)],
    ["Status", e(item.status ?? "")],
    ["Kind", e(item.kind ?? "")],
    ["Area", e(item.area ?? "")],
    ["Horizon", e(item.horizon ?? "")],
    ["Milestone", e(item.milestone?.title ?? "")],
    ["Labels", (item.labels ?? []).map(e).join(", ")],
    ["Blocked by", (item.blockedBy ?? []).map((r) => itemLink(r)).join(", ")],
    ["Blocks", (item.blocks ?? []).map((r) => itemLink(r)).join(", ")],
  ];
  const table = `<table><tbody>${fields
    .filter(([, v]) => v !== "")
    .map(([k, v]) => `<tr><th>${e(k)}</th><td>${v}</td></tr>`)
    .join("")}</tbody></table>`;
  const body = `<p class="crumbs">${crumbs}</p><p>${e(g.sentence)}</p>${release}${after}${table}`;
  return page(item.title, body, renderedAt);
}

function roadmapTable(rows: readonly RoadmapRow[]): string {
  const body = rows
    .map((row) => {
      const priority = row.priority ? itemLink(row.priority) : "";
      const after = (row.afterRefs ?? []).map((r) => itemLink(r)).join(", ");
      const release = row.releaseRef ? itemLink(row.releaseRef) : "";
      const links = row.links.map((l) => `${itemLink(l.ref)} <span class="muted">${e(l.relation)}</span>`).join(", ");
      const horizon = row.horizonError ? `${e(row.horizon)} <span class="sev-warning">${e(row.horizonError)}</span>` : e(row.horizon);
      return `<tr><td>${e(row.title)}</td><td>${e(row.status)}</td><td>${horizon}</td><td>${links}</td><td>${priority}</td><td>${after}</td><td>${release}</td></tr>`;
    })
    .join("\n");
  return `<table><thead><tr><th>Row</th><th>Status</th><th>Horizon</th><th>Links</th><th>Priority</th><th>After</th><th>Release</th></tr></thead><tbody>${body}</tbody></table>`;
}

export function renderRoadmap(rows: readonly RoadmapRow[], renderedAt: string): Response {
  if (rows.length === 0) return page("Roadmap", '<p class="muted">The tenant store has no roadmap rows.</p>', renderedAt);
  // Overall priorities: every row that names a priority, grouped by it.
  const byPriority = new Map<string, RoadmapRow[]>();
  for (const row of rows) {
    if (!row.priority) continue;
    byPriority.set(row.priority, [...(byPriority.get(row.priority) ?? []), row]);
  }
  const priorities = byPriority.size
    ? [...byPriority.entries()]
        .sort(([a], [b]) => (a < b ? -1 : 1))
        .map(([ref, group]) => `<h3>${itemLink(ref)}</h3>${roadmapTable(group)}`)
        .join("")
    : '<p class="muted">No roadmap row names a priority yet.</p>';
  // Per-project roadmap: rows grouped by area.
  const byArea = new Map<string, RoadmapRow[]>();
  for (const row of rows) byArea.set(row.area || "(no area)", [...(byArea.get(row.area || "(no area)") ?? []), row]);
  const projects = [...byArea.entries()]
    .sort(([a], [b]) => (a < b ? -1 : 1))
    .map(([area, group]) => `<h3>${e(area)}</h3>${roadmapTable(group)}`)
    .join("");
  return page("Roadmap", `<h2>Overall priorities</h2>${priorities}<h2>By project</h2>${projects}`, renderedAt);
}

export function renderReleases(releases: readonly ReleaseSnapshot[], renderedAt: string): Response {
  if (releases.length === 0) return page("Releases", '<p class="muted">No milestones in the tenant store.</p>', renderedAt);
  const cards = releases
    .map((r) => {
      const c = r.counts;
      const date = r.releaseDate ? ` · ${e(r.releaseDate)}` : "";
      const carried = r.carriedOverEpics.length
        ? `<p>Carried over: ${r.carriedOverEpics.map((ref) => itemLink(ref)).join(", ")}</p>`
        : "";
      return `<div class="card"><h3>${e(r.milestone.title)}</h3><p class="muted">${e(r.kind)}${date}</p>
<p>${e(r.descriptionLine)}</p>
<table><thead><tr><th></th><th>Closed</th><th>Carried over</th><th>Total</th></tr></thead><tbody>
${(["issues", "designs", "epics", "total"] as const)
  .map((k) => `<tr><th>${e(k)}</th><td>${e(c[k].closed)}</td><td>${e(c[k].carriedOver)}</td><td>${e(c[k].total)}</td></tr>`)
  .join("")}
</tbody></table>${carried}</div>`;
    })
    .join("\n");
  return page("Releases", cards, renderedAt);
}

export function renderAttention(
  findings: readonly OpenFinding[],
  checks: readonly AuditCheck[],
  renderedAt: string,
): Response {
  const checkRows = checks
    .map(
      (c) => `<tr><td class="sev-${e(c.severity)}">${e(c.severity)}</td><td><a href="/board/${encodeURIComponent(c.boardId)}">${e(c.boardId)}</a></td><td>${e(c.summary)}</td></tr>`,
    )
    .join("\n");
  const findingRows = findings
    .map((f) => {
      const subject = f.subjectRef ? (f.subjectRef.includes("#") ? itemLink(f.subjectRef) : e(f.subjectRef)) : "";
      return `<tr><td class="sev-${e(f.severity)}">${e(f.severity)}</td><td>${e(f.kind)}</td><td>${e(f.boardId ?? "")}</td><td>${subject}</td><td>${e(f.summary)}</td><td>${e(f.recordedAt)}</td></tr>`;
    })
    .join("\n");
  const audit = checks.length
    ? `<table><thead><tr><th>Severity</th><th>Board</th><th>Check</th></tr></thead><tbody>${checkRows}</tbody></table>`
    : '<p class="muted">Every board\'s latest snapshot passes the coverage checks.</p>';
  const open = findings.length
    ? `<table><thead><tr><th>Severity</th><th>Kind</th><th>Board</th><th>Subject</th><th>Finding</th><th>Recorded</th></tr></thead><tbody>${findingRows}</tbody></table>`
    : '<p class="muted">No open reconcile or ingest findings.</p>';
  return page("Needs attention", `<h2>Coverage and audit</h2>${audit}<h2>Open findings</h2>${open}`, renderedAt);
}

export function renderQuota(renderedAt: string): Response {
  return page(
    "Quota",
    `<p>Quota and usage are not in the tenant store yet. The laptop aggregator reads them from its own stores, and no
hosted slice lands them in this database, so this page shows nothing rather than numbers it cannot back.</p>`,
    renderedAt,
  );
}

function notFound(what: string, renderedAt: string): Response {
  return page("Not found", `<p>${e(what)}</p>`, renderedAt, 404);
}

const BOARD_PATH = /^\/board\/([^/]+)$/;
const ITEM_PATH = /^\/item\/(.+)$/;

/** The dashboard's fetch handler. Returns null for paths it does not
 * own, so the Worker can fall through to the JSON read API. */
export function createDashboardHandler(deps: DashboardDeps): (req: Request) => Promise<Response | null> {
  const now = deps.now ?? (() => new Date().toISOString());
  return async (req) => {
    const path = new URL(req.url).pathname;
    const owned =
      path === "/" ||
      path === "/roadmap" ||
      path === "/releases" ||
      path === "/attention" ||
      path === "/quota" ||
      BOARD_PATH.test(path) ||
      ITEM_PATH.test(path);
    if (!owned) return null;
    if (req.method !== "GET") return page("Method not allowed", "<p>The dashboard is read-only.</p>", now(), 405);
    const at = now();
    if (path === "/") return renderBoards(await loadBoardSummariesAsync(deps.db), at);
    if (path === "/roadmap") return renderRoadmap(await loadRoadmapRowsAsync(deps.db, await loadPoolAsync(deps.db)), at);
    if (path === "/releases") {
      return renderReleases(await loadReleaseSnapshotsAsync(deps.db, await loadPoolAsync(deps.db)), at);
    }
    if (path === "/attention") {
      const [findings, boards] = await Promise.all([loadOpenFindingsAsync(deps.db), loadBoardSummariesAsync(deps.db)]);
      return renderAttention(findings, auditChecks(boards), at);
    }
    if (path === "/quota") return renderQuota(at);
    const boardMatch = BOARD_PATH.exec(path);
    if (boardMatch) {
      let id: string;
      try {
        id = decodeURIComponent(boardMatch[1] as string);
      } catch {
        return notFound("That board id is not valid.", at);
      }
      const summary = (await loadBoardSummariesAsync(deps.db)).find((b) => b.board.id === id);
      if (!summary) return notFound(`No board ${id} in the registry.`, at);
      return renderBoard(summary, await loadBoardItemsAsync(deps.db, id), at);
    }
    const itemMatch = ITEM_PATH.exec(path);
    if (itemMatch) {
      let ref: string;
      try {
        ref = decodeURIComponent(itemMatch[1] as string);
      } catch {
        return notFound("That item ref is not valid.", at);
      }
      const pool = await loadPoolAsync(deps.db);
      const item = pool.find((entry) => entry.ref === ref);
      if (!item) return notFound(`No pool item with ref ${ref}.`, at);
      return renderItem(item, pool, at);
    }
    return null;
  };
}
