// render.ts split (#13051): board, roadmap and item-detail views.
// render.ts re-exports every public name from here; callers keep importing "./render.ts".

import {
  type RoadmapGrain,
  allRefs,
  commentStream,
  executionStrip,
  monthTimeline,
  roadmapDayGrain,
  timeline as buildTimeline,
} from "./derive.ts";
import type { RoadmapReconciliation } from "./roadmap-reconcile.ts";
import type { RoadmapAnnotation } from "./roadmap-annotations.ts";
import type { ReleaseSnapshot } from "./releases.ts";
import { formatForecastSummary, type ForecastContract } from "./forecast-contract.ts";
import type { FlowAnalytics } from "./flow-analytics.ts";
import { flowChartsHtml } from "./flow-charts.ts";
import type { DashAction } from "./actions.ts";
import { applyBoardDonePolicy, DONE_STRIP_RENDER_LIMIT, selectVisibleDoneCards } from "./board-policy.ts";
import { summarizeWipLanes } from "./wip-limits.ts";
import type { CommentKindCounts } from "./comments.ts";
import type { ItemServiceLink } from "./service-links.ts";
import { activityCoverageHtml, gapsHtml, type ActivityCoverage, type GapRecord } from "./activity.ts";
import type { AuditEvent } from "./events.ts";
import {
  type BoardItem,
  type Comment,
  type DashComment,
  type GhItem,
  type ProjectPoolItem,
  type Provenance,
  type ProvenanceDetail,
  type RefKey,
  type RoadmapRow,
  type SessionRecord,
  isValidRef,
  refUrl,
} from "./model.ts";
import { canonicalStatusFor, ghItemDerivedStatus } from "./status-taxonomy.ts";
import type { ItemBrief } from "./brief.ts";
import { isEpicChildDone, uniqueProjectItemsByRef, type EpicRollup } from "./epics.ts";
import { spendCardHtml, unattributedSpendHtml, type SpendCardsView } from "./spend-cards.ts";
import type { AuthorIdentity } from "./registry.ts";

import {
  esc,
  fmtDate,
  COMMENT_SCRIPT,
  ROADMAP_ANNOTATION_SCRIPT,
  FRAG_WATCHDOG_SCRIPT,
  ACTION_SCRIPT,
  freshnessChips,
  layout,
  ghObjectRef,
  commentBadge,
  authorBadge,
  dashCommentHtml,
  type CommentFormOpts,
  authorOptionsHtml,
  commentFormHtml,
  actionBadgesHtml,
  statusRequestHtml,
  sessionActionsHtml,
  sessionRecordIsLive,
  itemBadge,
  stripHtml,
  briefPanelsForRefs,
} from "./render-core.ts";
import { renderSectionPrompt } from "./prompt-widget.ts";

// --- Board view -----------------------------------------------------------

/** The board's card grid alone (#1444 fragment payload). */
export function renderBoardBody(
  board: BoardItem[],
  items: Map<RefKey, GhItem>,
  counts?: Map<string, number | CommentKindCounts>,
  act?: {
    actionsByObject?: Map<string, DashAction[]>;
    poolByRef?: Map<RefKey, ProjectPoolItem>;
    identities?: AuthorIdentity[];
    briefs?: Map<RefKey, ItemBrief>;
  },
  renderOpts?: { showAllDone?: boolean },
): string {
  // #1146: finished work is not WIP — aged-out done items and the
  // retired ROADMAP Done-table class never reach the board, whatever
  // source assembled it (the Kaneo source filters too; see
  // board-policy.ts). Surviving done renders collapsed, below.
  board = applyBoardDonePolicy(board);
  const wipByLane = new Map(
    summarizeWipLanes(board, [...(act?.poolByRef?.values() ?? [])]).map((summary) => [summary.lane, summary]),
  );
  const cols = new Map<string, BoardItem[]>();
  for (const item of board) {
    const key = item.status === "done" ? "done" : item.status;
    const k = key === "to-do" ? "to-do" : key;
    cols.set(k, [...(cols.get(k) ?? []), item]);
  }
  for (const lane of wipByLane.keys()) {
    if (!cols.has(lane)) cols.set(lane, []);
  }
  const order = ["in-flight", "to-do", "in-progress", "in-review", "done"];
  const extra = [...cols.keys()].filter((k) => !order.includes(k));
  const bodies = [...order, ...extra]
    .filter((k) => cols.has(k))
    .map((k) => {
      const colItems = cols.get(k) ?? [];
      // #1179: the done strip renders only the most recent
      // DONE_STRIP_RENDER_LIMIT cards (newest first); the rest are
      // one click away (?done=all). Rendering all ~950 cost ~1 MB.
      let visibleItems = colItems;
      let doneNote = "";
      if (k === "done" && colItems.length > DONE_STRIP_RENDER_LIMIT) {
        visibleItems = selectVisibleDoneCards(colItems, { showAll: renderOpts?.showAllDone ?? false });
        if (renderOpts?.showAllDone) {
          doneNote = ` — showing all, newest first; <a href="/board">show fewer</a>`;
        } else {
          doneNote = ` — showing the ${DONE_STRIP_RENDER_LIMIT} most recent; <a href="/board?done=all">show all ${colItems.length}</a>`;
        }
      }
      const cards = visibleItems
        .map((item) => {
          const repo = item.repo ? `<span class="muted">${esc(item.repo)}</span><br>` : "";
          const cmt = counts ? commentBadge(item.id, counts) : "";
          const briefs = briefPanelsForRefs(
            item.links.map((link) => link.ref),
            act?.briefs,
          );
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
            return `<div class="card"><a href="/item/${encodeURIComponent(item.id)}"><span class="badge implementing">orchestrator · in flight</span> ${esc(item.title)}</a>${cmt}<br>${status}${repo}${goal}${blocked}${refs}${briefs}</div>`;
          }
          const src = item.source === "github" ? `<span class="badge open">gh-only</span> ` : "";
          // #1154: a card whose linked issue sits in the Project pool
          // carries the status-request control for that pool item.
          let statusCtl = "";
          if (act?.identities && act.poolByRef) {
            const pooled = item.links.map((l) => act.poolByRef!.get(l.ref)).find((p) => p !== undefined);
            if (pooled) {
              const objectRef = ghObjectRef(pooled.ref);
              statusCtl = `${actionBadgesHtml(act.actionsByObject?.get(objectRef))}${statusRequestHtml(objectRef, pooled.ref, pooled.status ?? null, { identities: act.identities })}`;
            }
          }
          const kaneo = item.kaneoUrl ? ` <a class="badge" href="${esc(item.kaneoUrl)}">Kaneo ↗</a>` : "";
          // #1198: the board column is Kaneo's status; when the linked
          // Project pool item's Status disagrees canonically, show BOTH
          // instead of silently picking the column.
          let statusClash = "";
          if (act?.poolByRef) {
            const pooled = item.links.map((l) => act.poolByRef!.get(l.ref)).find((p) => p !== undefined);
            if (pooled?.status) {
              const boardCanonical = canonicalStatusFor("kaneo", item.status);
              const projectCanonical = canonicalStatusFor("project", pooled.status);
              if (boardCanonical && projectCanonical && boardCanonical !== projectCanonical) {
                statusClash = ` <span class="badge stale" title="The Kaneo board column and the Project Status disagree in the canonical taxonomy (#1198) — both are shown, neither is preferred">⚠ board: ${esc(item.status)} · project: ${esc(pooled.status)}</span>`;
              }
            }
          }
          return `<div class="card"><a href="/item/${encodeURIComponent(item.id)}">${src}${esc(item.title)}</a>${cmt}<br>${repo}${kaneo}${statusClash}${stripHtml(items, item.links)}${statusCtl}${briefs}${renderSectionPrompt("board", item.id)}</div>`;
        })
        .join("");
      // The in-flight lane is dated by the orchestration section that
      // produced it (stamped on every synthetic entry as updatedAt).
      if (k === "done") {
        // Collapsed by default (#1146): a count + the last 7 days
        // behind an expander, never a full column of finished cards.
        return `<details class="donestrip"><summary><strong>Done recently (${colItems.length})</strong> — done in the last 7 days, collapsed by default; older done is aged off the board (GitHub is the record)${doneNote}</summary>${cards}</details>`;
      }
      const heading =
        k === "in-flight"
          ? `in flight — orchestrator · in flight · section ${fmtDate(colItems[0]?.updatedAt)} (${colItems.length}/${wipByLane.get(k)?.cap ?? 0})`
          : `${esc(k)} (${colItems.length}/${wipByLane.get(k)?.cap ?? 0})`;
      const summary = wipByLane.get(k);
      const scopeFlags = [
        ...(summary?.projects ?? [])
          .filter((scope) => scope.overCap)
          .map((scope) => `project ${scope.key} ${scope.count}/${scope.cap}`),
        ...(summary?.epics ?? [])
          .filter((scope) => scope.overCap)
          .map((scope) => `epic ${scope.key} ${scope.count}/${scope.cap}`),
      ];
      if (summary?.overCap) scopeFlags.unshift(`lane over cap by ${summary.count - summary.cap}`);
      const overCapBadges = scopeFlags
        .map((flag) => `<span class="badge blocked" role="status">${esc(flag)}</span>`)
        .join(" ");
      return `<div class="col"><h2>${heading} ${overCapBadges}</h2>${cards}</div>`;
    })
    .join("");
  return `<div class="board">${bodies}</div>`;
}

/** The board page (#1444): full server render. */
export function renderBoard(
  board: BoardItem[],
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  counts?: Map<string, number | CommentKindCounts>,
  detail?: ProvenanceDetail,
  act?: {
    actionsByObject?: Map<string, DashAction[]>;
    poolByRef?: Map<RefKey, ProjectPoolItem>;
    identities?: AuthorIdentity[];
    briefs?: Map<RefKey, ItemBrief>;
  },
  renderOpts?: { showAllDone?: boolean },
): string {
  return layout(
    "Cross-repo WIP board",
    `<h1>Cross-repo WIP board</h1><p class="muted">What is being worked on now is the <strong>in-flight</strong> lane: synthetic entries from the orchestrator's lined-up tasks (the fleet feed's orchestration section), not Kaneo cards. The other columns are the curated Kaneo board across the four repos, including its completion history — on their own they are a record of finished and in-review work, not a live WIP list. Each Kaneo card's execution strip is derived from its linked issues/PRs: implementing → review → merged.</p>${renderBoardBody(board, items, counts, act, renderOpts)}`,
    provenance,
    "/board",
    act?.identities ? ACTION_SCRIPT : "",
    detail ? freshnessChips(detail, ["board", "items", "orchestration"]) : "",
  );
}

/** The Datastar shell for / (#1444): heading + intro paint at once;
 *  the board itself streams in from /frag/board. */
export function renderBoardShell(provenance: Provenance, detail?: ProvenanceDetail, viewQuery = ""): string {
  const fragBoard = viewQuery ? `/frag/board?${viewQuery}` : "/frag/board";
  return layout(
    "Cross-repo WIP board",
    `<h1>Cross-repo WIP board</h1><p class="muted">What is being worked on now is the <strong>in-flight</strong> lane: synthetic entries from the orchestrator's lined-up tasks (the fleet feed's orchestration section), not Kaneo cards.</p><noscript><meta http-equiv="refresh" content="0; url=/board"></noscript><div id="board-main" data-frag data-init="@get('${fragBoard}')" data-on:dashrefresh__window="@get('${fragBoard}')"><div class="skel">Loading the board…</div></div>`,
    provenance,
    "/board",
    ACTION_SCRIPT + FRAG_WATCHDOG_SCRIPT,
    detail ? freshnessChips(detail, ["board", "items", "orchestration"]) : "",
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

/** One roadmap row's drill-down (#1168): its matched Project epic
 *  (absent when the row is unmatched) and the epic's pool children —
 *  the pool items whose epicRef names the epic (#2399). */
export interface RoadmapDrill {
  epic?: ProjectPoolItem;
  children: ProjectPoolItem[];
}

/** Build the per-row drill-down index from a reconciliation and the
 *  Project pool, keyed by the row's registry subject. Every row gets
 *  an entry, so an unmatched row is distinguishable from drill data
 *  never having been supplied. Callers pass no index at all when the
 *  reconciliation is unavailable — a failed source read must not
 *  masquerade as "no epic yet" (#1144's availability rule). */
export function buildRoadmapDrill(
  rows: RoadmapRow[],
  reconciliation: RoadmapReconciliation,
  pool: ProjectPoolItem[],
): Map<string, RoadmapDrill> {
  const epicByRow = new Map(reconciliation.matched.map((m) => [m.row.subject, m.epic] as const));
  const childrenByEpic = new Map<string, ProjectPoolItem[]>();
  for (const item of uniqueProjectItemsByRef(pool)) {
    if (!item.epicRef) continue;
    const list = childrenByEpic.get(item.epicRef) ?? [];
    list.push(item);
    childrenByEpic.set(item.epicRef, list);
  }
  return new Map(
    rows.map((row) => {
      const epic = epicByRow.get(row.subject);
      return [row.subject, { epic, children: epic ? (childrenByEpic.get(epic.ref) ?? []) : [] }] as const;
    }),
  );
}

/** The shared forecast contract (§4, #3984) as page lines: the same
 * one-line summaries the API serves, never a bare date. */
export function forecastSummaryHtml(contracts: ForecastContract[] | undefined): string {
  if (!contracts?.length) return "";
  const lines = contracts.map((c) => `<div class="forecast-line">${esc(formatForecastSummary(c))}</div>`).join("");
  return `<div class="forecast">${lines}</div>`;
}

function roadmapDrilldown(
  drill: RoadmapDrill,
  row: RoadmapRow,
  forecasts?: Map<RefKey, ForecastContract[]>,
  spend?: SpendCardsView | null,
): string {
  const parts: string[] = [];
  if (!drill.epic) {
    parts.push(`<p class="muted">No epic yet — no Project epic matches this roadmap row.</p>`);
  } else {
    const epic = drill.epic;
    parts.push(
      `<p>Epic: <a href="${esc(epic.issueUrl)}">${esc(epic.title)}</a> <span class="muted">${esc(epic.ref)} · ${esc(epic.state)}</span></p>`,
    );
    if (drill.children.length === 0) {
      parts.push(`<p class="muted">No child issues in the Project pool for this epic yet.</p>`);
    } else {
      const done = drill.children.filter(isEpicChildDone).length;
      const total = drill.children.length;
      const pct = Math.round((done / total) * 100);
      parts.push(
        `<p class="muted">Progress: ${done}/${total} child issues done</p><div class="progress"><progress value="${done}" max="${total}">${pct}%</progress></div>`,
      );
      parts.push(
        `<ul>${drill.children
          .map(
            (c) =>
              `<li><a href="${esc(c.issueUrl)}">${esc(c.title)}</a> <span class="badge ${esc(c.state)}">${esc(c.state)}</span> <span class="muted">${esc(c.ref)}</span></li>`,
          )
          .join("")}</ul>`,
      );
    }
  }
  if (drill.epic) {
    const fc = forecastSummaryHtml(forecasts?.get(drill.epic.ref));
    if (fc) parts.push(fc);
  }
  if (drill.epic) {
    const card = spend?.byScope.get(drill.epic.ref);
    if (card) parts.push(spendCardHtml(card));
  }
  if (row.notes.trim()) parts.push(`<p class="notes">${esc(row.notes)}</p>`);
  return `<details class="roadmap-drill"><summary>Epic &amp; issues</summary>${parts.join("")}</details>`;
}

function roadmapCard(
  row: RoadmapRow,
  items: Map<RefKey, GhItem>,
  counts?: Map<string, number | CommentKindCounts>,
  drill?: RoadmapDrill,
  annotations?: Map<string, RoadmapAnnotation[]>,
  briefs?: Map<RefKey, ItemBrief>,
  forecasts?: Map<RefKey, ForecastContract[]>,
  spend?: SpendCardsView | null,
  epicRollups?: Map<string, EpicRollup>,
  showRowNotes = false,
  promptPage?: string,
): string {
  const refs = allRefs([row]);
  const refBadges = refs
    .map((r) => items.get(r))
    .filter((x): x is GhItem => Boolean(x))
    .map(itemBadge)
    .join(" ");
  const cmt = counts ? commentBadge(row.subject, counts) : "";
  const grist = row.gristUrl ? ` <a class="badge" href="${esc(row.gristUrl)}">Grist ↗</a>` : "";
  const repoFile = row.fileUrl ? ` <a class="badge" href="${esc(row.fileUrl)}">Repo file ↗</a>` : "";
  const expansion = drill ? roadmapDrilldown(drill, row, forecasts, spend) : "";
  // The program timeline has no drill map; keep its Notes visible without
  // duplicating the same text when a card already has a drilldown.
  const rowNotes = showRowNotes && !drill && row.notes.trim() ? `<p class="notes">${esc(row.notes)}</p>` : "";
  const epicRollup = epicRollups?.get(row.subject);
  const rollupHtml = epicRollup ? epicRollupHtml(epicRollup) : "";
  const notes = annotations?.get(row.subject) ?? [];
  const annotationHtml = notes.length
    ? `<div class="roadmap-annotations">${notes.map(roadmapAnnotationHtml).join("")}</div>`
    : "";
  const briefPanels = briefPanelsForRefs(refs, briefs);
  const horizonWarning = row.horizonError
    ? ` <span class="badge horizon-invalid" title="${esc(row.horizonError)}">Invalid Horizon</span>`
    : "";
  // #1198: the row's status is Grist's; when the linked GitHub item's
  // derived lifecycle status (or its Project Status) disagrees
  // canonically, show BOTH instead of silently picking Grist's.
  let statusClash = "";
  const ghLinked = refs.map((r) => items.get(r)).find((x) => x !== undefined);
  if (ghLinked) {
    const gristCanonical = canonicalStatusFor("grist", row.status);
    const derived = ghItemDerivedStatus(ghLinked);
    if (gristCanonical && derived && derived !== gristCanonical) {
      statusClash = ` <span class="badge stale" title="The Grist Roadmap Status and the status derived from the linked GitHub item disagree in the canonical taxonomy (#1198) — both are shown, neither is preferred">⚠ grist: ${esc(row.status)} · derived: ${esc(derived)}</span>`;
    } else if (gristCanonical && ghLinked.projectStatus) {
      const projectCanonical = canonicalStatusFor("project", ghLinked.projectStatus);
      if (projectCanonical && projectCanonical !== gristCanonical) {
        statusClash = ` <span class="badge stale" title="The Grist Roadmap Status and the Project Status disagree in the canonical taxonomy (#1198) — both are shown, neither is preferred">⚠ grist: ${esc(row.status)} · project: ${esc(ghLinked.projectStatus)}</span>`;
      }
    }
  }
  return `<div class="card"><a href="/item/${encodeURIComponent(row.subject)}">${esc(row.title)}</a>${cmt}<br><span class="badge ${esc(row.status.toLowerCase().replace(/[^a-z]/g, "-"))}">${esc(row.status)}</span> <span class="muted">${esc(row.area)} · ${esc(row.horizon)}</span>${horizonWarning}${statusClash}${refBadges}${grist}${repoFile}${rollupHtml}${annotationHtml}${rowNotes}${expansion}${briefPanels}${promptPage ? renderSectionPrompt(promptPage, row.subject) : ""}</div>`;
}

function roadmapAnnotationHtml(annotation: RoadmapAnnotation): string {
  return `<div class="roadmap-annotation"><span class="muted">${esc(annotation.date)}</span> <span class="badge">${esc(annotation.kind)}</span> ${authorBadge(annotation.author)}<br>${esc(annotation.body)}</div>`;
}

export function timelineTable(
  tl: ReturnType<typeof buildTimeline>,
  items: Map<RefKey, GhItem>,
  counts?: Map<string, number | CommentKindCounts>,
  drill?: Map<string, RoadmapDrill>,
  annotations?: Map<string, RoadmapAnnotation[]>,
  briefs?: Map<RefKey, ItemBrief>,
  forecasts?: Map<RefKey, ForecastContract[]>,
  spend?: SpendCardsView | null,
  options?: { epicRollups?: Map<string, EpicRollup>; showRowNotes?: boolean; promptPage?: string },
): string {
  const head = `<tr><th scope="col">Theme</th>${tl.lanes.map((l) => `<th scope="col">${esc(l.label)}</th>`).join("")}</tr>`;
  const body = tl.themes
    .map((theme) => {
      const themeRows = [...(tl.cells.get(theme)?.values() ?? [])].flat();
      const done = themeRows.filter((row) => row.status.trim().toLowerCase() === "done").length;
      const cells = tl.lanes
        .map((lane) => {
          const rs = tl.cells.get(theme)?.get(lane.key) ?? [];
          return `<td class="tl-cell">${rs.map((r) => roadmapCard(r, items, counts, drill?.get(r.subject), annotations, briefs, forecasts, spend, options?.epicRollups, options?.showRowNotes, options?.promptPage)).join("")}</td>`;
        })
        .join("");
      return `<tr><th scope="row">${esc(theme)} <span class="muted theme-rollup">(${done}/${themeRows.length} done)</span></th>${cells}</tr>`;
    })
    .join("");
  return `<div class="tblwrap"><table class="table table-zebra"><caption>Roadmap timeline by theme</caption>${head}${body}</table></div>`;
}

export function renderRoadmap(
  rows: RoadmapRow[],
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  grain: RoadmapGrain = "quarter",
  counts?: Map<string, number | CommentKindCounts>,
  detail?: ProvenanceDetail,
  reconciliation?: RoadmapReconciliation,
  drill?: Map<string, RoadmapDrill>,
  annotations?: Map<string, RoadmapAnnotation[]>,
  viewQuery = "",
  briefs?: Map<RefKey, ItemBrief>,
  forecasts?: Map<RefKey, ForecastContract[]>,
  flowAnalytics?: FlowAnalytics,
  spend?: SpendCardsView | null,
  releases: ReleaseSnapshot[] = [],
  prioritiesView = false,
): string {
  // #1144: when the Grist rows and the Project's Kind=Epic pool items
  // disagree, say so on the page itself — drift the reader can see,
  // with the full lists published in /api/roadmap.json.
  const driftFlag =
    reconciliation?.availability === "unavailable"
      ? `<div class="flag">Roadmap reconciliation unavailable: ${esc(reconciliation.unavailableReason ?? "a roadmap source could not be read")} — no drift is asserted when a source read failed (#1144).</div>`
      : reconciliation && reconciliation.drift > 0
        ? `<div class="flag">Roadmap drift: ${reconciliation.unmatchedRows.length} Grist row${reconciliation.unmatchedRows.length === 1 ? "" : "s"} with no matching Project epic · ${reconciliation.unmatchedEpics.length} Project epic${reconciliation.unmatchedEpics.length === 1 ? "" : "s"} with no roadmap row (#1144 — the two roadmap sources disagree; both lists are in <a href="/api/roadmap.json">/api/roadmap.json</a>).</div>`
        : "";
  const chips = `${GRAINS.map(
    ([g, label]) =>
      `<a href="/roadmap?h=${g}${viewQuery ? `&${viewQuery}` : ""}"${g === grain ? ' class="active"' : ""}>${label}</a>`,
  ).join("")} <a href="/roadmap?view=priorities"${prioritiesView ? ' class="active"' : ""}>Priorities</a>`;
  const invalidHorizons = rows.filter((row) => row.horizonError);
  const horizonFlag = invalidHorizons.length
    ? `<div class="flag">${invalidHorizons.length} roadmap row${invalidHorizons.length === 1 ? " has" : "s have"} an invalid Horizon value. Original values are preserved and marked on their cards; correct them in Grist using the documented Horizon grammar.</div>`
    : "";
  let body: string;
  if (prioritiesView) {
    const ordered = [...rows].sort((a, b) => {
      const rank = (row: RoadmapRow) => {
        const match = /#(\d+)$/.exec(row.priority ?? "");
        return match ? Number(match[1]) : Number.MAX_SAFE_INTEGER;
      };
      return rank(a) - rank(b) || a.title.localeCompare(b.title);
    });
    body = ordered.length
      ? ordered
          .map((row) => {
            const priority = row.priority
              ? `<span class="badge src">Priority <a href="${esc(refUrl(row.priority))}">${esc(row.priority)}</a></span>`
              : '<span class="badge stale">No priority link</span>';
            const after = row.afterRefs?.length
              ? `<span class="muted">After: ${row.afterRefs.map((ref) => `<a href="${esc(refUrl(ref))}">${esc(ref)}</a>`).join(", ")}</span>`
              : '<span class="muted">No predecessor recorded</span>';
            const release = row.releaseRef
              ? `<span class="badge src">Release <a href="${esc(refUrl(row.releaseRef))}">${esc(row.releaseRef)}</a></span>`
              : '<span class="muted">No release linked</span>';
            return `<div class="card priority-row"><strong><a href="/item/${encodeURIComponent(row.subject)}">${esc(row.title)}</a></strong><br>${priority} ${release}<br>${after}<br><span class="muted">${esc(row.area)} · ${esc(row.horizon)} · ${esc(row.status)}</span></div>`;
          })
          .join("")
      : '<p class="muted">No priority rows in the current roadmap source.</p>';
  } else if (grain === "quarter") {
    body = timelineTable(buildTimeline(rows), items, counts, drill, annotations, briefs, forecasts, spend, {
      promptPage: "roadmap",
    });
  } else if (grain === "month") {
    body = timelineTable(monthTimeline(rows), items, counts, drill, annotations, briefs, forecasts, spend, {
      promptPage: "roadmap",
    });
  } else {
    const dg = roadmapDayGrain(rows, grain);
    const byTheme = new Map<string, RoadmapRow[]>();
    for (const row of rows) {
      const theme = row.area || "Unthemed";
      byTheme.set(theme, [...(byTheme.get(theme) ?? []), row]);
    }
    body = byTheme.size
      ? [...byTheme.entries()]
          .map(([theme, themeRows]) => {
            const done = themeRows.filter((row) => row.status.trim().toLowerCase() === "done").length;
            const cards = (laneRows: RoadmapRow[]) =>
              laneRows
                .map((row) =>
                  roadmapCard(
                    row,
                    items,
                    counts,
                    drill?.get(row.subject),
                    annotations,
                    briefs,
                    forecasts,
                    spend,
                    undefined,
                    false,
                    "roadmap",
                  ),
                )
                .join("");
            const lanes = dg.lanes
              .map((lane) => {
                const laneRows = lane.rows.filter((row) => (row.area || "Unthemed") === theme);
                return laneRows.length ? `<h3>${esc(lane.label)}</h3>${cards(laneRows)}` : "";
              })
              .join("");
            const outsideRows = dg.outsideRows.filter((row) => (row.area || "Unthemed") === theme);
            const coarseRows = dg.coarse.filter((row) => (row.area || "Unthemed") === theme);
            const outside = outsideRows.length
              ? `<h3>Outside this window (${outsideRows.length})</h3>${cards(outsideRows)}`
              : "";
            const coarse = coarseRows.length
              ? `<h3>Undated or coarse horizons (${coarseRows.length})</h3>${cards(coarseRows)}`
              : "";
            return `<section class="roadmap-theme"><h2>${esc(theme)} <span class="muted theme-rollup">(${done}/${themeRows.length} done)</span></h2>${lanes}${outside}${coarse}</section>`;
          })
          .join("")
      : '<p class="muted">No roadmap rows.</p>';
  }
  const releaseCards = releases
    .filter((release) => release.kind === "release")
    .map((release) => {
      const counts = release.counts;
      const carried = release.carriedOverEpics.length
        ? `<p>Carried-over epics: ${release.carriedOverEpics.map((ref) => `<a href="${esc(refUrl(ref))}">${esc(ref)}</a>`).join(", ")}</p>`
        : '<p class="muted">No carried-over epics.</p>';
      return `<article class="card release-card"><h3>${esc(release.milestone.title)}</h3><p>Date: <strong>${esc(release.releaseDate ?? "date not set")}</strong></p><p>Issues: ${counts.issues.closed}/${counts.issues.total} closed · Designs: ${counts.designs.closed}/${counts.designs.total} closed · Epics: ${counts.epics.closed}/${counts.epics.total} closed</p>${carried}<p class="muted">${esc(release.descriptionLine)}</p></article>`;
    })
    .join("");
  const releasesHtml = `<section class="release-snapshots"><h2>Releases</h2>${releaseCards || '<p class="muted">No release close-out snapshots are available.</p>'}<p><a href="/api/releases.json">Releases JSON</a></p></section>`;
  return layout(
    "Roadmap",
    `<h1>Roadmap</h1><p class="muted">Grist's <em>Program roadmap</em> table at every grain — one data model, switchable horizon. Rows with a calendar date land on their day; quarter-labelled rows stay at quarter grain. Every row remains visible at every grain.</p><div class="chips">${chips}</div>${driftFlag}${horizonFlag}${body}${releasesHtml}${spend?.unattributed ? unattributedSpendHtml(spend.unattributed) : ""}${flowAnalytics ? flowChartsHtml(flowAnalytics) : ""}`,
    provenance,
    "/roadmap",
    "",
    detail ? freshnessChips(detail, ["roadmap", "items"]) : "",
  );
}

// --- Item drill-down -------------------------------------------------------

export interface ItemExtras {
  /** Dashboard comments (#906 §2) across this item's object refs. */
  dashComments: DashComment[];
  /** Present only on roadmap rows; undefined on ordinary board items. */
  roadmapAnnotations?: RoadmapAnnotation[];
  /** The object ref the inline form posts to (the item's subject). */
  formRef: string;
  formOpts: CommentFormOpts;
  /** Registry sessions loosely linked to this item's refs (#906 §3). */
  sessions: SessionRecord[];
  /** Dashboard actions across this item's object refs (#1152/#1154). */
  actions?: DashAction[];
  /** Set when one of the item's refs is in the Project pool (#1154). */
  statusRequest?: { objectRef: string; targetRef: RefKey; currentStatus: string | null };
  /** Outbound links to the live services this item lives in (#1136). */
  serviceLinks?: ItemServiceLink[];
  /** Present when this item's Project-pool entry is an epic. */
  epicRollup?: EpicRollup;
  /** Honest chain summary derived from the live Project pool. */
  genealogySentence?: string;
  /** Project-pool briefs for linked refs (#1279). */
  briefs?: Map<RefKey, ItemBrief>;
  /** Real ready-for-agent dispatch state for explicitly labelled linked issues. */
  dispatchStates?: { ref: RefKey; state: string; provenance: { source: string; observedAt: string } }[];
  /** The shared audit stream, scoped to the item's canonical refs. */
  activity?: { events: AuditEvent[]; globalHref: string; gaps: GapRecord[]; coverage?: readonly ActivityCoverage[] };
}

function roadmapAnnotationFormHtml(objectRef: string, identities: AuthorIdentity[]): string {
  return `<form class="raform" data-object="${esc(objectRef)}">
<label>Annotation kind</label><select name="kind"><option value="decision">Decision</option><option value="outcome">Outcome</option><option value="context" selected>Context</option></select>
<label>Date</label><input type="date" name="date" required>
<label>Author</label><select name="author" data-author-picker>${authorOptionsHtml(identities)}</select>
<label>Annotation</label><textarea name="body" rows="3" required placeholder="Why did this change, slip, or get dropped?"></textarea>
<button type="submit">Add annotation</button><div class="err"></div></form>`;
}

export function epicRollupHtml(rollup: EpicRollup): string {
  const statusMix = rollup.statusMix.length
    ? rollup.statusMix.map(({ status, count }) => `${esc(status)} ${count}`).join(" · ")
    : "none";
  const children = rollup.total
    ? `Children: <strong>${rollup.done}/${rollup.total} done</strong>`
    : `Children: <strong>0 resolved</strong> · completion unknown`;
  return `<p class="muted epic-rollup"><strong>Epic rollup</strong> — ${children} · Status mix: ${statusMix} · Blocked: ${rollup.blocked}</p>`;
}

export function eventTableHtml(events: readonly AuditEvent[]): string {
  const rows = events
    .map((event) => {
      const source = event.source.link
        ? `<a href="${esc(event.source.link)}" rel="noopener">${esc(event.source.identity)}</a>`
        : esc(event.source.identity);
      const delay = Date.parse(event.ingestedAt) - Date.parse(event.at);
      const ingested =
        Number.isFinite(delay) && delay > 0 ? `<span class="muted">(+${Math.round(delay / 1000)}s ingest)</span>` : "";
      return `<tr><td>${esc(event.at)} ${ingested}</td><td>${esc(event.actor)} <span class="muted">(${esc(event.attribution.confidence)})</span></td><td>${esc(event.kind)}</td><td><code>${esc(event.ref)}</code></td><td>${esc(event.summary)}</td><td>${source}</td></tr>`;
    })
    .join("\n");
  return `<div class="tblwrap"><table class="activity-events"><caption>Shared activity events</caption><thead><tr><th scope="col">When</th><th scope="col">Actor</th><th scope="col">Kind</th><th scope="col">Ref</th><th scope="col">Summary</th><th scope="col">Source</th></tr></thead><tbody>${rows || '<tr><td colspan="6" class="muted">No matching events; this does not establish complete coverage.</td></tr>'}</tbody></table></div>`;
}

/** #1500: the honest empty-section wording, keyed off the provenance
 * strings the renderers already receive — no new data plumbing. The
 * comment stream is fixture-sourced while the projector is off (#1199),
 * so a bare "no comments" would claim a fact about the work when it is
 * really a fact about the dashboard's resolution path. */
export const FIXTURE_COMMENTS_CAUSE = "the comment stream is fixture-sourced while the projector is off (#1199)";

export function commentsFixtureSourced(provenance: Provenance): boolean {
  return provenance.comments.startsWith("fixtures");
}

/** #14065 slice 3: live mode withholds fixture comments when the
 * projector is off or unreachable, so the stream is plainly empty. */
export const UNAVAILABLE_COMMENTS_CAUSE =
  "the comment stream is unavailable while the projector is off or unreachable (#14065)";

export function commentsUnavailable(provenance: Provenance): boolean {
  return provenance.comments.startsWith("unavailable");
}

/** Why an empty comment stream may not mean "no discussion", or null. */
export function commentsMissingCause(provenance: Provenance): string | null {
  if (commentsFixtureSourced(provenance)) return FIXTURE_COMMENTS_CAUSE;
  if (commentsUnavailable(provenance)) return UNAVAILABLE_COMMENTS_CAUSE;
  return null;
}

export function fixtureCommentsBannerHtml(provenance: Provenance): string {
  if (commentsUnavailable(provenance)) {
    return `<div class="banner">The comment stream is unavailable while the projector is off or unreachable (#14065) — GitHub comments are not shown, and an empty stream does not mean the work has no discussion.</div>`;
  }
  return commentsFixtureSourced(provenance)
    ? `<div class="banner">The comment stream is fixture-sourced while the projector is off (#1199) — GitHub comments may be missing or stale, and an empty stream does not mean the work has no discussion.</div>`
    : "";
}

export function renderItem(
  title: string,
  subject: string,
  links: BoardItem["links"],
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
  // #1500: when some linked refs did not resolve, the Linked header
  // carries the tally and the cause; when every ref resolves it stays
  // plain. The fixture cause is keyed off the items provenance string
  // only (the partial-fixtures shape from sources/live.ts, or a fully
  // fixture-sourced items facet).
  const linkedResolved = all.length;
  const linkedHeader =
    links.length > 0 && linkedResolved < links.length
      ? `Linked issues / PRs — ${links.length} linked · ${linkedResolved} resolved — ${
          /\+ fixtures for \d+ ref\(s\) outside the pool/.test(provenance.items) ||
          provenance.items.startsWith("fixtures")
            ? "resolution fell back to fixtures (projector off, #1199)"
            : /\+ unavailable for \d+ ref\(s\) outside the pool/.test(provenance.items) ||
                provenance.items.startsWith("unavailable")
              ? "the projector is off or unreachable, so refs outside the project pool are unresolved (#14065)"
              : "some linked refs could not be resolved from the current sources"
        }`
      : "Linked issues / PRs";
  const commentsBanner = fixtureCommentsBannerHtml(provenance);
  const missingCause = commentsMissingCause(provenance);
  const emptyCommentRow = missingCause
    ? `<tr><td colspan="4" class="muted">no comments loaded — ${missingCause}</td></tr>`
    : '<tr><td colspan="4" class="muted">no comments</td></tr>';
  const dashSection = extras
    ? `<h2>Dashboard comments</h2>${
        extras.dashComments.length
          ? extras.dashComments.map((c) => dashCommentHtml(c, true)).join("")
          : `<p class="muted">No dashboard comments yet — notes are read when this is picked up; messages are triaged by the orchestrator.</p>`
      }${commentFormHtml(extras.formRef, extras.formOpts)}`
    : "";
  const roadmapAnnotationSection =
    extras?.roadmapAnnotations !== undefined
      ? `<section class="roadmap-annotation-section"><h2>Roadmap annotations</h2>${extras.roadmapAnnotations.length ? extras.roadmapAnnotations.map(roadmapAnnotationHtml).join("") : '<p class="muted">No roadmap annotations yet.</p>'}${roadmapAnnotationFormHtml(extras.formRef, extras.formOpts.identities)}</section>`
      : "";
  const sessionSection =
    extras && extras.sessions.length
      ? `<h2>Sessions that worked this</h2><div class="item-session-records">${extras.sessions.map((r) => sessionRecordLine(r)).join("")}</div>`
      : "";
  const statusSection =
    extras?.statusRequest && extras.formOpts
      ? `<h2>Request status change</h2>${actionBadgesHtml(extras.actions)}${statusRequestHtml(extras.statusRequest.objectRef, extras.statusRequest.targetRef, extras.statusRequest.currentStatus, { identities: extras.formOpts.identities })}`
      : "";
  const svcLinks = extras?.serviceLinks?.length
    ? `<p>Live services: ${extras.serviceLinks.map((s) => `<a href="${esc(s.url)}">${esc(s.label)} ↗</a>`).join(" · ")}</p>`
    : "";
  const epicRollup = extras?.epicRollup ? epicRollupHtml(extras.epicRollup) : "";
  const dispatchSection = extras?.dispatchStates?.length
    ? `<section><h2>ready-for-agent dispatch</h2><ul>${extras.dispatchStates
        .map(
          (entry) =>
            `<li><a href="https://github.com/${esc(entry.ref.replace("#", "/issues/"))}">${esc(entry.ref)}</a>: <strong>${esc(entry.state)}</strong> <span class="muted">· ${esc(entry.provenance.source)} checked ${esc(entry.provenance.observedAt)}</span></li>`,
        )
        .join("")}</ul></section>`
    : "";
  const activitySection = extras?.activity
    ? `<h2>Activity stream</h2><p><a href="${esc(extras.activity.globalHref)}">View these events in the global river</a></p><p class="muted"><strong>Freshness: unknown.</strong> The ingestion checkpoint records deliveries, not successful source polls. An empty event list does not establish complete coverage. Known gaps and documented source coverage are shown below.</p>${eventTableHtml(extras.activity.events)}<h3>Known activity gaps</h3><p class="muted">Gap records are source-scoped and do not currently identify affected item refs, so this is the global known-gap list.</p><div class="tblwrap">${gapsHtml(extras.activity.gaps)}</div><h3>Activity source coverage</h3><div class="tblwrap">${activityCoverageHtml(extras.activity.coverage)}</div>`
    : "";
  return layout(
    title,
    `<h1>${esc(title)}</h1><p class="muted">subject: <code>${esc(subject)}</code></p>${extras?.genealogySentence ? `<p class="genealogy">${esc(extras.genealogySentence)}</p>` : ""}${epicRollup}${svcLinks}${dispatchSection}${stripHtml(items, links)}
${
  extras?.briefs
    ? briefPanelsForRefs(
        links.map((link) => link.ref),
        extras.briefs,
      )
    : ""
}
<h2>${linkedHeader}</h2><div class="tblwrap"><table class="table table-zebra"><caption>Linked issues and pull requests</caption><tr><th scope="col">Ref</th><th scope="col">Title</th><th scope="col">Labels</th><th scope="col">Updated</th></tr>${refRows || '<tr><td colspan="4" class="muted">none resolved</td></tr>'}</table></div>
 <h2>Activity stream comments</h2>${commentsBanner}<div class="tblwrap"><table class="table table-zebra"><caption>Comments</caption><tr><th scope="col">When</th><th scope="col">Kind</th><th scope="col">Author</th><th scope="col">Body</th></tr>${commentRows || emptyCommentRow}</table></div>
 ${activitySection}${sessionSection}${statusSection}${roadmapAnnotationSection}${dashSection}`,
    provenance,
    "/board",
    extras
      ? COMMENT_SCRIPT +
          (extras.roadmapAnnotations !== undefined ? ROADMAP_ANNOTATION_SCRIPT : "") +
          (extras.statusRequest ? ACTION_SCRIPT : "")
      : "",
  );
}

/** One registry record as a compact line (item page, sessions page). */
export function sessionRecordLine(
  r: SessionRecord,
  act?: { actions?: DashAction[]; identities?: AuthorIdentity[] },
  options?: { showActionsLink?: boolean },
): string {
  const tokens = r.tokensTotal != null ? ` · ${r.tokensTotal.toLocaleString()} tok` : "";
  const parent = r.parentId
    ? ` · sub of <a href="/sessions?id=${encodeURIComponent(r.parentId)}#${encodeURIComponent(r.parentId)}">${esc(r.parentId)}</a>`
    : "";
  const renderedRefs = r.taskRefs.map((ref) => {
    // #6243 (ref contract rule 4, #6242): a malformed ref may still
    // exist in an older or ungated in-memory record. Keep the session
    // visible and render the invalid ref as plain text — the same
    // treatment tierItemCard (#6149) and poolRefLink give it — never
    // a throw, and never silently omitted.
    return isValidRef(ref) ? `<a href="${esc(refUrl(ref))}">${esc(ref)}</a>` : esc(ref);
  });
  const refs = renderedRefs.length ? ` · ${renderedRefs.join(" ")}` : "";
  // #1156: live records carry the session action set on the merged
  // registry identity; historical records render no controls.
  const badges = actionBadgesHtml(act?.actions);
  const live = sessionRecordIsLive(r);
  const controls = act?.identities && live ? sessionActionsHtml(r.id, r.source, { identities: act.identities }) : "";
  const actionsLink =
    options?.showActionsLink && live
      ? ` <a class="session-actions-link" href="/sessions?id=${encodeURIComponent(r.id)}#${encodeURIComponent(r.id)}">manage →</a>`
      : "";
  const provenance =
    r.provenance === "verified"
      ? `<span class="badge">verified identity</span>`
      : `<span class="badge">asserted-legacy — identity is a claim</span>`;
  return `<div class="card session-record-line" id="${esc(r.id)}"><strong>${esc(r.label)}</strong> <span class="badge src">${esc(r.source)}</span> <span class="badge agent">${esc(r.agentLabel)}</span> ${provenance}${badges}<br><span class="muted session-record-meta">${esc(r.id)} · ${esc(r.status)}${r.model ? ` · ${esc(r.model)}` : ""} · started ${fmtDate(r.startedAt)} · last seen ${fmtDate(r.lastSeenAt)}${tokens}${parent}${refs}${actionsLink}</span>${controls}</div>`;
}
