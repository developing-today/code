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
import type { RoadmapReconciliation } from "./roadmap-reconcile.ts";
import { diffCoverage, type CoverageSnapshotFile } from "./coverage.ts";
import { registryQueryString, type RegistryView } from "./registry-view.ts";
import { latestInFamily, newClientToken, type DashAction } from "./actions.ts";
import { applyBoardDonePolicy } from "./board-policy.ts";
import { cardState, pendingVerb, type CardState } from "./card-state.ts";
import {
  DELEGATION_DEFAULT_OBJECT_REF,
  NEEDS_ORDER_OBJECT_REF,
  effectiveStandingAuto,
  proposalExpired,
  proposalIdentity,
  type NeedsOrder,
} from "./tier-items.ts";
import { githubCommentUrl } from "./comments.ts";
import { serviceHomeLinks, type ItemServiceLink } from "./service-links.ts";
import { aaCaiPoints, aaPoints, summarizeAa } from "./aa.ts";
import { summarizeDns } from "./dns.ts";
import { ageMsText, feedAgeText } from "./feed.ts";
import type { HealthView } from "./health.ts";
import {
  type AaCaiRow,
  type AaConfig,
  type AaFeedSection,
  type AaRanking,
  type AaSection,
  type AaTestRun,
  type BoardItem,
  type ChargeRow,
  type Comment,
  type DashAuthor,
  type DashComment,
  type DelegationMode,
  type DnsEntry,
  type DnsSection,
  type GhItem,
  type OrchestrationSection,
  type PaygLine,
  type ProjectPoolItem,
  type Provenance,
  type ProvenanceDetail,
  type QuotaState,
  type QuotaView,
  type RefKey,
  type RoadmapRow,
  type SaasService,
  type SessionRecord,
  type SnapshotRecord,
  type SourceCoverage,
  type TieredEntry,
  type TierItem,
  type TierItemQuestion,
  type UsageMetric,
  type UsageView,
  type WhyUserClass,
  SAAS_GROUP_LABELS,
  SESSION_SOURCE_LABELS,
  countedEstimate,
  refUrl,
} from "./model.ts";
import { PAGES, groupedPages, manifestRouteFor, pageByRoute } from "./pages.ts";
import type { ProgramData, ProgramEntry, ProgramLive, TrackedIssue } from "./program.ts";
import type { ProjectDetail, ProjectSummary } from "./project-config.ts";
import { derivedProjectState, projectDisagreement, PROJECT_URL } from "./projects.ts";
import { effortWeight, handsLine, summarizeWeights, weightedLine } from "./weights.ts";
import { blockedSummary, depthLayout, hasGraphEdges } from "./graph.ts";
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
nav { display: flex; flex-wrap: wrap; align-items: center; gap: 2px 12px; padding: 8px 16px; border-bottom: 1px solid #30363d; background: #161b22; position: sticky; top: 0; }
.navgroup { display: flex; flex-wrap: wrap; align-items: center; gap: 2px; }
.navgroup + .navgroup { border-left: 1px solid #30363d; padding-left: 12px; }
.navlabel { color: #8b949e; font-size: 10.5px; font-weight: 600; text-transform: uppercase; letter-spacing: .05em; margin-right: 4px; }
nav a { color: #58a6ff; text-decoration: none; padding: 5px 9px; border-radius: 6px; white-space: nowrap; }
nav a.active, nav a:hover { background: #21262d; }
nav a.active { background: #1f6feb33; color: #fff; box-shadow: inset 0 -2px 0 #1f6feb; }
.navgroup[data-current="true"] .navlabel { color: #58a6ff; }
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
.badge.acked { background: #2ea04333; border-color: #2ea043; color: #3fb950; }
.badge.unverified { background: transparent; border-style: dashed; opacity: 0.75; }
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
caption { caption-side: top; text-align: left; font-weight: 600; font-size: 12.5px; padding: 6px 8px; color: #8b949e; }
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
.fchip { border: 1px solid #30363d; border-radius: 999px; padding: 5px 12px; color: #e6edf3; background: #161b22; font-size: 12.5px; }
.fchip.live { border-color: #2ea04388; }
.fchip.stale { border-color: #d29922; color: #e3b341; }
.fchip.invalid { border-color: #f85149; color: #f85149; }
.fchip.absent { border-color: #d29922; color: #e3b341; }
.dot { display: inline-block; width: 9px; height: 9px; border-radius: 50%; margin-right: 6px; background: #8b949e; vertical-align: baseline; }
.dot.up, .dot.self { background: #3fb950; }
.dot.down, .dot.invalid { background: #f85149; }
.dot.stale, .dot.absent { background: #d29922; }
.dot.off, .dot.unknown { background: #8b949e; }
.dot.fixtures { background: #bc8cff; }
.srcdetail { margin: 6px 0; }
.srcdetail summary { cursor: pointer; }
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
.actbox { border: 1px dashed #30363d; border-radius: 8px; padding: 8px 10px; margin: 10px 0 2px; background: #0d1117; }
.actbox fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
.actbox .ahead { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin-bottom: 6px; }
.actbox .ahead label { font-size: 12px; color: #8b949e; }
.actbox select, .aform select, .aform input[type="text"], .aform input[type="datetime-local"] { background: #161b22; color: #e6edf3; border: 1px solid #30363d; border-radius: 6px; padding: 7px 9px; font: inherit; min-height: 44px; }
.aform { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 6px 0; }
.aform input[type="text"] { flex: 1 1 180px; }
.aform button { background: #21262d; color: #e6edf3; border: 1px solid #30363d; border-radius: 6px; padding: 8px 14px; font: inherit; cursor: pointer; min-height: 44px; }
.aform button.primary { background: #1f6feb; border-color: #1f6feb; color: #fff; }
.aform button.danger { border-color: #f85149; color: #f85149; }
.aform button:disabled, .actbox fieldset:disabled button { opacity: .45; cursor: not-allowed; }
.aform .confirm { display: inline-flex; gap: 5px; align-items: center; font-size: 13px; min-height: 44px; }
.aform .err { color: #f85149; font-size: 12px; flex-basis: 100%; }
.aform .why { color: #8b949e; font-size: 12px; flex-basis: 100%; }
.badge.act-pending { background: #d2992233; border-color: #d29922; color: #e3b341; }
.badge.act-applied { background: #2ea04333; border-color: #2ea043; color: #3fb950; }
.badge.act-rejected { background: #f8514933; border-color: #f85149; color: #f85149; }
.badge.act-snoozed { background: #bc8cff33; border-color: #bc8cff; color: #bc8cff; }
.badge.whyuser { background: #d2992233; border-color: #d29922; color: #e3b341; }
.badge.whyuser.access, .badge.whyuser.policy { background: #f8514933; border-color: #f85149; color: #f85149; }
.propbox { border: 1px solid #2ea043; background: #2ea04311; border-radius: 8px; padding: 8px 10px; margin: 8px 0; }
.propbox.expired { border-color: #8b949e; background: #8b949e11; }
.unablebox { border: 1px solid #f85149; background: #f8514911; border-radius: 8px; padding: 8px 10px; margin: 8px 0; }
.qblock { border-top: 1px solid #21262d; padding: 6px 0 2px; margin-top: 6px; }
.qblock .qopts { display: flex; flex-wrap: wrap; gap: 6px 14px; margin: 4px 0; }
.qblock label { display: inline-flex; gap: 5px; align-items: center; font-size: 13px; min-height: 32px; }
.optform .effect { color: #8b949e; font-size: 12px; flex-basis: 100%; }
nav select { background: #161b22; color: #e6edf3; border: 1px solid #30363d; border-radius: 6px; padding: 6px 8px; font: inherit; margin-left: auto; max-width: 45vw; }
.foot { margin-top: 40px; border-top: 1px solid #30363d; padding-top: 10px; color: #8b949e; font-size: 12px; }
.card.stale { opacity: .55; }
.card.acked { opacity: .72; }
.card .sentline { color: #3fb950; font-size: 12.5px; margin: 6px 0 2px; }
.card.tombstone { opacity: .6; }
details.carddetail { margin: 8px 0 2px; border-top: 1px solid #21262d; padding-top: 6px; }
details.carddetail summary { cursor: pointer; color: #58a6ff; font-size: 12.5px; padding: 4px 0; }
details.carddetail dl { display: grid; grid-template-columns: 110px 1fr; gap: 2px 10px; margin: 6px 0; font-size: 12.5px; }
details.carddetail dt { color: #8b949e; }
details.carddetail dd { margin: 0; overflow-wrap: anywhere; }
.histline { font-size: 12.5px; margin: 3px 0; }
.quickbar { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin: 6px 0; }
.quickbar .qlabel { font-size: 12px; color: #8b949e; flex-basis: 100%; }
.qchip { background: #21262d; color: #e6edf3; border: 1px solid #30363d; border-radius: 999px; padding: 6px 12px; font: inherit; font-size: 12.5px; cursor: pointer; min-height: 36px; }
.qchip.primary { background: #1f6feb; border-color: #1f6feb; color: #fff; }
.qchip:disabled { opacity: .45; cursor: not-allowed; }
.aform.qform { display: inline-flex; margin: 0; }
.cbox { border-top: 1px solid #21262d; margin-top: 8px; padding-top: 6px; }
.cbox .cform { border: 0; padding: 4px 0 0; margin: 0; background: none; }
.cbox summary { cursor: pointer; color: #58a6ff; font-size: 12.5px; padding: 4px 0; }
.qblock .qanswered { color: #3fb950; font-size: 12px; }
.qblock .qremaining { color: #e3b341; font-size: 12px; }
.skel { border: 1px dashed #30363d; border-radius: 8px; padding: 14px 12px; margin: 8px 0; color: #8b949e; background: #0d1117; }
.orderctl { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 8px 0; font-size: 12.5px; color: #8b949e; }
.orderctl .aform { margin: 0; }
.stripcard .restore-note { font-size: 12.5px; }
.badge.stale { background: #d2992233; border-color: #d29922; color: #e3b341; }
.badge.conflict { background: #bc8cff33; border-color: #bc8cff; color: #bc8cff; }
.flag { border: 1px solid #f85149; background: #f8514911; border-radius: 8px; padding: 8px 12px; margin: 8px 0; }
.flag.payg { border-color: #d29922; background: #d2992211; }
.svcgrid { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 12px; align-items: start; }
.chart { background: #0d1117; border: 1px solid #30363d; border-radius: 8px; padding: 8px; margin: 8px 0; overflow-x: auto; }
.xtabs { display: flex; gap: 6px; margin: 12px 0 4px; border-bottom: 1px solid #30363d; }
.xtab { background: none; border: 1px solid transparent; border-bottom: 0; color: #8b949e; font: inherit; font-weight: 600; padding: 8px 14px; border-radius: 8px 8px 0 0; cursor: pointer; }
.xtab:hover { color: #e6edf3; }
.xtab.active { color: #e6edf3; background: #161b22; border-color: #30363d; }
.xpl-filters { display: flex; flex-wrap: wrap; gap: 8px 14px; align-items: center; }
.xpl-filters input[type="search"], .xpl-filters select { background: #161b22; color: #e6edf3; border: 1px solid #30363d; border-radius: 6px; padding: 6px 8px; font: inherit; }
.xpl-filters input[type="search"] { min-width: 220px; flex: 1; }
.xchip { display: inline-flex; align-items: center; gap: 5px; font-size: 11.5px; padding: 3px 9px; border-radius: 999px; border: 1px solid #30363d; background: #161b22; color: #e6edf3; cursor: pointer; font: inherit; }
.xchip .dot { width: 9px; height: 9px; border-radius: 50%; flex: none; }
.xchip.off { opacity: .38; text-decoration: line-through; }
.xp-seg { display: inline-flex; border: 1px solid #30363d; border-radius: 8px; overflow: hidden; }
.xp-seg button { background: #161b22; color: #8b949e; border: 0; font: inherit; font-weight: 600; padding: 6px 12px; cursor: pointer; }
.xp-seg button.active { background: #1f6feb; color: #fff; }
.xpl-charts { display: grid; grid-template-columns: 1fr; gap: 4px; }
.xpl-charts .chart svg { width: 100%; height: auto; display: block; }
.xpl-charts .chart.primary { border-color: #1f6feb; }
circle.xpt { cursor: pointer; }
.xpl-table td.num, .xpl-table th.num { text-align: right; }
.xpl-table tr { cursor: pointer; }
.xpl-table tr:hover td { background: #161b22; }
.xpl-detail { display: none; }
body.xp-plan .xpl-costcol { display: none; }
.xpl-ctl { display: flex; flex-wrap: wrap; gap: 8px 14px; align-items: center; font-size: 12px; margin: 2px 0 4px; }
.xpl-ctl button { border: 1px solid #30363d; background: #161b22; color: #e6edf3; border-radius: 6px; padding: 3px 10px; cursor: pointer; font: inherit; font-size: 11.5px; }
.xpl-ctl button.active { background: #1f6feb; color: #fff; border-color: #1f6feb; }
.xpl-ctl select { background: #161b22; color: #e6edf3; border: 1px solid #30363d; border-radius: 6px; padding: 4px 6px; font: inherit; font-size: 12px; }
.chart h3 { display: inline-block; margin: 2px 10px 2px 0; font-size: 13.5px; }
.hchip { display: inline-flex; align-items: center; gap: 6px; font-size: 11.5px; padding: 3px 6px 3px 9px; border-radius: 6px; background: #21262d; border: 1px solid #30363d; margin: 0 5px 5px 0; }
.hchip button { border: none; background: none; cursor: pointer; font-size: 13px; line-height: 1; color: #e6edf3; padding: 2px 4px; }
.xpl-ctl-btn { margin-top: 6px; border: 1px solid #30363d; background: #1f6feb; color: #fff; border-radius: 6px; padding: 5px 12px; cursor: pointer; font: inherit; font-size: 12px; }
/* Non-colour cues (#1215): state badges also differ by icon prefix and
   status dots by shape, so colour is never the only signal. */
.badge.walled::before, .badge.blocked::before { content: "\\26d4  "; }
.badge.drew::before { content: "\\26a0  "; }
.badge.healthy::before, .badge.moving::before, .badge.merged::before, .badge.acked::before, .badge.act-applied::before { content: "\\2713  "; }
.badge.near::before, .badge.implementing::before, .badge.stale::before, .badge.act-pending::before { content: "\\25f7  "; }
.badge.review::before, .badge.conflict::before { content: "\\25c6  "; }
.badge.act-rejected::before { content: "\\2715  "; }
.badge.act-snoozed::before { content: "\\263e  "; }
.badge.open::before { content: "\\25cf  "; }
.dot.down, .dot.invalid { border-radius: 2px; }
.dot.stale, .dot.absent { border-radius: 2px; transform: rotate(45deg); }
.dot.fixtures { background: transparent; border: 2px solid #bc8cff; }
/* Light theme (#1215): a real light palette under prefers-color-scheme;
   the dark palette above stays the default for dark scheme. */
@media (prefers-color-scheme: light) {
  body { background: #ffffff; color: #1f2328; }
  nav { background: #f6f8fa; border-bottom-color: #d1d9e0; }
  .navgroup + .navgroup { border-left-color: #d1d9e0; }
  .navlabel { color: #59636e; }
  .navgroup[data-current="true"] .navlabel { color: #0969da; }
  nav a { color: #0969da; }
  nav a.active, nav a:hover { background: #eaeef2; }
  nav a.active { background: #0969da26; color: #0550ae; box-shadow: inset 0 -2px 0 #0969da; }
  a { color: #0969da; }
  .card { background: #ffffff; border-color: #d1d9e0; }
  .col { background: #f6f8fa; border-color: #d1d9e0; }
  .col h2 { color: #59636e; }
  .muted, .foot, caption { color: #59636e; }
  .badge { border-color: #d1d9e0; }
  .badge.open, .badge.orch, .badge.agent { color: #0969da; }
  .badge.merged, .badge.healthy, .badge.moving, .badge.gh, .badge.acked, .badge.act-applied { color: #1a7f37; }
  .badge.closed { color: #59636e; }
  .badge.implementing, .badge.near, .badge.stalled, .badge.stale, .badge.msg, .badge.act-pending, .badge.whyuser { color: #9a6700; }
  .badge.walled, .badge.drew, .badge.blocked, .badge.act-rejected { color: #cf222e; }
  .badge.review, .badge.Muse, .badge.conflict, .badge.act-snoozed { color: #8250df; }
  .badge.src { background: #eaeef2; color: #1f2328; }
  th, td { border-color: #d1d9e0; }
  th { background: #f6f8fa; }
  .banner { color: #9a6700; }
  .banner.drew { color: #cf222e; }
  .chips a, .fchip, .qchip, .hchip, .xchip { background: #f6f8fa; color: #1f2328; border-color: #d1d9e0; }
  /* The tray chip background lightens above, so its restore (x) button must drop its dark-surface foreground too (Codex P2 on PR #3308). */
  .hchip button { color: #1f2328; }
  .chips a.active { color: #0969da; }
  .fchip.stale, .fchip.absent, .qblock .qremaining { color: #9a6700; }
  .fchip.invalid, .cform .err, .aform .err { color: #cf222e; }
  .dot { background: #59636e; }
  .dot.up, .dot.self { background: #1a7f37; }
  .dot.down, .dot.invalid { background: #cf222e; }
  .dot.stale, .dot.absent { background: #9a6700; }
  .dot.off, .dot.unknown { background: #59636e; }
  .dot.fixtures { background: transparent; border-color: #8250df; }
  .cmt { background: #f6f8fa; border-left-color: #d1d9e0; }
  .cov, .cform, .actbox, .skel { background: #f6f8fa; border-color: #d1d9e0; }
  /* Charts keep their authored dark surface in light mode: the SVG
     foregrounds are generated for dark (model labels #c9d1d9, the
     Pareto frontier #e6edf3), so a light chart background would wash
     them out — the chart card and its controls stay dark instead. */
  .chart { background: #0d1117; border-color: #30363d; color: #e6edf3; }
  .chart .muted { color: #8b949e; }
  .chart .xpl-ctl button, .chart .xpl-ctl select { background: #161b22; color: #e6edf3; border-color: #30363d; }
  .chart .xpl-ctl button.active { background: #1f6feb; color: #fff; border-color: #1f6feb; }
  .cform textarea, .cform select, .cform input[type="text"], .actbox select, .aform select, .aform input[type="text"], .aform input[type="datetime-local"], nav select, .xpl-filters input[type="search"], .xpl-filters select, .xpl-ctl select { background: #ffffff; color: #1f2328; border-color: #d1d9e0; }
  .cform label, .actbox .ahead label, .aform .why, .optform .effect, .orderctl { color: #59636e; }
  .cform .kinds label { color: #1f2328; }
  .aform button { background: #f6f8fa; color: #1f2328; border-color: #d1d9e0; }
  .aform button.danger { color: #cf222e; border-color: #cf222e; }
  .xtabs { border-bottom-color: #d1d9e0; }
  .xtab { color: #59636e; }
  .xtab:hover, .xtab.active { color: #1f2328; }
  .xtab.active { background: #f6f8fa; border-color: #d1d9e0; }
  .xp-seg button { background: #f6f8fa; color: #59636e; }
  .xpl-table tr:hover td { background: #f6f8fa; }
  .xpl-ctl button { background: #f6f8fa; color: #1f2328; border-color: #d1d9e0; }
  .card .sentline, .qblock .qanswered { color: #1a7f37; }
  details.carddetail { border-top-color: #d1d9e0; }
  details.carddetail dt { color: #59636e; }
  details.carddetail summary, .cbox summary, .kids summary { color: #0969da; }
  .kids { border-left-color: #d1d9e0; }
  .obj .n { color: #0969da; }
  .foot { border-top-color: #d1d9e0; }
  .qblock { border-top-color: #d1d9e0; }
  .cbox { border-top-color: #d1d9e0; }
}
@media (max-width: 720px) {
  body { font-size: 13.5px; }
  main { padding: 12px 10px 60px; }
  nav { padding: 7px 10px; gap: 2px 8px; }
  .navgroup + .navgroup { padding-left: 8px; }
  .navlabel { font-size: 10px; margin-right: 2px; }
  nav a { padding: 5px 7px; }
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
 * Shared post-submit feedback (#1443): the click must visibly land.
 * The card's controls disable, a "✓ sent · pending" line appears in
 * place, and a suppressing verb (snooze / dismiss / mark-done)
 * tombstones the card immediately — the server render (via the
 * Datastar fragment refresh this dispatches, or the next page load)
 * then moves it to its strip for real. Pages without Datastar
 * fragment containers keep the historical full reload.
 */
const DASH_AFTER_JS = `
window.__dashAfter = window.__dashAfter || function (form, msg) {
  var card = form.closest(".card");
  form.querySelectorAll("button, input, select, textarea").forEach(function (b) { b.disabled = true; });
  if (card) {
    var line = document.createElement("div");
    line.className = "sentline";
    line.textContent = msg;
    card.appendChild(line);
    var verb = form.dataset.verb;
    if (verb === "snooze" || verb === "dismiss" || verb === "mark-done") card.classList.add("tombstone");
  }
  window.dispatchEvent(new CustomEvent("dashrefresh"));
  if (!document.querySelector("[data-frag]")) location.reload();
};
`;

/**
 * The client side of the comment form (one delegated submit handler;
 * pages without a form never notice it). Kept dependency-free and
 * inline, like everything else in this app. The card comment box
 * (#1446) posts kind "message" for the orchestrator and — when its
 * "also post publicly" box is ticked and the card carries a gh: ref
 * in data-ghref — a second, kind "github" comment the orchestrator
 * posts to the issue (the aggregator holds no GitHub credentials).
 */
const COMMENT_SCRIPT = `<script>${DASH_AFTER_JS}
document.addEventListener("submit", async (ev) => {
  const form = ev.target;
  if (!(form instanceof HTMLFormElement) || !form.classList.contains("cform")) return;
  ev.preventDefault();
  const err = form.querySelector(".err");
  const data = new FormData(form);
  const kind = String(data.get("kind") || "note");
  const [akind, aid, asession] = String(data.get("author") || "drew|drew|").split("|");
  const labelEl = form.querySelector('option[value="' + CSS.escape(String(data.get("author"))) + '"]');
  const author = { kind: akind, id: aid, label: labelEl ? labelEl.textContent : aid, ...(asession ? { sessionId: asession } : {}) };
  const payload = {
    objectRef: form.dataset.object,
    kind,
    author,
    body: String(data.get("body") || ""),
    ...(kind === "github" ? { githubRef: String(data.get("githubRef") || "") } : {}),
  };
  const post = async (p) => {
    const res = await fetch("/api/comments", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(p) });
    if (!res.ok) { const b = await res.json().catch(() => ({})); throw new Error(b.error || ("HTTP " + res.status)); }
  };
  try {
    await post(payload);
    const also = form.querySelector("input[name=alsoGithub]");
    if (also && also.checked && form.dataset.ghref) {
      await post({ objectRef: form.dataset.object, kind: "github", author, body: payload.body, githubRef: form.dataset.ghref });
    }
    window.__dashAfter(form, "✓ sent · pending — the orchestrator picks it up on its next pass");
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

/**
 * The client side of the dashboard action forms (#1152/#1154/#1156):
 * one delegated submit handler, exactly the COMMENT_SCRIPT pattern —
 * POST /api/actions, reload on success, inline error on failure. The
 * form's data-* attributes carry the envelope's fixed fields; named
 * inputs carry the payload. A stop form must have its confirm box
 * ticked before the script will send it (two-step confirm).
 */
const ACTION_SCRIPT = `<script>${DASH_AFTER_JS}
document.addEventListener("submit", async (ev) => {
  const form = ev.target;
  if (!(form instanceof HTMLFormElement) || !form.classList.contains("aform")) return;
  ev.preventDefault();
  const err = form.querySelector(".err");
  const data = new FormData(form);
  if (form.dataset.verb === "stop" && !data.get("confirm")) {
    if (err) err.textContent = "Not sent: tick Confirm stop first — stopping ends the session's run.";
    return;
  }
  const submitBtn = form.querySelector("button[type=submit]");
  if (submitBtn) submitBtn.disabled = true;
  const box = form.closest(".actbox");
  const authorRaw = String((box && box.querySelector("select[name=author]") || {}).value || "drew|drew|");
  const [akind, aid, asession] = authorRaw.split("|");
  const labelEl = box ? box.querySelector('option[value="' + CSS.escape(authorRaw) + '"]') : null;
  const payload = {};
  for (const name of ["text", "reason", "status", "tier", "mode", "optionId", "proposalRunId", "proposalUpdatedAt", "needsOrder"]) {
    const v = data.get(name);
    if (v) payload[name] = String(v);
  }
  const standingAuto = data.get("standingAuto");
  if (standingAuto) payload.standingAuto = standingAuto === "true";
  const answersByQuestion = {};
  for (const el of form.querySelectorAll("[name^='answers:']")) {
    const qid = el.name.slice("answers:".length);
    if (el.type === "checkbox") { if (el.checked) (answersByQuestion[qid] = answersByQuestion[qid] || []).push(el.value); }
    else if (el.type === "radio") { if (el.checked) answersByQuestion[qid] = el.value; }
    else if (el.value) answersByQuestion[qid] = el.value;
  }
  const answerList = Object.keys(answersByQuestion).map((questionId) => ({ questionId, value: answersByQuestion[questionId] }));
  if (answerList.length) payload.answers = answerList;
  const untilCustom = data.get("untilCustom");
  const untilPreset = data.get("untilPreset");
  if (untilCustom) payload.until = new Date(String(untilCustom)).toISOString();
  else if (untilPreset) payload.until = String(untilPreset);
  const body = {
    clientToken: form.dataset.token,
    kind: form.dataset.kind,
    verb: form.dataset.verb,
    objectRef: form.dataset.object,
    author: { kind: akind, id: aid, label: labelEl ? labelEl.textContent : aid, ...(asession ? { sessionId: asession } : {}) },
    payload,
  };
  if (form.dataset.targetRef) body.targetRef = form.dataset.targetRef;
  if (form.dataset.sessionId) body.sessionId = form.dataset.sessionId;
  if (form.dataset.sessionSource) body.sessionSource = form.dataset.sessionSource;
  if (form.dataset.supersedes) body.supersedes = form.dataset.supersedes;
  try {
    const res = await fetch("/api/actions", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    if (!res.ok) { const b = await res.json().catch(() => ({})); throw new Error(b.error || ("HTTP " + res.status)); }
    window.__dashAfter(form, "✓ sent · pending — the fleet applies it on its next pass");
  } catch (e) {
    if (submitBtn) submitBtn.disabled = false;
    if (err) err.textContent = "Not recorded: " + e.message;
  }
});
</script>`;

/** Facet labels for the freshness chips (#1138) — the footer nouns. */
const FACET_LABELS: Record<keyof Provenance, string> = {
  board: "board",
  roadmap: "roadmap",
  items: "items",
  comments: "comments",
  quota: "quota",
  usage: "usage",
  attention: "attention",
  orchestration: "orchestration",
  dns: "dns",
  registry: "registry",
  aa: "aa",
};

/**
 * Per-source freshness chips (#1138): only the sources the page being
 * rendered actually uses, each with its state word, its age when the
 * source carries one, and any count WITH its noun — a chip count and
 * a page header counting the same thing read the same variable, so
 * the two can never disagree unexplained. The facet's full provenance
 * string rides in the title attribute for whoever is debugging.
 */
export function freshnessChips(detail: ProvenanceDetail, keys: (keyof Provenance)[]): string {
  const chips = keys
    .map((key) => {
      const facet = detail[key];
      const count = facet.count !== undefined ? ` · ${facet.count} ${esc(facet.countLabel ?? "")}`.trimEnd() : "";
      const age = facet.ageMs !== undefined && facet.state !== "fixtures" ? ` · ${ageMsText(facet.ageMs)}` : "";
      return `<span class="fchip ${facet.state}" title="${esc(facet.text)}">${FACET_LABELS[key]} · ${facet.state}${count}${age}</span>`;
    })
    .join("");
  return `<div class="chips fresh" aria-label="Source freshness">${chips}</div>`;
}

/**
 * The grouped nav links (#1135, #1215): one labelled section per
 * PAGE_GROUP, in bar order, marking the active page's section. The bar
 * and the Pages dropdown are ONE navigation model: both render every
 * manifest page under the same sections — /pages included, no exclusions.
 */
export function navLinks(active: string, activeGroup?: string): string {
  // A sub-route (/program/<id>, /quota/service/<id>) marks the page that
  // reaches it, so the bar always shows where you are (#1135).
  const activeRoute = manifestRouteFor(active) ?? active;
  const groupOfActive = activeGroup ?? pageByRoute(activeRoute)?.group;
  let activeMarked = false;
  return groupedPages()
    .map(({ group, pages }) => {
      const anchors = pages
        .map((p) => {
          const isActive = p.route === activeRoute && !activeMarked;
          if (isActive) activeMarked = true;
          return `<a href="${p.route}"${isActive ? ' class="active" aria-current="page"' : ""}>${esc(p.title)}</a>`;
        })
        .join("");
      const current = group.id === groupOfActive ? ' data-current="true"' : "";
      return `<span class="navgroup"${current}><span class="navlabel">${esc(group.title)}</span>${anchors}</span>`;
    })
    .join("");
}

/**
 * The Pages dropdown (#906 §1, #1215): the same grouped model as the
 * bar — one <optgroup> per manifest section, every page present — so
 * the bar and the dropdown always list the identical destination set.
 */
export function pageDropdown(active: string): string {
  let selectedMarked = false;
  const groups = groupedPages()
    .map(({ group, pages }) => {
      const options = pages
        .map((p) => {
          const selected = p.route === active && !selectedMarked;
          if (selected) selectedMarked = true;
          return `<option value="${p.route}"${selected ? " selected" : ""}>${esc(p.title)} — ${esc(p.blurb)}</option>`;
        })
        .join("");
      return `<optgroup label="${esc(group.title)}">${options}</optgroup>`;
    })
    .join("");
  return `<select aria-label="Go to page" onchange="if(this.value)location.href=this.value"><option value="">Pages ▾</option>${groups}</select>`;
}

function layout(
  title: string,
  body: string,
  provenance: Provenance,
  active: string,
  extra = "",
  chipsHtml = "",
): string {
  // Nav + the page dropdown both render from the manifest (#906 §1):
  // registering a page in src/pages.ts is the only step that puts it
  // in either. Both use the same grouped model (#1215): the bar groups
  // its links by section (#1135) so pages wrap onto a second row at
  // phone width instead of scrolling sideways, the dropdown mirrors
  // the sections as <optgroup>s, and both list every page.
  const links = navLinks(active);
  const dropdown = pageDropdown(active);
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)} · aggregator</title><style>${CSS}</style><script type="module" src="/assets/datastar.js"></script></head>
<body><nav>${links}${dropdown}</nav><main>${chipsHtml}${body}
<div class="foot"><details class="srcdetail"><summary>source detail</summary>board: ${esc(provenance.board)} · roadmap: ${esc(provenance.roadmap)} · items: ${esc(provenance.items)} · comments: ${esc(provenance.comments)} · quota: ${esc(provenance.quota)} · usage: ${esc(provenance.usage)} · attention: ${esc(provenance.attention)} · orchestration: ${esc(provenance.orchestration)} · dns: ${esc(provenance.dns)} · registry: ${esc(provenance.registry)} · aa: ${esc(provenance.aa)}</details>tailnet: <a href="${TAILNET_URL}">${TAILNET_URL.replace("http://", "")}</a></div>
</main>${extra}</body></html>`;
}

// --- Dashboard comments (#906 §2) ------------------------------------------

/** Object refs: the keys dashboard comment threads hang on. */
export const sessionObjectRef = (id: string): string => `session:${id}`;
export const ghObjectRef = (ref: RefKey): string => `gh:${ref}`;
export function tierItemObjectRef(item: TierItem): string {
  // Watchdog items (#1148) all carry the implementation issue as their
  // display ref, but each watchdog:<input> is an independent object:
  // keying them all by the shared gh: ref would give every watchdog
  // card the same DOM id, actions, dismissal state, and comment
  // thread, so dismissing one input's card would suppress them all
  // (Codex P1 on PR #2431). Watchdog cards key by their unique id;
  // the gh: ref still renders as the card's issue link.
  if (item.source === "watchdog") return `orch:${item.id}`;
  return item.ref ? ghObjectRef(item.ref) : `orch:${item.id}`;
}

/** A 💬 link to an object's comment thread, with its comment count. */
export function commentBadge(objectRef: string, counts?: Map<string, number>): string {
  const n = counts?.get(objectRef) ?? 0;
  return ` <a class="badge" href="/discuss/${encodeURIComponent(objectRef)}" title="Dashboard comments on this item">💬${n ? ` ${n}` : ""}</a>`;
}

export function authorBadge(a: DashAuthor): string {
  const cls = a.kind === "drew" ? "drew" : a.kind === "muse" ? "Muse" : "agent";
  const link = a.sessionId ? ` <a href="/sessions?id=${encodeURIComponent(a.sessionId)}#${encodeURIComponent(a.sessionId)}" class="muted">session</a>` : "";
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

// --- Dashboard actions (#1152/#1154/#1156) --------------------------------
// Forms post DashActions to the actions store (src/actions.ts); the
// fleet consumers apply them and report back. Nothing here writes
// anywhere else — a control renders the REQUEST and the store's state
// (pending / applied / rejected), never a pretend-completed change.

/** "12m ago" style relative stamp for action badges. */
export function relTime(iso: string, now = Date.now()): string {
  const ms = now - Date.parse(iso);
  if (Number.isNaN(ms)) return iso;
  if (ms < 60_000) return "just now";
  const m = Math.floor(ms / 60_000);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

const VERB_BADGE_LABELS: Record<string, string> = {
  acknowledge: "acknowledge",
  snooze: "snooze",
  escalate: "escalate",
  "de-escalate": "de-escalate",
  answer: "answer",
  delegate: "delegate",
  dismiss: "dismiss",
  "mark-done": "already done",
  restore: "restore",
  "set-needs-order": "card order",
  "approve-proposal": "approve proposal",
  "reject-proposal": "reject proposal",
  "request-status": "status change",
  "set-delegation-default": "standing default",
  poke: "poke",
  stop: "stop",
};

function pendingBadgeLabel(a: DashAction): string {
  if (a.verb === "request-status") return `status → ${a.payload.status ?? "?"} requested`;
  if (a.verb === "snooze") return `snooze until ${a.payload.until ? fmtDate(a.payload.until) : "?"} requested`;
  if (a.verb === "delegate") return `delegate (${a.payload.mode ?? "?"}) requested`;
  return `${VERB_BADGE_LABELS[a.verb] ?? a.verb} requested`;
}

/**
 * The visible state of an object's actions: every still-pending action
 * as a ⏳ badge, plus the latest applied/rejected outcome per verb
 * family (snooze / status / acknowledge / tier / delegation) so the
 * user can SEE that an action landed — or that it was rejected, with
 * the consumer's note.
 */
export function actionBadgesHtml(actions: DashAction[] | undefined, now = Date.now()): string {
  if (!actions?.length) return "";
  const badges: string[] = [];
  for (const a of actions) {
    if (a.state === "new") {
      badges.push(
        `<span class="badge act-pending" title="Action ${esc(a.id)} — waiting for the consuming fleet job's next run">⏳ ${esc(pendingBadgeLabel(a))} · ${esc(relTime(a.createdAt, now))}</span>`,
      );
    }
  }
  const snooze = latestInFamily(actions, "snooze");
  if (snooze?.state === "applied" && snooze.payload.until && Date.parse(snooze.payload.until) > now) {
    badges.push(
      `<span class="badge act-snoozed" title="Snooze applied by ${esc(snooze.appliedByRun ?? "a fleet run")}">💤 snoozed until ${esc(fmtDate(snooze.payload.until))}</span>`,
    );
  }
  const status = latestInFamily(actions, "status");
  if (status?.state === "applied") {
    badges.push(
      `<span class="badge act-applied">✓ status → ${esc(status.payload.status ?? "")} applied · ${esc(relTime(status.appliedAt ?? status.createdAt, now))}</span>`,
    );
  } else if (status?.state === "rejected") {
    badges.push(
      `<span class="badge act-rejected" title="${esc(status.resultNote ?? "")}">✗ status → ${esc(status.payload.status ?? "")} rejected${status.resultNote ? ` — ${esc(status.resultNote)}` : ""}</span>`,
    );
  }
  const ack = latestInFamily(actions, "ack");
  if (ack?.state === "applied") {
    badges.push(
      `<span class="badge act-applied">✓ acknowledged · ${esc(relTime(ack.appliedAt ?? ack.createdAt, now))}</span>`,
    );
  }
  const tier = latestInFamily(actions, "tier");
  if (tier?.state === "applied") {
    badges.push(
      `<span class="badge act-applied">✓ moved to ${tier.payload.tier === "needs-drew" ? "Needs User" : "Needs Orchestrator"} · ${esc(relTime(tier.appliedAt ?? tier.createdAt, now))}</span>`,
    );
  } else if (tier?.state === "rejected") {
    badges.push(
      `<span class="badge act-rejected" title="${esc(tier.resultNote ?? "")}">✗ ${esc(VERB_BADGE_LABELS[tier.verb] ?? tier.verb)} rejected${tier.resultNote ? ` — ${esc(tier.resultNote)}` : ""}</span>`,
    );
  }
  const delegation = latestInFamily(actions, "delegation");
  if (delegation?.state === "applied") {
    badges.push(
      `<span class="badge act-applied">✓ delegated (${esc(delegation.payload.mode ?? "")}) · ${esc(relTime(delegation.appliedAt ?? delegation.createdAt, now))}</span>`,
    );
  } else if (delegation?.state === "rejected") {
    badges.push(
      `<span class="badge act-rejected" title="${esc(delegation.resultNote ?? "")}">✗ delegate rejected${delegation.resultNote ? ` — ${esc(delegation.resultNote)}` : ""}</span>`,
    );
  }
  return badges.length ? ` <span class="actbadges">${badges.join(" ")}</span>` : "";
}

export interface ActionFormOpts {
  /** Author identities for the picker (standing + registry sessions). */
  identities: AuthorIdentity[];
  /** When set, every control renders disabled with this reason shown. */
  disabledReason?: string;
}

/** Stable DOM id for one card, from its object ref (#1444 fragments
 *  patch sections; #1443's in-place feedback finds cards by ref). */
export function cardDomId(objectRef: string): string {
  return `card-${objectRef.replace(/[^a-zA-Z0-9]+/g, "-")}`;
}

/** The card's action history (#1445): every recorded action with its
 *  state — the audit trail the store keeps, readable on the card. */
export function actionHistoryHtml(actions: DashAction[] | undefined): string {
  if (!actions?.length) return `<p class="muted">No actions recorded on this card yet.</p>`;
  return [...actions]
    .reverse()
    .map((a) => {
      const bits = [
        `<strong>${esc(VERB_BADGE_LABELS[a.verb] ?? a.verb)}</strong>`,
        a.state === "new"
          ? "⏳ pending"
          : a.state === "applied"
            ? `✓ applied${a.appliedByRun ? ` by ${esc(a.appliedByRun)}` : ""}`
            : a.state === "rejected"
              ? "✗ rejected"
              : "superseded",
        esc(relTime(a.createdAt)),
        a.payload.reason ? `— ${esc(a.payload.reason)}` : "",
        a.payload.until ? `until ${esc(fmtDate(a.payload.until))}` : "",
        a.resultNote ? `· ${esc(a.resultNote)}` : "",
      ].filter(Boolean);
      return `<div class="histline">${bits.join(" ")}</div>`;
    })
    .join("");
}

/**
 * The per-card comment box (#1446): the card's existing dashboard
 * comments with their queued/posted state, plus a compact form. The
 * form posts kind "message" (the orchestrator's queue input); the
 * "also post publicly" checkbox adds a kind "github" comment when
 * the card carries a gh: ref — posted later by the orchestrator,
 * never by the aggregator (no GitHub credentials here).
 */
export function cardCommentHtml(
  objectRef: string,
  ghRef: RefKey | undefined,
  comments: DashComment[] | undefined,
  opts: CommentFormOpts,
): string {
  const list = comments?.length
    ? comments.map((c) => dashCommentHtml(c)).join("")
    : `<p class="muted">No comments on this card yet.</p>`;
  const authorOptions = opts.identities
    .map(
      (i) =>
        `<option value="${esc(`${i.kind}|${i.id}|${i.sessionId ?? ""}`)}"${i.id === "drew" ? " selected" : ""}>${esc(i.label)}${i.kind === "agent" ? " (agent)" : ""}</option>`,
    )
    .join("");
  const ghBox = ghRef
    ? `<label><input type="checkbox" name="alsoGithub" value="1"> Also post publicly to the GitHub issue (${esc(ghRef)})</label>`
    : `<span class="muted">No GitHub issue on this card — the comment stays internal.</span>`;
  return `<div class="cbox"><details><summary>💬 Comments for the orchestrator${comments?.length ? ` (${comments.length})` : ""}</summary>${list}<form class="cform" data-object="${esc(objectRef)}"${ghRef ? ` data-ghref="${esc(ghRef)}"` : ""}>
<input type="hidden" name="kind" value="message">
<label>Comment as</label><select name="author">${authorOptions}</select>
<label>Comment</label><textarea name="body" rows="2" required placeholder="Note for the orchestrator…"></textarea>
<div class="kinds">${ghBox}</div>
<button type="submit">Send comment</button><div class="err"></div></form></details></div>`;
}

/** One-tap dismissal reasons (#1449): the common resolutions. */
export const DISMISS_REASONS = ["Already handled", "No longer relevant", "Duplicate", "Won't do"] as const;
/** One-tap de-escalation reasons (#1449): Drew's "send why" chips. */
export const DEESCALATE_REASONS = [
  "Orchestrator should have handled this",
  "Passing to orchestrator this time",
  "Need more info",
  "Need recommendations",
] as const;

/**
 * The quick-action bar (#1449): one-tap resolutions with the reason
 * attached, writing the existing action vocabulary. Chips disable
 * while their verb is pending, so a resolution can't be double-sent.
 */
export function quickBarHtml(objectRef: string, tier: "needs-drew" | "needs-orchestrator", cs: CardState): string {
  const chip = (
    verb: DashAction["verb"],
    label: string,
    hidden: string,
    opts: { primary?: boolean; disabled?: boolean; sentLabel?: string } = {},
  ): string =>
    aformHtml({
      kind: "triage",
      verb,
      objectRef,
      inner: `${hidden}<button type="submit" class="qchip${opts.primary ? " primary" : ""}"${opts.disabled ? " disabled" : ""}>${esc(opts.disabled && opts.sentLabel ? opts.sentLabel : label)}</button>`,
    }).replace('class="aform"', 'class="aform qform"');
  const donePending = pendingVerb(cs, "mark-done");
  const dismissPending = pendingVerb(cs, "dismiss");
  const movePending = pendingVerb(cs, "de-escalate") ?? pendingVerb(cs, "escalate");
  const parts: string[] = [
    chip("mark-done", "Already done", "", {
      primary: true,
      disabled: !!donePending,
      sentLabel: "✓ Already done · sent",
    }),
  ];
  for (const reason of DISMISS_REASONS) {
    parts.push(
      chip("dismiss", `Dismiss: ${reason}`, `<input type="hidden" name="reason" value="${esc(reason)}">`, {
        disabled: !!dismissPending,
        sentLabel: "Dismiss · sent",
      }),
    );
  }
  if (tier === "needs-drew") {
    for (const reason of DEESCALATE_REASONS) {
      parts.push(
        chip(
          "de-escalate",
          reason,
          `<input type="hidden" name="tier" value="needs-orchestrator"><input type="hidden" name="reason" value="${esc(reason)}">`,
          { disabled: !!movePending, sentLabel: "Sent to orchestrator" },
        ),
      );
    }
  }
  const note = donePending || dismissPending || movePending ? ` <span class="muted">⏳ sent · pending</span>` : "";
  return `<div class="quickbar"><span class="qlabel">Quick actions — one tap, reason attached${note}</span>${parts.join("")}</div>`;
}

function authorSelectHtml(identities: AuthorIdentity[]): string {
  const options = identities
    .map(
      (i) =>
        `<option value="${esc(`${i.kind}|${i.id}|${i.sessionId ?? ""}`)}"${i.id === "drew" ? " selected" : ""}>${esc(i.label)}${i.kind === "agent" ? " (agent)" : ""}</option>`,
    )
    .join("");
  return `<div class="ahead"><label>Act as</label><select name="author">${options}</select></div>`;
}

interface AFormSpec {
  kind: DashAction["kind"];
  verb: DashAction["verb"];
  objectRef: string;
  targetRef?: RefKey;
  sessionId?: string;
  sessionSource?: string;
  supersedes?: string;
  /** Inner controls (inputs/buttons), already escaped HTML. */
  inner: string;
}

function aformHtml(spec: AFormSpec): string {
  const data = [
    `data-kind="${esc(spec.kind)}"`,
    `data-verb="${esc(spec.verb)}"`,
    `data-object="${esc(spec.objectRef)}"`,
    `data-token="${esc(newClientToken())}"`,
    spec.targetRef ? `data-target-ref="${esc(spec.targetRef)}"` : "",
    spec.sessionId ? `data-session-id="${esc(spec.sessionId)}"` : "",
    spec.sessionSource ? `data-session-source="${esc(spec.sessionSource)}"` : "",
    spec.supersedes ? `data-supersedes="${esc(spec.supersedes)}"` : "",
  ]
    .filter(Boolean)
    .join(" ");
  return `<form class="aform" ${data}>${spec.inner}<div class="err"></div></form>`;
}

/** Wrap a card's action forms: one author picker + the disabled gate. */
function actboxHtml(opts: ActionFormOpts, forms: string): string {
  const inner = `${authorSelectHtml(opts.identities)}${forms}`;
  return opts.disabledReason
    ? `<div class="actbox"><p class="muted">⚠ Actions unavailable — ${esc(opts.disabledReason)}</p><fieldset disabled>${inner}</fieldset></div>`
    : `<div class="actbox">${inner}</div>`;
}

function snoozeFieldsHtml(): string {
  const now = new Date();
  const tonight = new Date(now);
  tonight.setHours(23, 59, 0, 0);
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  tomorrow.setHours(9, 0, 0, 0);
  const nextWeek = new Date(now);
  nextWeek.setDate(nextWeek.getDate() + 7);
  nextWeek.setHours(9, 0, 0, 0);
  return `<select name="untilPreset" aria-label="Snooze until"><option value="${tonight.toISOString()}">until tonight</option><option value="${tomorrow.toISOString()}">until tomorrow 9:00</option><option value="${nextWeek.toISOString()}">until next week</option></select><input type="datetime-local" name="untilCustom" aria-label="Snooze until (custom)">`;
}

/**
 * The #1152 triage control set for one tier item: acknowledge (a
 * non-resolving "seen" record), snooze (presets + custom), and the
 * tier move toward the OTHER tier (de-escalate on Needs User cards,
 * escalate on Needs Orchestrator cards), each with an optional reason.
 * The answer control lives with the item's own surface (session cards
 * carry the session answer; #1291 adds the tier answer row).
 */
export function triageActionsHtml(
  objectRef: string,
  tier: "needs-drew" | "needs-orchestrator",
  opts: ActionFormOpts,
  extraForms = "",
  cs: CardState = cardState(undefined),
): string {
  // #1443: a control whose verb is already pending (or, for
  // acknowledge, already in effect) renders disabled — repeat clicks
  // cannot pile up duplicate pending actions, and the card SAYS so.
  const ackPending = pendingVerb(cs, "acknowledge");
  const snoozePending = pendingVerb(cs, "snooze");
  const moveVerb = tier === "needs-drew" ? "de-escalate" : "escalate";
  const movePending = pendingVerb(cs, moveVerb);
  const move =
    tier === "needs-drew"
      ? aformHtml({
          kind: "triage",
          verb: "de-escalate",
          objectRef,
          inner: `<input type="hidden" name="tier" value="needs-orchestrator"><input type="text" name="reason" placeholder="Why? (optional free text)" aria-label="De-escalate reason"><button type="submit"${movePending ? " disabled" : ""}>${movePending ? "De-escalate · sent · pending" : "De-escalate to Needs Orchestrator"}</button>`,
        })
      : aformHtml({
          kind: "triage",
          verb: "escalate",
          objectRef,
          inner: `<input type="hidden" name="tier" value="needs-drew"><input type="text" name="reason" placeholder="Why? (optional free text)" aria-label="Escalate reason"><button type="submit"${movePending ? " disabled" : ""}>${movePending ? "Escalate · sent · pending" : "Escalate to Needs User"}</button>`,
        });
  const ackInner = ackPending
    ? `<button type="submit" class="primary" disabled>⏳ Acknowledge sent · pending</button><span class="why">Recorded — the fleet applies it on its next pass.</span>`
    : cs.acked
      ? `<button type="submit" class="primary" disabled>✓ Acknowledged</button><span class="why">You have seen this — it stays until a resolving action or a new signal.</span>`
      : `<button type="submit" class="primary">Acknowledge</button><span class="why">Records that you have seen this — it stays until a resolving action or a new signal.</span>`;
  const forms = [
    quickBarHtml(objectRef, tier, cs),
    aformHtml({
      kind: "triage",
      verb: "acknowledge",
      objectRef,
      inner: ackInner,
    }),
    aformHtml({
      kind: "triage",
      verb: "snooze",
      objectRef,
      inner: `${snoozeFieldsHtml()}<button type="submit"${snoozePending ? " disabled" : ""}>${snoozePending ? "Snooze · sent · pending" : "Snooze"}</button>`,
    }),
    move,
  ].join("");
  return actboxHtml(opts, forms + extraForms);
}

// --- Needs-user action layer (#1291) ---------------------------------------
// Everything in this block renders ONLY what the published item and
// the actions store actually say — why-class, options, survey,
// delegation state, proposal, inability. No decorative states: a
// control exists exactly when its verb can be posted and consumed.

const WHY_USER_LABELS: Record<WhyUserClass, string> = {
  decision: "Decision",
  "missing-info": "Missing Info",
  approval: "Approval",
  access: "Access",
  policy: "Policy",
};

const DELEGATION_MODE_LABELS: Record<DelegationMode, string> = {
  user: "With You",
  "first-pass": "First Pass",
  auto: "Auto",
};

export function whyUserBadge(item: TierItem): string {
  if (!item.whyUser) return "";
  return ` <span class="badge whyuser ${esc(item.whyUser)}" title="Why this needs the user: ${esc(WHY_USER_LABELS[item.whyUser])}">${esc(WHY_USER_LABELS[item.whyUser])}</span>`;
}

function questionFieldHtml(q: TierItemQuestion, answeredValue?: string | string[]): string {
  const name = `answers:${q.id}`;
  const answered = answeredValue !== undefined;
  let field: string;
  if (q.type === "single" || q.type === "yesno") {
    const opts = q.options ?? [
      { id: "yes", label: "Yes" },
      { id: "no", label: "No" },
    ];
    field = `<span class="qopts">${opts
      .map(
        (o) =>
          `<label><input type="radio" name="${esc(name)}" value="${esc(o.id)}"${answeredValue === o.id ? " checked" : ""}> ${esc(o.label)}</label>`,
      )
      .join("")}</span>`;
  } else if (q.type === "multi") {
    const chosen = Array.isArray(answeredValue) ? answeredValue : answeredValue ? [answeredValue] : [];
    field = `<span class="qopts">${(q.options ?? [])
      .map(
        (o) =>
          `<label><input type="checkbox" name="${esc(name)}" value="${esc(o.id)}"${chosen.includes(o.id) ? " checked" : ""}> ${esc(o.label)}</label>`,
      )
      .join("")}</span>`;
  } else {
    field = `<input type="text" name="${esc(name)}" aria-label="${esc(q.prompt)}" value="${esc(typeof answeredValue === "string" ? answeredValue : "")}">`;
  }
  // #1451: answered state rides the card — a partial submit records
  // what it covered and the still-open required questions say so.
  const state = answered
    ? ` <span class="qanswered">✓ answered: ${esc(Array.isArray(answeredValue) ? answeredValue.join(", ") : (answeredValue ?? ""))}</span>`
    : q.required
      ? ` <span class="qremaining">still needed</span>`
      : "";
  return `<div class="qblock"><strong>${esc(q.prompt)}</strong>${q.required ? "" : ` <span class="muted">(optional)</span>`}${state}<br>${field}<br><span class="muted">Needed: ${esc(q.covers)}</span></div>`;
}

/** The latest applied reject-proposal on an item, when one exists —
 *  its reason rides the next proposal's box (#1293). */
function lastRejection(actions: DashAction[] | undefined): DashAction | undefined {
  return (actions ?? []).filter((a) => a.verb === "reject-proposal" && a.state === "applied").at(-1);
}

/**
 * The needs-user body block for one card: summary, the what-happens-
 * if-this-sits line, the delegation status line, and the proposal /
 * inability boxes (#1291/#1293). Pure content — the forms live in
 * needsUserFormsHtml, inside the card's actbox.
 */
export function needsUserBodyHtml(item: TierItem, actions: DashAction[] | undefined, now = Date.now()): string {
  const parts: string[] = [];
  if (item.summary) parts.push(`<br><span>${esc(item.summary)}</span>`);
  if (item.ifIgnored)
    parts.push(`<br><span class="muted"><strong>If nothing happens:</strong> ${esc(item.ifIgnored)}</span>`);
  const d = item.delegation;
  if (d) {
    const rejected = lastRejection(actions);
    const statusBits = [
      `Delegation: <strong>${esc(DELEGATION_MODE_LABELS[d.mode])}</strong>`,
      d.status === "idle"
        ? "not started"
        : d.status === "working"
          ? "the orchestrator is working it"
          : d.status === "proposal-ready"
            ? "proposal ready for your review"
            : d.status === "unable"
              ? "returned — the orchestrator could not resolve it"
              : "done",
      d.runId ? `run ${esc(d.runId)}` : "",
      `updated ${esc(relTime(d.updatedAt, now))}`,
    ].filter(Boolean);
    parts.push(`<br><span class="muted">${statusBits.join(" · ")}</span>`);
    if (rejected && d.mode === "first-pass" && d.status !== "proposal-ready")
      parts.push(
        `<br><span class="muted">Previous proposal rejected: ${esc(rejected.payload.reason ?? "")} — a new first pass must differ materially or say why it cannot.</span>`,
      );
    if (d.status === "proposal-ready" && d.proposal) {
      if (proposalExpired(d, now)) {
        parts.push(
          `<div class="propbox expired"><strong>Proposal expired</strong> — drafted ${esc(fmtDate(d.proposal.at))} by run ${esc(d.runId ?? "unknown")}; it is no longer actionable. Delegate a fresh first pass below.<br><strong>${esc(d.proposal.summary)}</strong></div>`,
        );
      } else {
        parts.push(
          `<div class="propbox"><strong>First-pass proposal</strong>${rejected ? ` — second first pass; previous rejected: ${esc(rejected.payload.reason ?? "")}` : ""}<br><strong>${esc(d.proposal.summary)}</strong>${
            d.proposal.detail ? `<br>${esc(d.proposal.detail)}` : ""
          }${
            d.proposal.proposedChanges?.length
              ? `<br>Proposed changes:<ul>${d.proposal.proposedChanges.map((c) => `<li>${esc(c)}</li>`).join("")}</ul>`
              : ""
          }<span class="muted">Drafted ${esc(fmtDate(d.proposal.at))} — nothing is applied unless you approve; approval takes effect on the next orchestration pass.</span></div>`,
        );
      }
    }
    if (d.status === "unable" && d.unable) {
      parts.push(
        `<div class="unablebox"><strong>The orchestrator could not resolve this</strong> — ${esc(d.unable.reason)}<br>Tried:<ul>${d.unable.whatItTried.map((t) => `<li>${esc(t)}</li>`).join("")}</ul>Needs: ${esc(d.unable.whatItNeeds)}<br><span class="muted">Declared ${esc(fmtDate(d.unable.at))} by run ${esc(d.runId ?? "unknown")}.</span></div>`,
      );
    }
  }
  // #1293: a pending approve/reject whose proposal has since changed
  // renders as no longer applicable, not as a live request.
  const pendingProposalAction = (actions ?? []).some(
    (a) => a.state === "new" && (a.verb === "approve-proposal" || a.verb === "reject-proposal"),
  );
  if (pendingProposalAction) {
    const live = proposalIdentity(item);
    const stillLive = (actions ?? []).some(
      (a) =>
        a.state === "new" &&
        (a.verb === "approve-proposal" || a.verb === "reject-proposal") &&
        live !== null &&
        a.payload.proposalRunId === live.runId &&
        a.payload.proposalUpdatedAt === live.updatedAt,
    );
    if (!stillLive)
      parts.push(
        `<br><span class="muted">⏳ A pending proposal action is no longer applicable — the proposal changed since it was recorded; the pass will reject it with that note.</span>`,
      );
  }
  return parts.join("");
}

/**
 * The needs-user forms for one card (they join the card's actbox):
 * the item's own options as answer buttons, the survey as one answer
 * form, a free-text answer when the item offers neither, delegation
 * mode controls, proposal approve/reject, and dismiss (#1291).
 */
export function needsUserFormsHtml(
  item: TierItem,
  objectRef: string,
  now = Date.now(),
  cs: CardState = cardState(undefined),
): string {
  const forms: string[] = [];
  for (const opt of item.options ?? []) {
    const picked = cs.answeredOptionId === opt.id;
    forms.push(
      aformHtml({
        kind: "triage",
        verb: "answer",
        objectRef,
        inner: `<input type="hidden" name="optionId" value="${esc(opt.id)}"><button type="submit" class="primary"${picked ? " disabled" : ""}>${esc(opt.label)}${picked ? " ✓ answered · pending" : ""}</button>${
          opt.description ? `<span class="why">${esc(opt.description)}</span>` : ""
        }${opt.effect ? `<span class="effect">Effect: ${esc(opt.effect)}</span>` : ""}`,
      }),
    );
  }
  if (item.questions?.length) {
    const remaining = item.questions.filter((q) => q.required && !cs.answered.has(q.id));
    const allDone = remaining.length === 0 && item.questions.some((q) => cs.answered.has(q.id));
    forms.push(
      aformHtml({
        kind: "triage",
        verb: "answer",
        objectRef,
        inner: `${item.questions.map((q) => questionFieldHtml(q, cs.answered.get(q.id))).join("")}<button type="submit" class="primary">Submit answers</button><span class="why">${
          allDone
            ? "All required questions are answered — the answers are recorded and pending with the orchestrator."
            : `Answer any subset and submit (#1451): answered questions are recorded, and the card keeps showing what remains${remaining.length ? ` — still needed: ${esc(remaining.map((q) => q.id).join(", "))}` : ""}.`
        }</span>`,
      }),
    );
  }
  if (!item.options?.length && !item.questions?.length) {
    forms.push(
      aformHtml({
        kind: "triage",
        verb: "answer",
        objectRef,
        inner: `<input type="text" name="text" placeholder="Your answer / decision" aria-label="Answer" required><button type="submit" class="primary">Answer</button>`,
      }),
    );
  }
  const d = item.delegation;
  if (d) {
    const autoBlocked = item.whyUser === "access" || item.whyUser === "policy";
    const expired = proposalExpired(d, now);
    const modeButton = (mode: DelegationMode, label: string) => {
      if (mode === "auto" && autoBlocked)
        return `<span class="aform"><button type="button" disabled>Delegate: Auto</button><span class="why">${esc(WHY_USER_LABELS[item.whyUser!])} items can never be auto-delegated — they stay with you.</span></span>`;
      const current = d.mode === mode && !(expired && mode === "first-pass");
      return aformHtml({
        kind: "triage",
        verb: "delegate",
        objectRef,
        inner: `<input type="hidden" name="mode" value="${esc(mode)}"><button type="submit"${current ? " disabled" : ""}>${esc(label)}${current ? " (current)" : ""}</button>`,
      });
    };
    forms.push(
      modeButton(
        "first-pass",
        expired && d.mode === "first-pass" ? "Delegate: First Pass (fresh draft)" : "Delegate: First Pass",
      ),
      modeButton("auto", "Delegate: Auto"),
      modeButton("user", "Bring Back to Me"),
    );
    if (d.status === "proposal-ready" && d.proposal && !expired && d.runId) {
      const identity =
        `<input type="hidden" name="proposalRunId" value="${esc(d.runId)}">` +
        `<input type="hidden" name="proposalUpdatedAt" value="${esc(d.updatedAt)}">`;
      forms.push(
        aformHtml({
          kind: "triage",
          verb: "approve-proposal",
          objectRef,
          inner: `${identity}<button type="submit" class="primary">I accept your proposal</button><span class="why">The pass applies it on its next run and reports the application.</span>`,
        }),
        aformHtml({
          kind: "triage",
          verb: "reject-proposal",
          objectRef,
          inner: `${identity}<input type="text" name="reason" placeholder="Why not? (required)" aria-label="Rejection reason" required><button type="submit">No — make another</button>`,
        }),
      );
    }
  }
  forms.push(
    aformHtml({
      kind: "triage",
      verb: "dismiss",
      objectRef,
      inner: `<input type="text" name="reason" placeholder="Reason (required)" aria-label="Dismiss reason" required><button type="submit">Dismiss</button><span class="why">Dismissed items stay out of the queue unless a new material fact appears — then they resurface with the fact cited.</span>`,
    }),
  );
  // A pending delegation action supersedes the stamped mode display:
  // the badges already say so; nothing extra to render here.
  return forms.join("");
}

/** The standing delegation-default toggle (#1290/#1291): the current
 *  effective value from the preference records, with the change form. */
export function delegationDefaultHtml(records: DashAction[] | undefined, opts: ActionFormOpts): string {
  const eff = effectiveStandingAuto(records ?? []);
  const state = eff.standingAuto
    ? `<strong>On</strong> — Decision, Missing Info, and Approval items delegate to Auto by default`
    : `<strong>Off</strong> — items stay with you unless you delegate them per item`;
  const pending = eff.pending
    ? ` <span class="badge act-pending">⏳ change pending — applies on the next orchestration pass</span>`
    : "";
  const form = aformHtml({
    kind: "preference",
    verb: "set-delegation-default",
    objectRef: DELEGATION_DEFAULT_OBJECT_REF,
    inner: `<label>Standing auto <select name="standingAuto"><option value="true"${eff.standingAuto ? " selected" : ""}>On</option><option value="false"${eff.standingAuto ? "" : " selected"}>Off</option></select></label><button type="submit">Save standing default</button><span class="why">Access and Policy items never auto, whatever this says. Per-item delegation always overrides the default. Initial record: the user's decision, 2026-10-27.</span>`,
  });
  return `<div class="card"><strong>Standing delegation default</strong> — ${state}.${pending}${actboxHtml(opts, form)}</div>`;
}

/** The Project Status values a status request may target (#910 field). */
export const PROJECT_STATUS_VALUES = ["Todo", "In Progress", "In Review", "Done"] as const;

/** The #1154 control: request a Project Status change (never a direct write). */
export function statusRequestHtml(
  objectRef: string,
  targetRef: RefKey,
  currentStatus: string | null,
  opts: ActionFormOpts,
): string {
  const options = PROJECT_STATUS_VALUES.map(
    (s) => `<option value="${esc(s)}"${s === currentStatus ? " selected" : ""}>${esc(s)}</option>`,
  ).join("");
  const form = aformHtml({
    kind: "status-request",
    verb: "request-status",
    objectRef,
    targetRef,
    inner: `<label>Request status change to <select name="status">${options}</select></label><input type="text" name="reason" placeholder="Reason (optional)" aria-label="Status change reason"><button type="submit">Request status change</button><span class="why">A request, not a write — the orchestration pass (the Project's single writer) applies it on its next run.</span>`,
  });
  return actboxHtml(opts, form);
}

/** Which session verbs a source can support, and why not when it can't. */
export function sessionVerbSupport(source: string): { verb: "poke" | "answer" | "stop"; reason?: string }[] {
  if (source === "opencode-sandbox" || source === "opencode-laptop") {
    return [{ verb: "poke" }, { verb: "answer" }, { verb: "stop" }];
  }
  if (source === "jules") {
    return [
      { verb: "poke", reason: "Jules sessions are keeper-mediated — answer only" },
      { verb: "answer" },
      { verb: "stop", reason: "Jules sessions are keeper-mediated — answer only" },
    ];
  }
  if (source === "claude-code") {
    const reason = "No live control channel for Muse sessions — launch records only";
    return [
      { verb: "poke", reason },
      { verb: "answer", reason },
      { verb: "stop", reason },
    ];
  }
  return [];
}

/** The #1156 control set for one live session (merged registry identity). */
export function sessionActionsHtml(sessionId: string, source: string, opts: ActionFormOpts): string {
  const support = sessionVerbSupport(source);
  if (!support.length) return "";
  const objectRef = sessionObjectRef(sessionId);
  const forms = support
    .map(({ verb, reason }) => {
      if (reason) {
        return `<div class="aform"><button type="button" disabled>${esc(verb === "poke" ? "Poke" : verb === "answer" ? "Answer" : "Stop")}</button><span class="why">${esc(reason)}.</span></div>`;
      }
      if (verb === "poke") {
        return aformHtml({
          kind: "session",
          verb,
          objectRef,
          sessionId,
          sessionSource: source,
          inner: `<button type="submit" class="primary">Poke</button><span class="why">Nudges the session via its owning server.</span>`,
        });
      }
      if (verb === "answer") {
        return aformHtml({
          kind: "session",
          verb,
          objectRef,
          sessionId,
          sessionSource: source,
          inner: `<input type="text" name="text" placeholder="Answer for the waiting session…" aria-label="Answer text" required><button type="submit">Answer</button>`,
        });
      }
      return aformHtml({
        kind: "session",
        verb,
        objectRef,
        sessionId,
        sessionSource: source,
        inner: `<label class="confirm"><input type="checkbox" name="confirm" value="yes"> Confirm stop</label><button type="submit" class="danger">Stop session</button><span class="why">Two-step: tick Confirm stop, then Stop — the owning job terminates the run.</span>`,
      });
    })
    .join("");
  return actboxHtml(opts, forms);
}

/** Terminal registry statuses — a record in one of these is history, not live. */
const TERMINAL_SESSION_STATUSES = new Set([
  "completed",
  "complete",
  "done",
  "failed",
  "error",
  "stopped",
  "terminated",
  "succeeded",
  "cancelled",
]);

/** A registry record counts as live inside the attention currency window (2 h). */
export function sessionRecordIsLive(r: SessionRecord, now = Date.now()): boolean {
  if (TERMINAL_SESSION_STATUSES.has(r.status.toLowerCase())) return false;
  if (!r.lastSeenAt) return false;
  return now - Date.parse(r.lastSeenAt) < 2 * 60 * 60 * 1000;
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
  // #1141: fixture-resolved state is invented fallback data — render it
  // hollow (dashed, transparent) and named "unverified" so it can never
  // be mistaken for a live badge on board/roadmap/item/program pages.
  const unverified = item.resolution === "fixture";
  const cls = item.state === "merged" ? "merged" : item.state === "open" ? "open" : "closed";
  const kind = item.kind === "pr" ? (item.state === "merged" ? "PR merged" : "PR") : "issue";
  const draft = item.draft ? " · draft" : "";
  const marker = unverified ? " · unverified" : "";
  const title = unverified ? `${item.title} (unverified — fixture state, not live GitHub data)` : item.title;
  return `<a class="badge ${cls}${unverified ? " unverified" : ""}" href="${esc(item.url)}" title="${esc(title)}">${kind} ${esc(item.ref)}${draft}${marker}</a>`;
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

/** The board's card grid alone (#1444 fragment payload). */
export function renderBoardBody(
  board: BoardItem[],
  items: Map<RefKey, GhItem>,
  counts?: Map<string, number>,
  act?: {
    actionsByObject?: Map<string, DashAction[]>;
    poolByRef?: Map<RefKey, ProjectPoolItem>;
    identities?: AuthorIdentity[];
  },
): string {
  // #1146: finished work is not WIP — aged-out done items and the
  // retired ROADMAP Done-table class never reach the board, whatever
  // source assembled it (the Kaneo source filters too; see
  // board-policy.ts). Surviving done renders collapsed, below.
  board = applyBoardDonePolicy(board);
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
          return `<div class="card"><a href="/item/${encodeURIComponent(item.id)}">${src}${esc(item.title)}</a>${cmt}<br>${repo}${kaneo}${stripHtml(items, item.links)}${statusCtl}</div>`;
        })
        .join("");
      // The in-flight lane is dated by the orchestration section that
      // produced it (stamped on every synthetic entry as updatedAt).
      if (k === "done") {
        // Collapsed by default (#1146): a count + the last 7 days
        // behind an expander, never a full column of finished cards.
        return `<details class="donestrip"><summary><strong>Done recently (${colItems.length})</strong> — done in the last 7 days, collapsed by default; older done is aged off the board (GitHub is the record)</summary>${cards}</details>`;
      }
      const heading =
        k === "in-flight"
          ? `in flight — orchestrator · in flight · section ${fmtDate(colItems[0]?.updatedAt)} (${colItems.length})`
          : `${esc(k)} (${colItems.length})`;
      return `<div class="col"><h2>${heading}</h2>${cards}</div>`;
    })
    .join("");
  return `<div class="board">${bodies}</div>`;
}

/** The board page (#1444): full server render. */
export function renderBoard(
  board: BoardItem[],
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  counts?: Map<string, number>,
  detail?: ProvenanceDetail,
  act?: {
    actionsByObject?: Map<string, DashAction[]>;
    poolByRef?: Map<RefKey, ProjectPoolItem>;
    identities?: AuthorIdentity[];
  },
): string {
  return layout(
    "Cross-repo WIP board",
    `<h1>Cross-repo WIP board</h1><p class="muted">What is being worked on now is the <strong>in-flight</strong> lane: synthetic entries from the orchestrator's lined-up tasks (the fleet feed's orchestration section), not Kaneo cards. The other columns are the curated Kaneo board across the four repos, including its completion history — on their own they are a record of finished and in-review work, not a live WIP list. Each Kaneo card's execution strip is derived from its linked issues/PRs: implementing → review → merged.</p>${renderBoardBody(board, items, counts, act)}`,
    provenance,
    "/board",
    act?.identities ? ACTION_SCRIPT : "",
    detail ? freshnessChips(detail, ["board", "items", "orchestration"]) : "",
  );
}

/** The Datastar shell for / (#1444): heading + intro paint at once;
 *  the board itself streams in from /frag/board. */
export function renderBoardShell(provenance: Provenance, detail?: ProvenanceDetail): string {
  return layout(
    "Cross-repo WIP board",
    `<h1>Cross-repo WIP board</h1><p class="muted">What is being worked on now is the <strong>in-flight</strong> lane: synthetic entries from the orchestrator's lined-up tasks (the fleet feed's orchestration section), not Kaneo cards.</p><noscript><meta http-equiv="refresh" content="0; url=/board"></noscript><div id="board-main" data-frag data-on-load="@get('/frag/board')" data-on-dashrefresh__window="@get('/frag/board')"><div class="skel">Loading the board…</div></div>`,
    provenance,
    "/board",
    ACTION_SCRIPT,
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

function roadmapCard(row: RoadmapRow, items: Map<RefKey, GhItem>, counts?: Map<string, number>): string {
  const refs = allRefs([row]);
  const refBadges = refs
    .map((r) => items.get(r))
    .filter((x): x is GhItem => Boolean(x))
    .map(itemBadge)
    .join(" ");
  const cmt = counts ? commentBadge(row.subject, counts) : "";
  const grist = row.gristUrl ? ` <a class="badge" href="${esc(row.gristUrl)}">Grist ↗</a>` : "";
  return `<div class="card"><a href="/item/${encodeURIComponent(row.subject)}">${esc(row.title)}</a>${cmt}<br><span class="badge ${esc(row.status.toLowerCase().replace(/[^a-z]/g, "-"))}">${esc(row.status)}</span> <span class="muted">${esc(row.area)} · ${esc(row.horizon)}</span>${refBadges}${grist}</div>`;
}

function timelineTable(
  tl: ReturnType<typeof buildTimeline>,
  items: Map<RefKey, GhItem>,
  counts?: Map<string, number>,
): string {
  const head = `<tr><th scope="col">Theme</th>${tl.lanes.map((l) => `<th scope="col">${esc(l.label)}</th>`).join("")}</tr>`;
  const body = tl.themes
    .map((theme) => {
      const cells = tl.lanes
        .map((lane) => {
          const rs = tl.cells.get(theme)?.get(lane.key) ?? [];
          return `<td class="tl-cell">${rs.map((r) => roadmapCard(r, items, counts)).join("")}</td>`;
        })
        .join("");
      return `<tr><th scope="row">${esc(theme)}</th>${cells}</tr>`;
    })
    .join("");
  return `<div class="tblwrap"><table><caption>Roadmap timeline by theme</caption>${head}${body}</table></div>`;
}

export function renderRoadmap(
  rows: RoadmapRow[],
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  grain: RoadmapGrain = "quarter",
  counts?: Map<string, number>,
  detail?: ProvenanceDetail,
  reconciliation?: RoadmapReconciliation,
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
    `<h1>Roadmap</h1><p class="muted">Grist's <em>Program roadmap</em> table at every grain — one data model, switchable horizon. Rows with a calendar date land on their day; quarter-labelled rows stay at quarter grain.</p><div class="chips">${chips}</div>${driftFlag}${body}`,
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
  const statusSection =
    extras?.statusRequest && extras.formOpts
      ? `<h2>Request status change</h2>${actionBadgesHtml(extras.actions)}${statusRequestHtml(extras.statusRequest.objectRef, extras.statusRequest.targetRef, extras.statusRequest.currentStatus, { identities: extras.formOpts.identities })}`
      : "";
  const svcLinks = extras?.serviceLinks?.length
    ? `<p>Live services: ${extras.serviceLinks.map((s) => `<a href="${esc(s.url)}">${esc(s.label)} ↗</a>`).join(" · ")}</p>`
    : "";
  return layout(
    title,
    `<h1>${esc(title)}</h1><p class="muted">subject: <code>${esc(subject)}</code></p>${svcLinks}${stripHtml(items, links)}
<h2>Linked issues / PRs</h2><div class="tblwrap"><table><caption>Linked issues and pull requests</caption><tr><th scope="col">Ref</th><th scope="col">Title</th><th scope="col">Labels</th><th scope="col">Updated</th></tr>${refRows || '<tr><td colspan="4" class="muted">none resolved</td></tr>'}</table></div>
${noteList ? `<h2>Planning notes</h2><ul>${noteList}</ul>` : ""}
<h2>Activity stream</h2><div class="tblwrap"><table><caption>Activity stream</caption><tr><th scope="col">When</th><th scope="col">Kind</th><th scope="col">Author</th><th scope="col">Body</th></tr>${commentRows || '<tr><td colspan="4" class="muted">no comments</td></tr>'}</table></div>
${sessionSection}${statusSection}${dashSection}`,
    provenance,
    "/board",
    extras ? COMMENT_SCRIPT + (extras.statusRequest ? ACTION_SCRIPT : "") : "",
  );
}

/** One registry record as a compact line (item page, sessions page). */
export function sessionRecordLine(
  r: SessionRecord,
  act?: { actions?: DashAction[]; identities?: AuthorIdentity[] },
): string {
  const tokens = r.tokensTotal != null ? ` · ${r.tokensTotal.toLocaleString()} tok` : "";
  const parent = r.parentId
    ? ` · sub of <a href="/sessions?id=${encodeURIComponent(r.parentId)}#${encodeURIComponent(r.parentId)}">${esc(r.parentId)}</a>`
    : "";
  const refs = r.taskRefs.length
    ? ` · ${r.taskRefs.map((ref) => `<a href="${esc(refUrl(ref))}">${esc(ref)}</a>`).join(" ")}`
    : "";
  // #1156: live records carry the session action set on the merged
  // registry identity; historical records render no controls.
  const badges = actionBadgesHtml(act?.actions);
  const controls =
    act?.identities && sessionRecordIsLive(r) ? sessionActionsHtml(r.id, r.source, { identities: act.identities }) : "";
  return `<div class="card" id="${esc(r.id)}"><strong>${esc(r.label)}</strong> <span class="badge src">${esc(r.source)}</span> <span class="badge agent">${esc(r.agentLabel)}</span>${badges}<br><span class="muted">${esc(r.id)} · ${esc(r.status)}${r.model ? ` · ${esc(r.model)}` : ""} · started ${fmtDate(r.startedAt)} · last seen ${fmtDate(r.lastSeenAt)}${tokens}${parent}${refs}</span>${controls}</div>`;
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
  if (unit === "cny") return `¥${v.toFixed(2)}`;
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

/**
 * The DeepSeek PAYG reserve's balance, from the snapshot store (#1052):
 * the newest unit "cny" record. Shown in ¥ with its own measured-at —
 * the yuan figure as read, never converted to dollars.
 */
function paygBalanceHtml(usage: UsageView): string {
  const recs = (usage.history["deepseek:api"] ?? []).filter((r) => r.unit === "cny" && typeof r.value === "number");
  if (!recs.length) return "";
  const last = recs[recs.length - 1];
  return `<div><strong>DeepSeek PAYG reserve balance</strong>: <strong>¥${(last.value as number).toFixed(2)}</strong><br><span class="muted">reserve only — never a routine route · measured ${esc(last.ts.slice(0, 10))} · ${esc(last.observedBy)}</span></div>`;
}

function paygCardHtml(usage: UsageView): string {
  const balance = paygBalanceHtml(usage);
  const balanceBlock = balance ? `<div>${balance}</div>` : "";
  if (!usage.payg.length) {
    return `<div class="card"><div><strong>PAYG dollars</strong><br><span class="muted">pay-as-you-go agent spend</span></div><div class="muted">No metered spill recorded in local records.</div>${balanceBlock}</div>`;
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
  return `<div class="card"><div><strong>PAYG dollars</strong><br><span class="muted">metered spill, counted per Decision 004 — estimates at 1.2 × high</span><br><span class="badge ${total > 5 ? "near" : "healthy"}">$${total.toFixed(2)} projected unbilled</span></div><div>${lines}${balanceBlock}</div></div>`;
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
  return `<h2>Charges this period</h2><p class="muted">Evidence, not accounting — charges observed on cards/receipts in the local records (full balances stay in the finance tooling). Total observed: $${total.toFixed(2)}.</p><div class="tblwrap"><table><caption>Charges this period</caption><tr><th scope="col">Date</th><th scope="col">Charge</th><th scope="col">Amount</th><th scope="col">Method</th><th scope="col">Evidence</th></tr>${rows}</table></div>`;
}

export function renderQuota(
  view: QuotaView,
  provenance: Provenance,
  usage?: UsageView,
  detail?: ProvenanceDetail,
): string {
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
      const credRows = (p.credentials ?? [])
        .map((c) => {
          const wins = c.windows.map((w) => `${esc(w.name)} ${windowPercent(w).toFixed(0)}%`).join(" \u00b7 ");
          const state =
            c.freshness.state === "ok"
              ? ""
              : ` \u00b7 ${esc(c.freshness.state)}${c.freshness.error ? ` \u00b7 ${esc(c.freshness.error)}` : ""}`;
          return `<div class="muted">credential ${esc(c.label)}: ${wins || esc(c.freshness.state)}${state}</div>`;
        })
        .join("");
      const fresh = p.freshness
        ? `<div class="muted">gatherer · ${esc(p.freshness.state)} · measured ${fmtDate(p.freshness.measuredAt)} · ${esc(p.freshness.source)}${p.freshness.error ? ` · ${esc(p.freshness.error)}` : ""}</div>`
        : "";
      return `<div class="card pool"><div><strong>${esc(p.provider)}</strong><br><span class="muted">${esc(p.plan)}</span><br><span class="badge ${esc(p.state)}">${esc(p.state)}${p.wallLiftsAt ? ` · lifts ${fmtDate(p.wallLiftsAt)}` : ""}</span>${p.burnTokensPerMin ? ` <span class="muted">${p.burnTokensPerMin} tok/min</span>` : ""}</div><div>${windows}${credRows}${sessions}${fresh}${p.note ? `<div class="muted">${esc(p.note)}</div>` : ""}</div></div>`;
    })
    .join("");
  const burn = view.burn
    .map(
      (b) =>
        `<tr><td>${esc(b.provider)}</td><td>${esc(b.pool)}</td><td>${fmtDate(b.from)}</td><td>${fmtDate(b.to)}</td><td>${b.tokens.toLocaleString()}</td></tr>`,
    )
    .join("");
  const choices = (list: string[]) => (list.length ? list.map(esc).join(", ") : "—");
  const burnAndChoices = `<h2>Burn between resets</h2><div class="tblwrap"><table><caption>Burn between resets</caption><tr><th scope="col">Provider</th><th scope="col">Pool</th><th scope="col">From</th><th scope="col">To</th><th scope="col">Tokens</th></tr>${burn || '<tr><td colspan="5" class="muted">none</td></tr>'}</table></div>
<h2>Model choice</h2><div class="tblwrap"><table><caption>Model choice rules</caption><tr><th scope="col">Rule</th><th scope="col">Pools</th></tr>
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
    "",
    detail ? freshnessChips(detail, ["quota", "usage"]) : "",
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

export function renderServiceDetail(
  svc: SaasService,
  history: SnapshotRecord[],
  provenance: Provenance,
  detail?: ProvenanceDetail,
): string {
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
<h2>All metrics</h2><div class="tblwrap"><table><caption>All metrics</caption><tr><th scope="col">Metric</th><th scope="col">Used</th><th scope="col">Limit</th><th scope="col">%</th><th scope="col">Note</th></tr>${metricRows}</table></div>
${payg}
<h2>Snapshot history</h2>${charts || '<p class="muted">No snapshots in the store for this service yet — history starts with the §4 backfill and accumulates as writers append.</p>'}
${paces ? `<h2>Pace projections</h2><div class="card">${paces}</div>` : ""}
${conflicts}
<h2>Dated source reads</h2>${reads}
<p class="muted">Machine-first: <a href="/api/quota/service.json?id=${encodeURIComponent(svc.id)}">/api/quota/service.json?id=${esc(svc.id)}</a></p>`;
  return layout(
    `${svc.name} · usage`,
    body,
    provenance,
    "/quota",
    "",
    detail ? freshnessChips(detail, ["quota", "usage"]) : "",
  );
}

// --- Attention view (#852, tiered in slice 2) -------------------------------

function sessionCard(
  e: TieredEntry,
  children: Map<string, TieredEntry[]>,
  depth = 0,
  counts?: Map<string, number>,
  act?: {
    actionsByObject?: Map<string, DashAction[]>;
    identities?: AuthorIdentity[];
    commentsByObject?: Map<string, DashComment[]>;
    commentOpts?: CommentFormOpts;
  },
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
      ? `<span class="badge drew">Needs User</span>`
      : e.tier === "needs-orchestrator"
        ? `<span class="badge orch">Needs Orchestrator</span>`
        : "";
  const evidence = e.supervisionNote ? `${e.evidence} Keeper: ${e.supervisionNote}` : e.evidence;
  const reason = e.drewReason ? `<div class="reason"><strong>Why you:</strong> ${esc(e.drewReason)}</div>` : "";
  const orch = e.orchestratorNote ? `<div class="orch-note">${esc(e.orchestratorNote)}</div>` : "";
  const kids = children.get(e.sessionId) ?? [];
  const kidsHtml = kids.length
    ? `<details class="kids"><summary>${kids.length} sub-session${kids.length === 1 ? "" : "s"}</summary>${kids.map((k) => sessionCard(k, children, depth + 1, counts, act)).join("")}</details>`
    : "";
  const cmt = counts ? commentBadge(sessionObjectRef(e.sessionId), counts) : "";
  // #1156 + #1152: session actions (poke/answer/stop by source support)
  // and, for tiered entries, the triage set — session-derived items
  // stay actionable even when the tier source is down (the session
  // feed is live), so no disabledReason ever applies here.
  const objectRef = sessionObjectRef(e.sessionId);
  const own = act?.actionsByObject?.get(objectRef);
  const badges = actionBadgesHtml(own);
  const cs = cardState(own);
  let controls = "";
  if (act?.identities) {
    const sessionBox = sessionActionsHtml(e.sessionId, sourceOf(e), { identities: act.identities });
    const triageBox =
      e.tier === "needs-drew" || e.tier === "needs-orchestrator"
        ? triageActionsHtml(objectRef, e.tier, { identities: act.identities }, "", cs)
        : "";
    controls = `${sessionBox}${triageBox}`;
  }
  // #1445/#1446: inline detail (state + full action history) and the
  // per-card comment box ride the session card too.
  const detail = `<details class="carddetail"><summary>Details &amp; history</summary><dl><dt>Session</dt><dd><code>${esc(e.sessionId)}</code></dd><dt>State</dt><dd>${esc(STATE_LABELS[e.state] ?? e.state)}</dd><dt>Last activity</dt><dd>${esc(fmtDate(e.lastActivityAt))}</dd><dt>Ref</dt><dd><code>${esc(objectRef)}</code></dd></dl>${actionHistoryHtml(own)}</details>`;
  const commentBox =
    act?.identities && act.commentOpts
      ? cardCommentHtml(objectRef, undefined, act.commentsByObject?.get(objectRef), act.commentOpts)
      : "";
  return `<div class="card${cs.acked ? " acked" : ""}" id="${cardDomId(objectRef)}"><strong><a href="/sessions?id=${encodeURIComponent(e.sessionId)}#${encodeURIComponent(e.sessionId)}">${esc(e.title ?? e.sessionId)}</a></strong> <span class="badge src">${esc(src)}</span> <span class="badge ${esc(e.state)}">${esc(STATE_LABELS[e.state])}</span>${tierBadge}${cs.acked ? ` <span class="badge acked">Acknowledged</span>` : ""}${sub}${cmt}${badges}<br><span class="muted">${esc(e.sessionId)} · ${esc(e.directory ?? "—")}${e.branch ? ` · ${esc(e.branch)}` : ""} · ${esc(e.model ?? "—")}${e.provider ? ` / ${esc(e.provider)}` : ""} · last activity ${fmtDate(e.lastActivityAt)} · ${esc(e.messageCount ?? "—")} msgs${e.outcome ? ` · outcome: ${esc(e.outcome)}` : ""}</span><br>${esc(evidence)}${reason}${orch}${controls}${detail}${commentBox}${kidsHtml}</div>`;
}

/** A #899 tier item that is not a live session — a decision, task, or gap. */
function tierItemCard(
  item: TierItem,
  whyLabel: string,
  counts?: Map<string, number>,
  act?: {
    actions?: DashAction[];
    identities?: AuthorIdentity[];
    disabledReason?: string;
    comments?: DashComment[];
    commentOpts?: CommentFormOpts;
  },
): string {
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
  const badges = actionBadgesHtml(act?.actions);
  // #1291: on Needs User cards, the item's own contract renders — the
  // why-class badge in the header, the summary / if-ignored /
  // delegation / proposal / inability block in the body, and the
  // options / survey / delegation / dismiss forms in the actbox.
  const isUserTier = whyLabel === "Why you";
  const objectRef = tierItemObjectRef(item);
  const cs = cardState(act?.actions);
  const extBody = isUserTier ? needsUserBodyHtml(item, act?.actions) : "";
  const extForms = isUserTier ? needsUserFormsHtml(item, objectRef, Date.now(), cs) : "";
  // #1152: the triage control set. Section-derived items disable with
  // the reason when the tier source is absent/invalid (#1139 state) —
  // the item may not exist in the publisher's current view.
  const controls = act?.identities
    ? triageActionsHtml(
        objectRef,
        isUserTier ? "needs-drew" : "needs-orchestrator",
        {
          identities: act.identities,
          ...(act.disabledReason ? { disabledReason: act.disabledReason } : {}),
        },
        extForms,
        cs,
      )
    : "";
  // #1445: the title itself links to the GitHub issue when the card
  // carries a gh: ref, and every card expands inline to its full
  // detail — source, state, since/lastSeenAt, horizon, blockedBy,
  // the question payload, and the recorded action history.
  const titleHtml = item.ref ? `<a href="${esc(refUrl(item.ref))}">${esc(item.title)}</a>` : esc(item.title);
  const questionPayload = item.questions?.length
    ? `<dt>Questions</dt><dd>${item.questions
        .map(
          (q) =>
            `${esc(q.prompt)} <span class="muted">(covers: ${esc(q.covers)}${q.required ? "" : "; optional"})</span>`,
        )
        .join("<br>")}</dd>`
    : item.question
      ? `<dt>Question</dt><dd>“${esc(item.question)}”</dd>`
      : "";
  const detail = `<details class="carddetail"><summary>Details &amp; history</summary><dl><dt>Source</dt><dd>${esc(item.source)} · ${esc(item.kind)}${item.state ? ` · state ${esc(item.state)}` : ""}</dd><dt>Since</dt><dd>${esc(fmtDate(item.since))}</dd><dt>Last seen</dt><dd>${esc(fmtDate(item.lastSeenAt))}</dd>${item.horizon ? `<dt>Horizon</dt><dd>${esc(item.horizon)}</dd>` : ""}<dt>Ref</dt><dd><code>${esc(objectRef)}</code>${item.ref ? ` · <a href="${esc(refUrl(item.ref))}">${esc(item.ref)}</a>` : ""}</dd>${item.blockedBy?.length ? `<dt>Blocked by</dt><dd>${item.blockedBy.map((d) => esc(`${d.task} (${d.status}${d.workerLabel ? `, ${d.workerLabel}` : ""})`)).join("; ")}</dd>` : ""}${questionPayload}</dl>${actionHistoryHtml(act?.actions)}</details>`;
  const commentBox =
    act?.identities && act.commentOpts ? cardCommentHtml(objectRef, item.ref, act.comments, act.commentOpts) : "";
  return `<div class="card${cs.acked ? " acked" : ""}" id="${cardDomId(objectRef)}"><span class="badge src">${esc(item.source)}</span> <span class="badge">${esc(item.kind)}</span>${isUserTier ? whyUserBadge(item) : ""}${cs.acked ? ` <span class="badge acked">Acknowledged</span>` : ""} <strong>${titleHtml}</strong>${cmt}${badges}<br><strong>${esc(whyLabel)}:</strong> ${esc(item.reason)}${question}${blocked}${action}${extBody}${ref}${controls}${detail}${commentBox}</div>`;
}

export interface AttentionAct {
  actionsByObject?: Map<string, DashAction[]>;
  identities?: AuthorIdentity[];
  commentsByObject?: Map<string, DashComment[]>;
  commentOpts?: CommentFormOpts;
  /** #1456: the effective Needs-you card order + its pending state. */
  needsOrder?: NeedsOrder;
  needsOrderPending?: boolean;
}

interface StripEntry {
  objectRef: string;
  title?: string;
  cs: CardState;
}

/** One suppressed card in a strip (#1443/#1450): what happened, when it
 *  lifts, and the way back — Unsnooze for a snooze (a superseding
 *  snooze at "now"), Restore for a dismiss / mark-done (a restore
 *  action superseding the suppressor). */
function stripCardHtml(entry: StripEntry, identities: AuthorIdentity[] | undefined): string {
  const sup = entry.cs.suppressed!;
  const refLink = `<a href="/discuss/${encodeURIComponent(entry.objectRef)}">${entry.title ? esc(entry.title) : `<code>${esc(entry.objectRef)}</code>`}</a>`;
  if (sup.kind === "snoozed") {
    const a = sup.action;
    const stateLine =
      a.state === "new"
        ? `⏳ sent · pending — snooze until ${esc(fmtDate(sup.until))}`
        : `applied by ${esc(a.appliedByRun ?? "a fleet run")} — snoozed until ${esc(fmtDate(sup.until))}`;
    const unsnooze =
      identities !== undefined
        ? actboxHtml(
            { identities },
            aformHtml({
              kind: "triage",
              verb: "snooze",
              objectRef: entry.objectRef,
              supersedes: a.id,
              inner: `<input type="hidden" name="untilPreset" value="${new Date().toISOString()}"><button type="submit">Unsnooze now</button>`,
            }),
          )
        : "";
    return `<div class="card stripcard" id="${cardDomId(entry.objectRef)}">💤 ${refLink} <span class="badge act-snoozed">snoozed until ${esc(fmtDate(sup.until))}</span><br><span class="muted">${stateLine}${a.resultNote ? ` — ${esc(a.resultNote)}` : ""}</span>${unsnooze}</div>`;
  }
  const a = sup.action;
  const label = sup.kind === "done" ? "✓ marked done by you" : "dismissed by you";
  const stateLine =
    a.state === "new"
      ? "⏳ sent · pending"
      : a.state === "applied"
        ? `applied by ${esc(a.appliedByRun ?? "a fleet run")}`
        : a.state;
  const restore =
    identities !== undefined
      ? actboxHtml(
          { identities },
          aformHtml({
            kind: "triage",
            verb: "restore",
            objectRef: entry.objectRef,
            supersedes: a.id,
            inner: `<button type="submit">Restore to the queue</button><span class="why">Records a restore that supersedes the ${sup.kind === "done" ? "mark-done" : "dismiss"} — the card returns with its history intact.</span>`,
          }),
        )
      : "";
  return `<div class="card stripcard" id="${cardDomId(entry.objectRef)}">${refLink} <span class="badge act-applied">${label}</span><br><span class="muted">${stateLine}${a.payload.reason ? ` — reason: ${esc(a.payload.reason)}` : ""}${a.resultNote ? ` · ${esc(a.resultNote)}` : ""} · ${esc(relTime(a.createdAt))}</span>${restore}</div>`;
}

const HORIZON_RANK: Record<string, number> = { today: 0, "3d": 1, week: 2, month: 3, quarter: 4 };

/** #1456: the order control for the Needs-you list — a preference
 *  action; the render applies the saved order immediately. */
function needsOrderHtml(act: AttentionAct | undefined): string {
  if (!act?.identities) return "";
  const order = act.needsOrder ?? "urgency";
  const labels: Record<NeedsOrder, string> = {
    urgency: "Urgency (orchestrator's order)",
    newest: "Newest first",
    oldest: "Oldest first",
    deadline: "Deadline (horizon)",
  };
  const options = (Object.keys(labels) as NeedsOrder[])
    .map((o) => `<option value="${o}"${o === order ? " selected" : ""}>${labels[o]}</option>`)
    .join("");
  const form = aformHtml({
    kind: "preference",
    verb: "set-needs-order",
    objectRef: NEEDS_ORDER_OBJECT_REF,
    inner: `<label>Card order <select name="needsOrder">${options}</select></label><button type="submit">Save order</button>${act.needsOrderPending ? `<span class="badge act-pending">⏳ change pending — the orchestrator's canonical order follows on its next pass</span>` : ""}`,
  });
  return `<div class="orderctl">Needs-you list order: <strong>${esc(labels[order])}</strong>${actboxHtml({ identities: act.identities }, form)}</div>`;
}

export function renderAttention(
  view: AttentionView,
  coverage: SourceCoverage[],
  provenance: Provenance,
  counts?: Map<string, number>,
  detail?: ProvenanceDetail,
  act?: AttentionAct,
): string {
  const sections = attentionSections(view, coverage, counts, act);
  return layout(
    "Needs you",
    `<h1>Needs you</h1><p class="muted">The NEEDS USER queue is an intake the orchestrator works first — it resolves what it can and escalates only what genuinely requires the user, each escalated item with its reason. Everything else the fleet can work is the orchestrator's own queue. Finished runs and stale items expire out (2 h after a run finishes, 24 h at the latest). Machine-first: <a href="/api/attention.json">/api/attention.json</a> · <a href="/api/coverage.json">/api/coverage.json</a>.</p>
${sections.header}${sections.queues}${sections.sessions}`,
    provenance,
    "/attention",
    act?.identities ? ACTION_SCRIPT + COMMENT_SCRIPT : "",
    detail ? freshnessChips(detail, ["orchestration", "attention", "registry"]) : "",
  );
}

/**
 * The attention page in three independently loadable sections
 * (#1444): header (banner / chips / coverage / standing default),
 * queues (Needs User + Needs Orchestrator + the snoozed & hidden
 * strips), sessions. The full page assembles them; the shell serves
 * them as Datastar fragments from /frag/attention/*.
 */
export function attentionSections(
  view: AttentionView,
  coverage: SourceCoverage[],
  counts?: Map<string, number>,
  act?: AttentionAct,
): { header: string; queues: string; sessions: string } {
  const children = sessionChildren(view.all);
  const roots = sessionRoots(view.all);
  // #1139: the tier source's state decides what the counts may claim.
  // Absent/invalid → the orchestrator's published queues are NOT
  // reflected here, so the banner says so and the headings render
  // "unknown" rather than a confident number; stale → the numbers
  // render, badged with the section's age. This banner is also
  // #1148's aggregator-side dead-man's switch: if the orchestration
  // job dies, this is the surface that says so.
  const ts = view.tierSource;
  const tierBanner = !ts.present
    ? ts.state === "invalid"
      ? `<div class="banner">Tier source invalid — the orchestration section is present but failed validation (${esc((ts.failedChecks ?? []).join(", ") || "unknown check")}); showing session-derived items only — the orchestrator's published queues are NOT reflected in the counts below.</div>`
      : `<div class="banner">Tier source missing — showing session-derived items only; the orchestrator's published queues are NOT reflected in the counts below.</div>`
    : ts.state === "stale"
      ? `<div class="banner">Tier source stale — the last orchestration section (${esc(ts.runId ?? "unknown run")}) is ${esc(ts.generatedAt ? feedAgeText(ts.generatedAt) : "old")}; the queues below are its last published state and may have moved since.</div>`
      : "";
  const sectionStamp =
    ts.present && ts.runId
      ? ` <span class="muted">· section ${esc(ts.runId)} · ${esc(ts.generatedAt ? feedAgeText(ts.generatedAt) : "")}</span>`
      : "";
  // #1152: section-derived items disable their controls whenever the
  // tier source is not live — stale items come from the publisher's
  // last section and may no longer exist in its current view (absent/
  // invalid sources serve no section items at all). Session-derived
  // items stay enabled: the session feed is live.
  const sectionDisabledReason =
    ts.state !== "live"
      ? `the tier source is ${ts.state}, so this item cannot be confirmed against the orchestrator's current published queues`
      : undefined;
  const tierAct = (item: TierItem) => {
    const objectRef = tierItemObjectRef(item);
    // #1148 watchdog cards describe the aggregator's CURRENT diagnosis,
    // not the publisher's possibly-obsolete section, so the #1152 gate
    // must not disable them: the orchestration-outage card is exactly
    // the one that has to stay actionable while the tier source is
    // absent/invalid/stale (Codex P2, PR #2431).
    const disabledReason = item.source === "watchdog" ? undefined : sectionDisabledReason;
    return {
      actions: act?.actionsByObject?.get(objectRef),
      identities: act?.identities,
      ...(disabledReason ? { disabledReason } : {}),
      comments: act?.commentsByObject?.get(objectRef),
      commentOpts: act?.commentOpts,
    };
  };
  const sessionAct = act
    ? {
        actionsByObject: act.actionsByObject,
        identities: act.identities,
        commentsByObject: act.commentsByObject,
        commentOpts: act.commentOpts,
      }
    : undefined;

  // #1443/#1449/#1450: partition every queue entry by its render-time
  // effective state. A suppressed entry (active snooze, dismiss, or
  // mark-done — pending counts, exactly like applied) leaves its list
  // NOW and lands in the matching strip with its way back; the counts
  // below are the VISIBLE counts, so acting on a card visibly moves
  // the numbers.
  const now = Date.now();
  const snoozedList: StripEntry[] = [];
  const hiddenList: StripEntry[] = [];
  const inViewRefs = new Set<string>();
  const partition = (
    objectRef: string,
    title: string | null | undefined,
    actions: DashAction[] | undefined,
  ): CardState => {
    inViewRefs.add(objectRef);
    const cs = cardState(actions, now);
    if (cs.suppressed) {
      (cs.suppressed.kind === "snoozed" ? snoozedList : hiddenList).push({ objectRef, title: title ?? undefined, cs });
    }
    return cs;
  };
  interface QueuedCard {
    html: string;
    sinceMs: number;
    seenMs: number;
    rank: number;
  }
  const drewCardsList: QueuedCard[] = [];
  for (const e of view.drew) {
    const objectRef = sessionObjectRef(e.sessionId);
    const cs = partition(objectRef, e.title ?? e.sessionId, act?.actionsByObject?.get(objectRef));
    if (cs.suppressed) continue;
    drewCardsList.push({
      html: sessionCard(e, children, 0, counts, sessionAct),
      sinceMs: Date.parse(e.lastActivityAt ?? "") || 0,
      seenMs: Date.parse(e.lastActivityAt ?? "") || 0,
      rank: 5,
    });
  }
  for (const item of view.drewExtra) {
    const objectRef = tierItemObjectRef(item);
    const cs = partition(objectRef, item.title, act?.actionsByObject?.get(objectRef));
    if (cs.suppressed) continue;
    drewCardsList.push({
      html: tierItemCard(item, "Why you", counts, tierAct(item)),
      sinceMs: Date.parse(item.since) || 0,
      seenMs: Date.parse(item.lastSeenAt) || 0,
      rank: item.horizon ? (HORIZON_RANK[item.horizon] ?? 5) : 5,
    });
  }
  // #1456: the saved card order applies at render. "urgency" is the
  // published order (no sort); the rest sort the visible list.
  // #1485: every order sorts on the single sinceMs axis (when the card
  // entered the queue), so newest and oldest are exact reverses. seenMs
  // (lastSeenAt) is NOT the axis: a stale card seen recently would lead
  // both orders. Deadline ties break oldest-waiting first (ascending
  // sinceMs), matching the deadline/urgency reading.
  const order = act?.needsOrder ?? "urgency";
  if (order === "newest") drewCardsList.sort((a, b) => b.sinceMs - a.sinceMs);
  else if (order === "oldest") drewCardsList.sort((a, b) => a.sinceMs - b.sinceMs);
  else if (order === "deadline") drewCardsList.sort((a, b) => a.rank - b.rank || a.sinceMs - b.sinceMs);
  const orchCardsList: string[] = [];
  for (const e of view.orchestrator) {
    const objectRef = sessionObjectRef(e.sessionId);
    const cs = partition(objectRef, e.title ?? e.sessionId, act?.actionsByObject?.get(objectRef));
    if (cs.suppressed) continue;
    orchCardsList.push(sessionCard(e, children, 0, counts, sessionAct));
  }
  for (const item of view.orchestratorExtra) {
    const objectRef = tierItemObjectRef(item);
    const cs = partition(objectRef, item.title, act?.actionsByObject?.get(objectRef));
    if (cs.suppressed) continue;
    orchCardsList.push(tierItemCard(item, "Why the fleet", counts, tierAct(item)));
  }
  // Store-only suppressed refs: items the publisher no longer lists
  // (their dismissal was applied and they left the tiers) stay visible
  // — and restorable — here, never silently gone (#1450).
  for (const [objectRef, actions] of act?.actionsByObject?.entries() ?? []) {
    if (inViewRefs.has(objectRef)) continue;
    const cs = cardState(actions, now);
    if (cs.suppressed) {
      (cs.suppressed.kind === "snoozed" ? snoozedList : hiddenList).push({ objectRef, cs });
    }
  }

  const drewVisible = drewCardsList.length;
  const orchVisible = orchCardsList.length;
  const drewCount = ts.present ? String(drewVisible) : "unknown";
  const orchCount = ts.present ? String(orchVisible) : "unknown";
  const chips = `<div class="chips"><a href="#drew">Needs User (${drewCount})</a><a href="#orchestrator">Needs Orchestrator (${orchCount})</a><a href="#sessions">All sessions (${roots.length})</a></div>`;
  // #1290/#1291: the standing delegation-default toggle — a dashboard
  // preference, so it renders whenever the action surface is up; its
  // state comes from the preference records in the store.
  const delegationDefault = act?.identities
    ? delegationDefaultHtml(act.actionsByObject?.get(DELEGATION_DEFAULT_OBJECT_REF), {
        identities: act.identities,
      })
    : "";
  const header = `${tierBanner}${chips}
${coverageHtml(coverage)}
${delegationDefault}`;
  const drewCards =
    drewCardsList.map((c) => c.html).join("") ||
    (ts.present
      ? `<p class="muted">Nothing in the user queue right now.</p>`
      : `<p class="muted">Unknown — the tier source is missing, so this queue cannot be reported.</p>`);
  const orchCards =
    orchCardsList.join("") ||
    (ts.present
      ? `<p class="muted">The fleet's queue is empty.</p>`
      : `<p class="muted">Unknown — the tier source is missing, so this queue cannot be reported.</p>`);
  const snoozedStrip = snoozedList.length
    ? `<h2 id="snoozed">Snoozed by you (${snoozedList.length})</h2><p class="muted">These items are out of the queues until their snooze expires — the orchestration pass omits them, and they resurface on their own. Unsnoozing records a superseding action.</p>${snoozedList.map((e) => stripCardHtml(e, act?.identities)).join("")}`
    : "";
  const hiddenStrip = hiddenList.length
    ? `<details class="hiddenstrip"><summary><strong>Dismissed / done by you (${hiddenList.length})</strong> — hidden from the queues; restore any of them to re-review or add a note</summary>${hiddenList.map((e) => stripCardHtml(e, act?.identities)).join("")}</details>`
    : "";
  const queues = `${needsOrderHtml(act)}<h2 id="drew">Needs User (${drewCount})${sectionStamp}</h2>${drewCards}
${snoozedStrip}${hiddenStrip}<h2 id="orchestrator">Needs Orchestrator (${orchCount})${sectionStamp}</h2><p class="muted">The orchestration job (#899) reviews this queue and pushes work back into motion; it escalates to the user queue only what truly needs the user.</p>${orchCards}`;
  const sessionCards = roots.length
    ? roots.map((e) => sessionCard(e, children, 0, counts, sessionAct)).join("")
    : `<p class="muted">No current sessions.</p>`;
  const sessions = `<h2 id="sessions">All sessions (${roots.length})</h2><p class="muted">Every current session with its source and state; sub-sessions collapse under their parent.</p>${sessionCards}
${view.expiredCount ? `<p class="muted">${view.expiredCount} finished or stale ${view.expiredCount === 1 ? "entry" : "entries"} expired out of this view.</p>` : ""}`;
  return { header, queues, sessions };
}

/** Wrap one attention section as a Datastar fragment container: the
 *  shell's placeholder and the fragment payload share this shape, so
 *  a refresh re-patches the container with itself (attrs included). */
export function attentionFragment(section: "header" | "queues" | "sessions", innerHtml: string): string {
  return `<div id="attn-${section}" data-frag data-on-load="@get('/frag/attention/${section}')" data-on-dashrefresh__window="@get('/frag/attention/${section}')">${innerHtml}</div>`;
}

/**
 * The Datastar shell for /attention (#1444): nav + headings paint
 * immediately; the three heavy sections stream in as fragments.
 * Without JS, a meta refresh lands on the full server render.
 */
export function renderAttentionShell(provenance: Provenance, detail?: ProvenanceDetail): string {
  const skel = (label: string) => `<div class="skel">Loading ${label}…</div>`;
  const body = `<h1>Needs you</h1><p class="muted">Loading the Needs User and Needs Orchestrator queues…</p><noscript><meta http-equiv="refresh" content="0; url=/attention?full=1"></noscript>
${attentionFragment("header", skel("queue status"))}
${attentionFragment("queues", skel("the queues"))}
${attentionFragment("sessions", skel("sessions"))}`;
  return layout(
    "Needs you",
    body,
    provenance,
    "/attention",
    ACTION_SCRIPT + COMMENT_SCRIPT,
    detail ? freshnessChips(detail, ["orchestration", "attention", "registry"]) : "",
  );
}

// --- Today view (slice 2, #898 §4) -------------------------------------------

/** The today page's body alone (#1444 fragment payload). */
export function renderTodayBody(
  today: TodayView,
  attention: AttentionView,
  quota: QuotaView,
  coverage: SourceCoverage[],
): string {
  const walled = quota.pools.filter((p) => p.state === "walled");
  const alerts: string[] = [];
  const drewTotal = attention.drew.length + attention.drewExtra.length;
  if (drewTotal) {
    alerts.push(
      `<div class="banner drew"><strong>${drewTotal} item${drewTotal === 1 ? "" : "s"} in the user queue</strong> — <a href="/attention#drew">review them</a></div>`,
    );
  }
  if (walled.length) {
    alerts.push(
      `<div class="banner"><strong>Walled pools:</strong> ${walled.map((p) => esc(`${p.provider} (${p.plan})`)).join(", ")} — <a href="/quota">quota</a></div>`,
    );
  }
  // #1139: the derived view's banner names the section's TRUE state —
  // absent, invalid, and stale are different failures, and "has not
  // published yet" is only honest for one of them.
  const diag = today.sectionDiagnosis;
  const staleBanner =
    today.fromOrchestrator && diag?.state === "stale"
      ? `<div class="banner">The orchestration section is stale — its last run (${esc(diag.runId ?? "unknown")}) is ${esc(diag.generatedAt ? feedAgeText(diag.generatedAt) : "old")}; the measurements below are its last published state, not a current reading.</div>`
      : "";
  const provenanceLine = today.fromOrchestrator
    ? `<p class="muted">From the orchestration job's goal measurements (section written ${fmtDate(today.generatedAt)}). Machine-first: <a href="/api/today.json">/api/today.json</a> · <a href="/api/orchestration.json">/api/orchestration.json</a>.</p>${staleBanner}`
    : diag?.state === "invalid"
      ? `<div class="banner">Derived from the board, roadmap, and item activity — the orchestration section is present but failed validation (${esc((diag.failedChecks ?? []).join(", ") || "unknown check")}), so this is the aggregator's own derivation, not the orchestrator's measurement.</div>`
      : diag?.state === "stale"
        ? `<div class="banner">Derived from the board, roadmap, and item activity — the last orchestration section (${esc(diag.runId ?? "unknown run")}) is stale (${esc(diag.generatedAt ? feedAgeText(diag.generatedAt) : "old")}) and could not be used, so this is the aggregator's own derivation, not the orchestrator's measurement.</div>`
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
          const accomplished = (
            Array.isArray(rawAccomplished) ? rawAccomplished : rawAccomplished ? [rawAccomplished] : []
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
  return `${provenanceLine}${alerts.join("")}
<h2>Top objectives</h2>${objectives}
<h2>Project momentum</h2>${momentum}
${actions}${coverageHtml(coverage)}`;
}

/** The today page (#1444): full server render. */
export function renderToday(
  today: TodayView,
  attention: AttentionView,
  quota: QuotaView,
  coverage: SourceCoverage[],
  provenance: Provenance,
  detail?: ProvenanceDetail,
): string {
  return layout(
    "Today",
    `<h1>Today</h1>${renderTodayBody(today, attention, quota, coverage)}`,
    provenance,
    "/today",
    "",
    detail ? freshnessChips(detail, ["orchestration", "quota", "attention"]) : "",
  );
}

/** The Datastar shell for /today (#1444). */
export function renderTodayShell(provenance: Provenance, detail?: ProvenanceDetail): string {
  return layout(
    "Today",
    `<h1>Today</h1><noscript><meta http-equiv="refresh" content="0; url=/today?full=1"></noscript><div id="today-main" data-frag data-on-load="@get('/frag/today')" data-on-dashrefresh__window="@get('/frag/today')"><div class="skel">Loading today…</div></div>`,
    provenance,
    "/today",
    "",
    detail ? freshnessChips(detail, ["orchestration", "quota", "attention"]) : "",
  );
}

// --- Overview / front door (#1134) -------------------------------------------
// "/" is the stack front door: one glance at what needs Drew, what is
// moving, what is broken, and where the quotas sit. Every number here
// is derived from the same source views the dedicated pages render
// (attention, the Project pool, quota, health, the board) — the
// overview owns no data of its own, so its counts cannot drift from
// their source pages. deriveOverview is pure, so the HTML page and
// /api/overview.json serve the identical summary.

export interface OverviewData {
  attention: AttentionView;
  pool: ProjectPoolItem[];
  quota: QuotaView;
  health: HealthView;
  board: BoardItem[];
  /** Dashboard actions by object ref, so the needs-you summary applies
   *  the same render-time suppression as /attention (Codex review on
   *  PR 2424). Absent in tests and fixture renders: nothing suppressed. */
  actionsByObject?: Map<string, DashAction[]>;
}

export interface OverviewSummary {
  /** count is null when the tier source is not present: the queue is
   *  unknown, never a confident zero (Codex review on PR 2424). */
  needsYou: { count: number | null; tierState: string; top: { title: string; detail?: string }[] };
  projects: { repo: string; total: number; inProgress: number; inReview: number; done: number }[];
  quota: { provider: string; plan: string; state: QuotaState; worstPercent: number | null }[];
  health: { degraded: boolean; up: number; down: number; total: number; feedAgeMs: number | null };
  recentDone: { title: string; repo: string }[];
  inProgress: { title: string; repo: string }[];
}

export function deriveOverview(data: OverviewData): OverviewSummary {
  // The needs-you summary must agree with /attention on the same data:
  // suppressed entries (snoozed / dismissed / marked done — pending
  // counts, exactly as on the attention page) leave the count and the
  // top list, and when the tier source is not present the count is
  // unknown rather than an authoritative number over a partial queue.
  const now = Date.now();
  const suppressed = (objectRef: string): boolean =>
    cardState(data.actionsByObject?.get(objectRef), now).suppressed !== null;
  const drewVisible = data.attention.drew.filter((e) => !suppressed(sessionObjectRef(e.sessionId)));
  const drewExtraVisible = data.attention.drewExtra.filter((item) => !suppressed(tierItemObjectRef(item)));
  const tierPresent = data.attention.tierSource.present;
  const top: { title: string; detail?: string }[] = tierPresent
    ? [
        ...drewExtraVisible.map((t) => ({ title: t.title, detail: t.reason })),
        ...drewVisible.map((e) => ({ title: e.title ?? e.sessionId, detail: e.drewReason })),
      ].slice(0, 5)
    : [];
  const byRepo = new Map<string, { repo: string; total: number; inProgress: number; inReview: number; done: number }>();
  for (const item of data.pool) {
    const row = byRepo.get(item.repo) ?? { repo: item.repo, total: 0, inProgress: 0, inReview: 0, done: 0 };
    row.total += 1;
    if (item.status === "In Progress") row.inProgress += 1;
    else if (item.status === "In Review") row.inReview += 1;
    else if (item.status === "Done") row.done += 1;
    byRepo.set(item.repo, row);
  }
  const quota = data.quota.pools.map((pool) => {
    // #1134 review (Codex P2): derive from used/limit with the shared
    // helper — a window that carries `used` and `limit` but no explicit
    // `percent` must not silently drop out of the tightest-window figure.
    const percents = pool.windows.map((w) => windowPercent(w));
    return {
      provider: pool.provider,
      plan: pool.plan,
      state: pool.state,
      worstPercent: percents.length ? Math.max(...percents) : null,
    };
  });
  const services = data.health.services;
  const byRecency = (a: BoardItem, b: BoardItem) => (b.updatedAt ?? "").localeCompare(a.updatedAt ?? "");
  return {
    needsYou: {
      count: tierPresent ? drewVisible.length + drewExtraVisible.length : null,
      tierState: data.attention.tierSource.state,
      top,
    },
    projects: [...byRepo.values()].sort((a, b) => b.total - a.total || a.repo.localeCompare(b.repo)),
    quota,
    health: {
      degraded: data.health.degraded,
      up: services.filter((svc) => svc.ok === true).length,
      down: services.filter((svc) => svc.ok === false).length,
      total: services.length,
      feedAgeMs: data.health.feed.ageMs,
    },
    recentDone: data.board
      .filter((b) => b.status === "done")
      .sort(byRecency)
      .slice(0, 5)
      .map((b) => ({ title: b.title, repo: b.repo })),
    // In-flight board entries carry their real status in orchStatus
    // (in-progress | queued | blocked): only the running ones belong
    // under "In progress" — queued/blocked entries must not fill the
    // list ahead of running work (Codex review on PR 2424).
    inProgress: data.board
      .filter(
        (b) =>
          b.status === "in-progress" ||
          (b.status === "in-flight" && (b.orchStatus === undefined || b.orchStatus === "in-progress")),
      )
      .slice(0, 5)
      .map((b) => ({ title: b.title, repo: b.repo })),
  };
}

export function renderOverviewBody(summary: OverviewSummary): string {
  // #1134 review (Codex P2): the stale note is about the tier source, not
  // about the count, so an empty last published queue must still say it.
  const staleNote = `<p class="muted">Tier source stale — this is its last published state and may have moved since.</p>`;
  const needs =
    summary.needsYou.count === null
      ? `<div class="banner">Needs-you count <strong>unknown</strong> — the tier source is ${esc(summary.needsYou.tierState)}, so the orchestrator's published queues are not reflected here. <a href="/attention">Needs you</a></div>`
      : summary.needsYou.count
        ? `<div class="banner drew"><strong>${summary.needsYou.count} item${summary.needsYou.count === 1 ? "" : "s"} need you</strong> — <a href="/attention">review them</a></div>${summary.needsYou.tierState === "stale" ? staleNote : ""}`
        : `<p class="muted">Nothing needs you right now.</p>${summary.needsYou.tierState === "stale" ? staleNote : ""}`;
  const topItems = summary.needsYou.top.length
    ? `<ul>${summary.needsYou.top.map((t) => `<li><strong>${esc(t.title)}</strong>${t.detail ? ` — ${esc(t.detail)}` : ""}</li>`).join("")}</ul>`
    : "";
  const projectCards = summary.projects.length
    ? summary.projects
        .map(
          (proj) =>
            `<div class="card"><strong>${esc(proj.repo)}</strong> <span class="muted">${proj.total} tracked</span><br>${proj.inProgress} in progress · ${proj.inReview} in review · ${proj.done} done — <a href="/projects">projects</a></div>`,
        )
        .join("")
    : `<p class="muted">No Project pool items collected yet.</p>`;
  const quotaStrip = summary.quota.length
    ? summary.quota
        .map((pool) => {
          const pct = pool.worstPercent !== null ? ` · ${Math.round(pool.worstPercent)}% of tightest window` : "";
          return `<div class="card"><strong>${esc(pool.provider)}</strong> <span class="muted">${esc(pool.plan)}</span> <span class="badge ${esc(pool.state)}">${esc(pool.state)}</span>${pct}</div>`;
        })
        .join("")
    : `<p class="muted">No quota pools measured yet.</p>`;
  const healthStrip = `<div class="card"><span class="dot ${summary.health.down || summary.health.degraded ? "down" : "up"}"></span><strong>${summary.health.up} of ${summary.health.total} services up</strong>${summary.health.down ? ` · ${summary.health.down} down` : ""}${summary.health.degraded ? ` · <span class="badge stale">degraded</span>` : ""}${summary.health.feedAgeMs !== null ? ` · feed ${esc(ageMsText(summary.health.feedAgeMs))} old` : ""} — <a href="/health">health</a></div>`;
  const itemList = (items: { title: string; repo: string }[], empty: string) =>
    items.length
      ? `<ul>${items.map((i) => `<li>${esc(i.title)}${i.repo ? ` <span class="muted">${esc(i.repo)}</span>` : ""}</li>`).join("")}</ul>`
      : `<p class="muted">${empty}</p>`;
  return `<p class="muted">One glance at the whole stack: what needs you, what is moving, what is broken, and where the quotas sit. Every number is the same one its source page shows. Machine-first: <a href="/api/overview.json">/api/overview.json</a>.</p>
<h2>Needs you (${summary.needsYou.count ?? "unknown"})</h2>${needs}${topItems}
<h2>Projects</h2>${projectCards}
<h2>Quota</h2>${quotaStrip}
<h2>Health</h2>${healthStrip}
<h2>In progress</h2>${itemList(summary.inProgress, "Nothing in progress on the board.")}
<h2>Recently done</h2>${itemList(summary.recentDone, "Nothing done on the board yet.")}`;
}

/** The overview page (#1134): full server render. */
export function renderOverview(data: OverviewData, provenance: Provenance, detail?: ProvenanceDetail): string {
  return layout(
    "Overview",
    `<h1>Overview</h1>${renderOverviewBody(deriveOverview(data))}`,
    provenance,
    "/",
    "",
    detail ? freshnessChips(detail, ["board", "quota", "attention"]) : "",
  );
}

/** The Datastar shell for / (#1134, on the #1444 pattern). */
export function renderOverviewShell(provenance: Provenance, detail?: ProvenanceDetail): string {
  return layout(
    "Overview",
    `<h1>Overview</h1><noscript><meta http-equiv="refresh" content="0; url=/?full=1"></noscript><div id="overview-main" data-frag data-on-load="@get('/frag/overview')" data-on-dashrefresh__window="@get('/frag/overview')"><div class="skel">Loading the overview…</div></div>`,
    provenance,
    "/",
    "",
    detail ? freshnessChips(detail, ["board", "quota", "attention"]) : "",
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
  return `<div class="tblwrap"><table><caption>Domain register</caption><tr><th scope="col">Domain</th><th scope="col">Next renewal</th><th scope="col">Price (list)</th><th scope="col">Auto-renew</th><th scope="col">Plan / status</th><th scope="col">Review?</th></tr>${rows || '<tr><td colspan="6" class="muted">none</td></tr>'}</table></div>`;
}

export function renderDns(section: DnsSection | null, provenance: Provenance, detail?: ProvenanceDetail): string {
  const chips = detail ? freshnessChips(detail, ["dns"]) : "";
  if (!section) {
    return layout(
      "DNS register",
      `<h1>DNS / domain register</h1><div class="banner">The register has not reached the dashboard yet — the fleet-feed collector embeds it from #902's register file, and no section is in the current feed. The register of record is the file the #902 pipeline maintains; this page only renders it.</div>`,
      provenance,
      "/dns",
      "",
      chips,
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
  return layout("DNS register", body, provenance, "/dns", "", chips);
}

// --- AA cost explorer page (ainventory import data via the fleet feed) -----

/**
 * The Cost Explorer's client side: tabs, the shared filter bar, the
 * three frontier charts, and both tables, all rendered from the
 * #xpl-data JSON the server embeds (aaPoints + catalog, computed in
 * src/aa.ts). Dependency-free inline script, like COMMENT_SCRIPT.
 * Written without template literals on purpose — it lives inside one.
 */
const AA_EXPLORER_SCRIPT = `<script>
(function () {
  var dataEl = document.getElementById("xpl-data");
  if (!dataEl) return;
  var DATA = JSON.parse(dataEl.textContent);
  var AA_POINTS = DATA.points || [];
  var CAI_POINTS = DATA.caiPoints || [];
  var CATALOG = DATA.catalog || null;
  var POINTS = AA_POINTS;
  // Default benchmark vintage (2026-11-01, Drew): the payload carries
  // every vintage the AA import holds (v5 beside v4.3.2 today); ranking
  // across vintages compares scores from different methodologies, so
  // the first-load view pins the newest vintage present. "All
  // benchmarks" (bench "") stays one click away in the filter, and
  // saved setups / perspective switches keep their existing semantics.
  function benchVersionKey(b) {
    var m = /v(\d+(?:\.\d+)*)/.exec(b || "");
    return m ? m[1].split(".").map(function (n) { return parseInt(n, 10) || 0; }) : null;
  }
  function cmpVer(a, b) {
    var n = Math.max(a.length, b.length);
    for (var i = 0; i < n; i++) { var d = (a[i] || 0) - (b[i] || 0); if (d) return d; }
    return 0;
  }
  function latestBench(points) {
    var best = "", bestKey = null;
    (points || []).forEach(function (p) {
      var k = benchVersionKey(p.benchmark);
      if (!k) return;
      if (!bestKey || cmpVer(k, bestKey) > 0) { bestKey = k; best = p.benchmark; }
    });
    return best;
  }
  var PALETTE = ["#58a6ff","#3fb950","#e3b341","#f778ba","#bc8cff","#ffa657","#79c0ff","#56d364","#ff7b72","#a5d6ff","#7ee787","#f0883e","#d2a8ff","#e67e22","#2166ac","#b2182b","#1a9850","#386cb0","#7570b3","#66a61e","#e7298a","#bf812d","#0891b2","#ec4899","#2f9e6e","#d97706","#3b82f6","#a855f7","#94a3b8"];
  var famColor = {};
  var famList = [];
  // Colors are assigned over the union of both benchmark families so a
  // family keeps its color when the perspective switches.
  AA_POINTS.concat(CAI_POINTS).forEach(function (p) {
    if (!(p.family in famColor)) { famColor[p.family] = PALETTE[famList.length % PALETTE.length]; famList.push(p.family); }
  });
  var EORD = { "non-reasoning": 0, "low": 1, "medium": 2, "high": 3, "xhigh": 4, "max": 5, "reasoning": 6 };
  function eord(e) { var v = EORD[String(e || "").toLowerCase()]; return v == null ? 50 : v; }
  var ESHORT = { "medium": "med", "reasoning": "reas", "max (peak)": "max·pk", "max (off-peak, derived)": "max·offpk", "non-reasoning": "base" };
  function shortLabel(p) { var e = String(p.effort || ""); return p.family + " " + (ESHORT[e.toLowerCase()] || e); }
  function normName(s) {
    var t = String(s || "").toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
    for (var i = 0; i < 3; i++) t = t.replace(/(\d) (\d)/g, "$1.$2");
    return t.replace(/\s+/g, " ");
  }
  var goCapByName = {};
  (CATALOG || []).forEach(function (e) { if (e.goCapUsd != null) goCapByName[normName(e.name)] = e.goCapUsd; });
  function goCapFor(p) { var v = goCapByName[normName(p.family)]; return v == null ? null : v; }
  function fmtInt(v) { return String(Math.round(v)).replace(/\B(?=(\d{3})+(?!\d))/g, ","); }
  function tasksAt(cost, budget) { return cost ? Math.floor(budget / cost) : null; }
  function esc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }
  function fmt$(v) { return v == null ? "—" : "$" + (v < 0.01 ? v.toFixed(4) : v < 1 ? v.toFixed(3) : v.toFixed(2)); }
  function fmtT(v) {
    if (v == null) return "—";
    if (v < 60) return (Math.round(v * 10) / 10) + "s";
    return Math.floor(v / 60) + "m " + Math.round(v % 60) + "s";
  }
  function fmtN(v, d) { return v == null ? "—" : String(Math.round(v * Math.pow(10, d)) / Math.pow(10, d)); }
  function fmtCtx(v) {
    if (v == null) return "—";
    if (v >= 1000000) return (Math.round(v / 100000) / 10) + "M";
    if (v >= 1000) return Math.round(v / 1000) + "k";
    return String(v);
  }
  function cmpDesc(a, b) {
    if (a == null && b == null) return 0;
    if (a == null) return 1;
    if (b == null) return -1;
    return b - a;
  }
  function cmpAsc(a, b) {
    if (a == null && b == null) return 0;
    if (a == null) return 1;
    if (b == null) return -1;
    return a - b;
  }
  // --- benchmark perspective helpers -------------------------------------
  // The two benchmark families (AA Intelligence Index / Coding Agent
  // Index) are never converted into each other; the perspective only
  // swaps which point set is active and what its score is called.
  function scoreWord() { return S.persp === "cai" ? "CAI score" : "Intelligence"; }
  function scoreAxis() { return S.persp === "cai" ? "Coding Agent Index" : "Intelligence Index"; }
  function timeWord() { return S.persp === "cai" ? "agent wall time" : "time"; }
  /** Percentile rank of v within vals (100 = best). higherBetter picks
   * the direction; ties split. Null when the population is too small. */
  function pctRank(vals, v, higherBetter) {
    var xs = vals.filter(function (x) { return x != null; });
    if (xs.length < 2 || v == null) return null;
    var worse = 0, ties = 0;
    xs.forEach(function (x) {
      if (x === v) ties++;
      else if (higherBetter ? x < v : x > v) worse++;
    });
    return Math.round((100 * (worse + 0.5 * (ties - 1))) / (xs.length - 1));
  }
  /** Non-dominated subset: dims are {get, better:"high"|"low"}; a point
   * is kept unless another is at least as good on every dim and strictly
   * better on one. Points missing any dim are excluded. */
  function paretoSet(rows, dims) {
    var pool = rows.filter(function (p) { return dims.every(function (d) { return d.get(p) != null; }); });
    return pool.filter(function (p) {
      return !pool.some(function (q) {
        if (q === p) return false;
        var allGe = true, anyGt = false;
        dims.forEach(function (d) {
          var a = d.get(q), b = d.get(p);
          if (d.better === "high") { if (a < b) allGe = false; if (a > b) anyGt = true; }
          else { if (a > b) allGe = false; if (a < b) anyGt = true; }
        });
        return allGe && anyGt;
      });
    });
  }
  var S = {
    tab: location.hash === "#models" ? "models" : "aa",
    persp: "aa",
    xp: "api",
    q: "",
    creator: "",
    fam: "",
    bench: latestBench(AA_POINTS),
    efforts: {},
    effortAny: false,
    off: {},
    elig: false,
    pooledOnly: false,
    measuredOnly: false,
    hideDisabled: false,
    sel: null,
    hidden: [],
    sizeBy: {},
    tasksMode: "per",
    baseline: "",
    pfCut: "50",
    bandCount: 3,
    bandScale: 1,
    rec: { allow: 5, speedPriority: 85, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3, maxCost: "", maxSec: "" },
    pj: { turns: 3, inputTokens: 20000, answerTokens: 2000, thinkingTokens: 4000, cacheReadPct: 50, writesPct: 0, overheadSec: 10, window: "peak" },
    ineq: { mode: "off", combine: "and", conds: [{ metric: "score", op: ">=", value: "" }, { metric: "cost", op: "<=", value: "" }, { metric: "time", op: "<=", value: "" }] },
  };
  var XP_NOTES = {
    plan: "Plan perspective — plan quotas are sunk cost, so dollars are ignored: ranked on intelligence per second (speed + intelligence). Cost columns hidden.",
    api: "API perspective — ranked on intelligence per dollar and cost per task; the cost × score frontier is the value curve.",
    pertask: "Per-task perspective — ranked on total time and total cost to finish one task (the import's measured per-task figures, TTFT shown where timed).",
  };
  function basePoints() {
    return POINTS.filter(function (p) {
      if (S.q && (p.family + " " + (p.effort || "") + " " + p.creator).toLowerCase().indexOf(S.q) < 0) return false;
      if (S.creator && p.creator !== S.creator) return false;
      if (S.fam && p.family !== S.fam) return false;
      if (S.bench && p.benchmark !== S.bench) return false;
      if (S.effortAny && !S.efforts[p.effort || ""]) return false;
      if (S.elig && !p.eligible) return false;
      if (S.off[p.family]) return false;
      if (S.hidden.indexOf(p.key) >= 0) return false;
      return true;
    });
  }
  // --- inequality filters with Pareto-gap metrics (Site 1) ---------------
  // Gaps are measured against the observed frontier of the current scope
  // (all other filters applied): score gap = best score at or below this
  // cost minus this score; price premium = this cost over the cheapest
  // cost at or above this score; speed gap = score gap against time.
  function gapMetrics(rows) {
    var map = {};
    rows.forEach(function (p) {
      var g = { score: p.score, cost: p.cost, time: p.time, tps: p.tps, gapScore: null, gapCost: null, gapSpeed: null };
      if (p.cost != null && p.score != null) {
        var best = null, minCost = null;
        rows.forEach(function (q) {
          if (q.cost == null || q.score == null) return;
          if (q.cost <= p.cost && (best == null || q.score > best)) best = q.score;
          if (q.score >= p.score && (minCost == null || q.cost < minCost)) minCost = q.cost;
        });
        if (best != null) g.gapScore = best - p.score;
        if (minCost != null && minCost > 0) g.gapCost = p.cost / minCost;
      }
      if (p.time != null && p.score != null) {
        var bestT = null;
        rows.forEach(function (q) {
          if (q.time == null || q.score == null) return;
          if (q.time <= p.time && (bestT == null || q.score > bestT)) bestT = q.score;
        });
        if (bestT != null) g.gapSpeed = bestT - p.score;
      }
      map[p.key] = g;
    });
    return map;
  }
  var INEQ_OPS = {
    "<": function (a, b) { return a < b; },
    "<=": function (a, b) { return a <= b; },
    "=": function (a, b) { return a === b; },
    ">=": function (a, b) { return a >= b; },
    ">": function (a, b) { return a > b; },
  };
  function visPoints() {
    var base = basePoints();
    if (S.ineq.mode === "off") return base;
    var active = S.ineq.conds.filter(function (c) { return c.value !== "" && !isNaN(Number(c.value)); });
    if (!active.length) return base;
    var gaps = gapMetrics(base);
    return base.filter(function (p) {
      var g = gaps[p.key];
      var results = active.map(function (c) {
        var v = g ? g[c.metric] : null;
        if (v == null || !INEQ_OPS[c.op]) return false;
        return INEQ_OPS[c.op](v, Number(c.value));
      });
      var match = S.ineq.combine === "or" ? results.some(function (x) { return x; }) : results.every(function (x) { return x; });
      return S.ineq.mode === "reject" ? !match : match;
    });
  }
  function visCatalog() {
    if (!CATALOG) return [];
    return CATALOG.filter(function (e) {
      if (S.q && (e.id + " " + e.name + " " + e.providerName + " " + (e.plan || "")).toLowerCase().indexOf(S.q) < 0) return false;
      if (S.fam && e.provider !== S.fam) return false;
      if (S.pooledOnly && !e.routes.length) return false;
      if (S.measuredOnly && e.aaScore == null) return false;
      if (S.hideDisabled && e.disabled) return false;
      return true;
    });
  }
  function sortPoints(rows) {
    var out = rows.slice();
    if (S.xp === "plan") {
      out.sort(function (a, b) { return cmpDesc(a.scorePerSecond, b.scorePerSecond) || cmpDesc(a.score, b.score); });
    } else if (S.xp === "pertask") {
      out.sort(function (a, b) { return cmpAsc(a.time, b.time) || cmpAsc(a.cost, b.cost); });
    } else {
      out.sort(function (a, b) {
        var ka = a.pointsPerDollar != null ? a.pointsPerDollar : a.scorePerDollar;
        var kb = b.pointsPerDollar != null ? b.pointsPerDollar : b.scorePerDollar;
        return cmpDesc(ka, kb) || cmpDesc(a.score, b.score);
      });
    }
    return out;
  }
  function effCost(e) {
    return e.aaCostPerTask != null && e.multiplier ? e.aaCostPerTask / e.multiplier : null;
  }
  function sortCatalog(rows) {
    var out = rows.slice();
    if (S.xp === "plan") {
      out.sort(function (a, b) {
        var ka = a.aaScore != null && a.aaTimePerTask ? a.aaScore / a.aaTimePerTask : null;
        var kb = b.aaScore != null && b.aaTimePerTask ? b.aaScore / b.aaTimePerTask : null;
        return cmpDesc(ka, kb) || cmpDesc(a.aaScore, b.aaScore) || (a.id < b.id ? -1 : 1);
      });
    } else if (S.xp === "pertask") {
      out.sort(function (a, b) { return cmpAsc(a.aaTimePerTask, b.aaTimePerTask) || cmpDesc(a.aaScore, b.aaScore); });
    } else {
      out.sort(function (a, b) { return cmpAsc(effCost(a), effCost(b)) || cmpDesc(a.aaScore, b.aaScore) || (a.id < b.id ? -1 : 1); });
    }
    return out;
  }
  // --- charts (hand-rolled SVG, the reference design's three scatters) ---
  var W = 680, H = 400, ML = 46, MR = 14, MT = 12, MB = 40, PW = W - ML - MR, PH = H - MT - MB;
  function logPos(v, lo, hi) { return (Math.log10(Math.max(v, lo)) - Math.log10(lo)) / (Math.log10(hi) - Math.log10(lo)); }
  function fmtTickX(v, kind) {
    if (kind === "cost") return "$" + (v < 1 ? v : Math.round(v));
    return v < 60 ? v + "s" : v / 60 + "m";
  }
  function drawChart(id, cfg) {
    var svg = document.getElementById(id);
    if (!svg) return;
    var all = visPoints();
    var rows = all.filter(function (p) { return cfg.x(p) != null && cfg.y(p) != null; });
    var note = svg.parentElement.querySelector("[data-note]");
    if (note) {
      var missing = all.filter(function (p) { return cfg.x(p) == null || cfg.y(p) == null; });
      var names = missing.slice(0, 4).map(shortLabel).join(", ");
      note.textContent = rows.length + " of " + all.length + " shown — the rest lack a measured " + cfg.need + " in the import (nothing is estimated)" +
        (missing.length ? ": " + names + (missing.length > 4 ? " (+" + (missing.length - 4) + " more)" : "") + "." : ".");
    }
    if (!rows.length) { svg.innerHTML = ""; return; }
    var xs = rows.map(cfg.x), ys = rows.map(cfg.y);
    var xlo, xhi, ylo, yhi;
    if (cfg.xlog) {
      xlo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, xs)) * 2) / 2) / 1.4;
      xhi = Math.max.apply(null, xs) * 1.5;
    } else {
      xlo = Math.min.apply(null, xs); xhi = Math.max.apply(null, xs);
    }
    if (cfg.ylog) {
      ylo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, ys)) * 2) / 2) / 1.4;
      yhi = Math.max.apply(null, ys) * 1.5;
    } else {
      ylo = Math.max(0, Math.floor(Math.min.apply(null, ys) - 4));
      yhi = Math.ceil(Math.max.apply(null, ys) + 3);
    }
    var X = function (v) { return ML + (cfg.xlog ? logPos(v, xlo, xhi) : (v - xlo) / (xhi - xlo)) * PW; };
    var Y = function (v) { return MT + (1 - (cfg.ylog ? logPos(v, ylo, yhi) : (v - ylo) / (yhi - ylo))) * PH; };
    var g = "";
    // gridlines + ticks
    var xticks = [];
    if (cfg.xlog) {
      for (var e = Math.ceil(Math.log10(xlo)); e <= Math.log10(xhi); e++) xticks.push(Math.pow(10, e));
      if (cfg.xkind === "time") xticks = [10, 60, 300, 900, 1800, 3600].filter(function (v) { return v >= xlo && v <= xhi; });
    }
    xticks.forEach(function (v) {
      g += '<line x1="' + X(v) + '" y1="' + MT + '" x2="' + X(v) + '" y2="' + (MT + PH) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/>';
      g += '<text x="' + X(v) + '" y="' + (H - MB + 16) + '" font-size="9.5" fill="#8b949e" text-anchor="middle">' + fmtTickX(v, cfg.xkind) + "</text>";
    });
    var yticks = cfg.ylog ? [10, 60, 300, 900, 1800].filter(function (v) { return v >= ylo && v <= yhi; }) : [20, 30, 40, 50, 60].filter(function (v) { return v >= ylo && v <= yhi; });
    yticks.forEach(function (v) {
      g += '<line x1="' + ML + '" y1="' + Y(v) + '" x2="' + (ML + PW) + '" y2="' + Y(v) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/>';
      g += '<text x="' + (ML - 5) + '" y="' + (Y(v) + 3) + '" font-size="9.5" fill="#8b949e" text-anchor="end">' + (cfg.ylog ? fmtTickX(v, "time") : v) + "</text>";
    });
    g += '<line x1="' + ML + '" y1="' + (MT + PH) + '" x2="' + (ML + PW) + '" y2="' + (MT + PH) + '" stroke="#30363d"/><line x1="' + ML + '" y1="' + MT + '" x2="' + ML + '" y2="' + (MT + PH) + '" stroke="#30363d"/>';
    // bracket dividers from the reference design (cost tiers, time splits, II lines)
    (cfg.dividers || []).forEach(function (dv) {
      if (dv.axis === "x") {
        if (dv.v < xlo || dv.v > xhi) return;
        var dx = X(dv.v);
        g += '<line x1="' + dx + '" y1="' + MT + '" x2="' + dx + '" y2="' + (MT + PH) + '" stroke="#8b949e" stroke-width="1" stroke-dasharray="4 4" opacity="0.55"/>';
        if (dv.label) g += '<text x="' + (dx + 3) + '" y="' + (MT + 11) + '" font-size="9" fill="#8b949e">' + dv.label + "</text>";
      } else {
        if (dv.v < ylo || dv.v > yhi) return;
        var dy = Y(dv.v);
        g += '<line x1="' + ML + '" y1="' + dy + '" x2="' + (ML + PW) + '" y2="' + dy + '" stroke="#8b949e" stroke-width="1" stroke-dasharray="4 4" opacity="0.55"/>';
        if (dv.label) g += '<text x="' + (ML + 3) + '" y="' + (dy - 3) + '" font-size="9" fill="#8b949e">' + dv.label + "</text>";
      }
    });
    g += '<text x="' + (ML + PW / 2) + '" y="' + (H - 4) + '" font-size="10.5" fill="#8b949e" text-anchor="middle">' + cfg.xlabel + "</text>";
    g += '<text x="12" y="' + (MT + PH / 2) + '" font-size="10.5" fill="#8b949e" text-anchor="middle" transform="rotate(-90 12 ' + (MT + PH / 2) + ')">' + cfg.ylabel.replace("Intelligence Index", scoreAxis()) + "</text>";
    // effort ladder lines per family (kept readable: only when few points show)
    if (rows.length <= 60) {
      famList.forEach(function (fam) {
        if (S.off[fam]) return;
        var pts = rows.filter(function (p) { return p.family === fam; }).sort(function (a, b) { return eord(a.effort) - eord(b.effort); });
        if (pts.length < 2) return;
        var isDS = fam.indexOf("DeepSeek") === 0;
        g += '<polyline points="' + pts.map(function (p) { return X(cfg.x(p)).toFixed(1) + "," + Y(cfg.y(p)).toFixed(1); }).join(" ") + '" fill="none" stroke="' + famColor[fam] + '" stroke-width="' + (isDS ? 1.6 : 1) + '" opacity="' + (isDS ? 0.8 : 0.4) + '"' + (isDS ? ' stroke-dasharray="2.5 3"' : "") + "/>";
      });
    }
    // frontier curve for this chart's axis pair
    var fr = rows.filter(function (p) { return p[cfg.flag]; }).sort(function (a, b) { return cfg.x(a) - cfg.x(b); });
    if (fr.length > 1) {
      g += '<polyline points="' + fr.map(function (p) { return X(cfg.x(p)).toFixed(1) + "," + Y(cfg.y(p)).toFixed(1); }).join(" ") + '" fill="none" stroke="#e6edf3" stroke-width="1.4" opacity="0.65" stroke-dasharray="5 3"/>';
    }
    // dot sizing by the chart's third metric (reference design's toggle)
    var sizeVals = null, sizeLo = 0, sizeHi = 1;
    if (S.sizeBy[id] && cfg.sizeMetric) {
      sizeVals = rows.map(cfg.sizeMetric).filter(function (v) { return v != null; });
      if (sizeVals.length) {
        var tr = cfg.sizeLog ? sizeVals.map(function (v) { return Math.log10(v); }) : sizeVals;
        sizeLo = Math.min.apply(null, tr); sizeHi = Math.max.apply(null, tr);
      }
    }
    function radius(p, onF) {
      if (!sizeVals || !sizeVals.length || !cfg.sizeMetric) return onF ? 5.5 : 4.4;
      var v = cfg.sizeMetric(p);
      if (v == null) return 3;
      var t = cfg.sizeLog ? Math.log10(v) : v;
      var f = sizeHi > sizeLo ? Math.min(1, Math.max(0, (t - sizeLo) / (sizeHi - sizeLo))) : 0.5;
      return 1.6 + Math.pow(f, 1.5) * 8.4;
    }
    // 2026-11-01 (Drew, code#3335): every dot carries its family color;
    // frontier membership is emphasis (white ring, full opacity), not
    // the only color — on real data a chart's frontier can be a single
    // dominant point, which used to grey out the whole chart.
    rows.forEach(function (p) {
      var cx = X(cfg.x(p)), cy = Y(cfg.y(p));
      var onF = !!p[cfg.flag];
      var col = famColor[p.family];
      var r = radius(p, onF);
      var hollow = p.benchmark && p.benchmark.indexOf("AA") !== 0;
      var tip = p.family + " " + (p.effort || "") + " — " + scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + "/task" + (p.tps != null ? " · " + p.tps + " tok/s" : "") + (hollow ? " · " + p.benchmark + " basis" : "");
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="' + r.toFixed(1) + '" fill="' + col + '" fill-opacity="' + (hollow ? 0.25 : onF ? 0.95 : 0.45) + '" stroke="' + (S.sel === p.key || onF ? "#fff" : col) + '" stroke-width="' + (S.sel === p.key ? 2 : onF ? 1.8 : 1) + '"' + (hollow ? ' stroke-dasharray="2.5 1.8"' : "") + "><title>" + esc(tip) + "</title></circle>";
      if (onF || S.sel === p.key) {
        var anchorEnd = cx > ML + PW * 0.78;
        g += '<text data-key="' + esc(p.key) + '" x="' + (anchorEnd ? cx - r - 3 : cx + r + 3) + '" y="' + (cy + 3) + '" font-size="8.6" fill="' + (onF ? "#c9d1d9" : "#8b949e") + '" text-anchor="' + (anchorEnd ? "end" : "start") + '" style="cursor:pointer">' + esc(shortLabel(p)) + "</text>";
      }
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  var COST_DIVIDERS = [0.15, 0.75, 2.5].map(function (v, i) {
    return { axis: "x", v: v, label: ["Mid", "Premium", "Flagship"][i] };
  });
  var TIME_DIVIDERS_X = [60, 300, 900].map(function (v) { return { axis: "x", v: v, label: "" }; });
  var TIME_DIVIDERS_Y = [60, 300, 900].map(function (v) { return { axis: "y", v: v, label: "" }; });
  var CHARTS = [
    { id: "xpl-c1", x: function (p) { return p.time; }, y: function (p) { return p.score; }, xlog: true, ylog: false, xkind: "time", xlabel: "Seconds to finish one task (log)", ylabel: "Intelligence Index", flag: "frontierTime", need: "per-task time", sizeMetric: function (p) { return p.cost; }, sizeLog: true, dividers: TIME_DIVIDERS_X },
    { id: "xpl-c2", x: function (p) { return p.cost; }, y: function (p) { return p.score; }, xlog: true, ylog: false, xkind: "cost", xlabel: "Cost per task, USD (log)", ylabel: "Intelligence Index", flag: "frontierCost", need: "cost", sizeMetric: function (p) { return p.time; }, sizeLog: true, dividers: COST_DIVIDERS.concat([{ axis: "y", v: 40, label: "II 40" }, { axis: "y", v: 50, label: "II 50" }]) },
    { id: "xpl-c3", x: function (p) { return p.cost; }, y: function (p) { return p.time; }, xlog: true, ylog: true, xkind: "cost", xlabel: "Cost per task, USD (log)", ylabel: "Seconds per task (log)", flag: "frontierCostTime", need: "cost + per-task time", sizeMetric: function (p) { return p.score; }, sizeLog: false, dividers: COST_DIVIDERS.concat(TIME_DIVIDERS_Y) },
  ];
  var CHART_ORDER = { plan: ["xpl-c1", "xpl-c2", "xpl-c3"], api: ["xpl-c2", "xpl-c1", "xpl-c3"], pertask: ["xpl-c3", "xpl-c2", "xpl-c1"] };
  function drawCharts() {
    var wrap = document.getElementById("xpl-charts");
    var order = CHART_ORDER[S.xp];
    wrap.querySelectorAll(".chart").forEach(function (c) {
      c.classList.remove("primary");
      var idx = order.indexOf(c.getAttribute("data-chart"));
      c.style.order = idx >= 0 ? String(idx + 1) : "9";
    });
    var first = wrap.querySelector('[data-chart="' + order[0] + '"]');
    if (first) first.classList.add("primary");
    CHARTS.forEach(function (c) { drawChart(c.id, c); });
    drawTasks();
    drawMultiples();
    drawScores();
  }
  function downloadFile(name, text, type) {
    var blob = new Blob([text], { type: type || "text/plain" });
    var url = URL.createObjectURL(blob);
    var a = document.createElement("a");
    a.download = name;
    a.href = url;
    a.click();
    setTimeout(function () { URL.revokeObjectURL(url); }, 500);
  }
  function downloadPng(svgId) {
    var svg = document.getElementById(svgId);
    if (!svg) return;
    var vb = (svg.getAttribute("viewBox") || "0 0 680 400").split(" ");
    var w = Math.round(Number(vb[2]) * 2), h = Math.round(Number(vb[3]) * 2);
    var clone = svg.cloneNode(true);
    clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    clone.setAttribute("width", String(w));
    clone.setAttribute("height", String(h));
    var blob = new Blob([new XMLSerializer().serializeToString(clone)], { type: "image/svg+xml;charset=utf-8" });
    var url = URL.createObjectURL(blob);
    var img = new Image();
    img.onload = function () {
      var cv = document.createElement("canvas");
      cv.width = w; cv.height = h;
      var ctx = cv.getContext("2d");
      ctx.fillStyle = "#0d1117";
      ctx.fillRect(0, 0, w, h);
      ctx.drawImage(img, 0, 0, w, h);
      URL.revokeObjectURL(url);
      var a = document.createElement("a");
      a.download = "cost-explorer-" + svgId + ".png";
      a.href = cv.toDataURL("image/png");
      a.click();
    };
    img.src = url;
  }
  // --- chart 4: tasks completed vs duration (scatter-widget parity) ---
  var BUDGET_LADDER = [0.25, 1, 2, 5, 10, 20, 50, 100, 120, 200];
  function nominalBudget(cost) {
    var target = 30 * cost, best = BUDGET_LADDER[0], bd = Infinity;
    BUDGET_LADDER.forEach(function (b) {
      var d = Math.abs(Math.log10(b / target));
      if (d < bd) { bd = d; best = b; }
    });
    return best;
  }
  function drawTasks() {
    var svg = document.getElementById("xpl-c4");
    if (!svg) return;
    var all = visPoints();
    var rows = all.filter(function (p) { return p.cost != null && p.time != null; });
    var note = svg.parentElement.querySelector("[data-note]");
    if (note) {
      var missing = all.filter(function (p) { return p.cost == null || p.time == null; });
      note.textContent = rows.length + " of " + all.length + " shown — the rest lack a measured cost or per-task time: " +
        missing.slice(0, 4).map(shortLabel).join(", ") + (missing.length > 4 ? " (+" + (missing.length - 4) + " more)" : "") +
        ". Circles = a nominal budget ($0.25–$200, picked per model so its task count lands in range). ◆ = the model's published OpenCode Go monthly cap spent on that model alone (docs reading, 2026-10-20 — the fleet's workspaces appear to meter one pooled bar instead). Hollow = non-AA benchmark basis.";
    }
    document.querySelectorAll("[data-tmode]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-tmode") === S.tasksMode);
    });
    if (!rows.length) { svg.innerHTML = ""; return; }
    var per = S.tasksMode === "per";
    var pts = rows.map(function (p) {
      var budget = nominalBudget(p.cost);
      var tasks = Math.max(1, Math.floor(budget / p.cost));
      var cap = goCapFor(p);
      return { p: p, budget: budget, tasks: tasks, y: per ? p.time : tasks * p.time, goTasks: cap ? Math.floor(cap / p.cost) : null, goY: cap ? (per ? p.time : Math.floor(cap / p.cost) * p.time) : null };
    });
    var xs = pts.map(function (t) { return t.tasks; }).concat(pts.filter(function (t) { return t.goTasks; }).map(function (t) { return t.goTasks; }));
    var ys = pts.map(function (t) { return t.y; }).concat(pts.filter(function (t) { return t.goY != null; }).map(function (t) { return t.goY; }));
    var xlo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, xs)) * 2) / 2) / 1.4;
    var xhi = Math.max.apply(null, xs) * 1.6;
    var ylo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, ys)) * 2) / 2) / 1.4;
    var yhi = Math.max.apply(null, ys) * 1.5;
    var X = function (v) { return ML + logPos(v, xlo, xhi) * PW; };
    var Y = function (v) { return MT + (1 - logPos(v, ylo, yhi)) * PH; };
    var g = "";
    [1, 3, 10, 30, 100, 300, 1000, 3000].filter(function (v) { return v >= xlo && v <= xhi; }).forEach(function (v) {
      g += '<line x1="' + X(v) + '" y1="' + MT + '" x2="' + X(v) + '" y2="' + (MT + PH) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + X(v) + '" y="' + (H - MB + 16) + '" font-size="9.5" fill="#8b949e" text-anchor="middle">' + v + "</text>";
    });
    [10, 30, 100, 300, 1000, 3000, 10000, 43200, 140000].filter(function (v) { return v >= ylo && v <= yhi; }).forEach(function (v) {
      g += '<line x1="' + ML + '" y1="' + Y(v) + '" x2="' + (ML + PW) + '" y2="' + Y(v) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + (ML - 5) + '" y="' + (Y(v) + 3) + '" font-size="9.5" fill="#8b949e" text-anchor="end">' + fmtT(v) + "</text>";
    });
    g += '<line x1="' + ML + '" y1="' + (MT + PH) + '" x2="' + (ML + PW) + '" y2="' + (MT + PH) + '" stroke="#30363d"/><line x1="' + ML + '" y1="' + MT + '" x2="' + ML + '" y2="' + (MT + PH) + '" stroke="#30363d"/>';
    g += '<text x="' + (ML + PW / 2) + '" y="' + (H - 4) + '" font-size="10.5" fill="#8b949e" text-anchor="middle">tasks completed (log scale)</text>';
    g += '<text x="12" y="' + (MT + PH / 2) + '" font-size="10.5" fill="#8b949e" text-anchor="middle" transform="rotate(-90 12 ' + (MT + PH / 2) + ')">' + (per ? "duration per task" : "total time, all tasks back-to-back") + "</text>";
    pts.forEach(function (t) {
      var p = t.p, cx = X(t.tasks), cy = Y(t.y), col = famColor[p.family];
      var hollow = p.benchmark && p.benchmark.indexOf("AA") !== 0;
      var tip = shortLabel(p) + " — $" + t.budget + " nominal → " + t.tasks + " tasks · " + fmtT(p.time) + "/task · " + fmt$(p.cost) + "/task · " + scoreWord() + " " + fmtN(p.score, 1) + (t.goTasks ? " · published Go cap buys " + fmtInt(t.goTasks) + " tasks" : "");
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="5.5" fill="' + col + '" fill-opacity="' + (hollow ? 0.25 : 0.9) + '" stroke="' + (S.sel === p.key ? "#fff" : col) + '" stroke-width="' + (S.sel === p.key ? 2 : 1.5) + '"' + (hollow ? ' stroke-dasharray="2.5 1.8"' : "") + "><title>" + esc(tip) + "</title></circle>";
      if (t.goTasks) {
        var gx = X(t.goTasks), gy = Y(t.goY);
        g += '<rect data-key="' + esc(p.key) + '" x="' + (gx - 5).toFixed(1) + '" y="' + (gy - 5).toFixed(1) + '" width="10" height="10" transform="rotate(45 ' + gx.toFixed(1) + " " + gy.toFixed(1) + ')" fill="' + col + '" fill-opacity="0.95" stroke="#0d1117" stroke-width="1.2" style="cursor:pointer"><title>' + esc(tip) + "</title></rect>";
      }
      if (p.frontierCostTime || S.sel === p.key) {
        var anchorEnd = cx > ML + PW * 0.78;
        g += '<text data-key="' + esc(p.key) + '" x="' + (anchorEnd ? cx - 9 : cx + 9) + '" y="' + (cy + 3) + '" font-size="8.6" fill="#c9d1d9" text-anchor="' + (anchorEnd ? "end" : "start") + '" style="cursor:pointer">' + esc(shortLabel(p)) + "</text>";
      }
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  // --- chart 5: multiples of a baseline (multiples-widget parity) ---
  function baselinePoint() {
    var pool = POINTS.filter(function (p) { return p.cost != null && p.time != null && p.score != null && S.hidden.indexOf(p.key) < 0; });
    if (!pool.length) return null;
    var chosen = pool.find(function (p) { return p.key === S.baseline; });
    if (chosen) return chosen;
    var lunaLow = pool.find(function (p) { return p.family.indexOf("Luna") >= 0 && String(p.effort).toLowerCase() === "low"; });
    if (lunaLow) return lunaLow;
    return pool.slice().sort(function (a, b) { return a.cost - b.cost; })[0];
  }
  function buildBaselineOptions() {
    var sel = document.getElementById("xf-baseline");
    if (!sel) return;
    var pool = POINTS.filter(function (p) { return p.cost != null && p.time != null && p.score != null; })
      .sort(function (a, b) { return a.family < b.family ? -1 : a.family > b.family ? 1 : eord(a.effort) - eord(b.effort); });
    var base = baselinePoint();
    sel.innerHTML = pool.map(function (p) {
      return '<option value="' + esc(p.key) + '"' + (base && p.key === base.key ? " selected" : "") + ">" + esc(shortLabel(p)) + "</option>";
    }).join("");
  }
  function drawMultiples() {
    var svg = document.getElementById("xpl-c5");
    if (!svg) return;
    var base = baselinePoint();
    var note = svg.parentElement.querySelector("[data-note]");
    var all = visPoints();
    var rows = base ? all.filter(function (p) { return p.cost != null && p.time != null && p.score != null; }) : [];
    if (note) {
      note.textContent = base
        ? "Baseline: " + shortLabel(base) + " = 1× on all three measures (" + fmt$(base.cost) + "/task, " + fmtT(base.time) + "/task, " + scoreWord() + " " + fmtN(base.score, 1) + "). " + rows.length + " of " + all.length + " configurations have all three measures."
        : "No configuration with cost, time and score is visible — the multiples view needs all three.";
    }
    renderMultTable(base, rows);
    if (!base || !rows.length) { svg.innerHTML = ""; return; }
    var pts = rows.map(function (p) {
      return { p: p, cx: p.cost / base.cost, ty: p.time / base.time, sx: p.score / base.score };
    });
    var xs = pts.map(function (t) { return t.cx; }), ys = pts.map(function (t) { return t.ty; });
    var xlo = Math.min(0.8, Math.min.apply(null, xs) / 1.4), xhi = Math.max.apply(null, xs) * 1.5;
    var ylo = Math.min(0.8, Math.min.apply(null, ys) / 1.4), yhi = Math.max.apply(null, ys) * 1.5;
    var X = function (v) { return ML + logPos(v, xlo, xhi) * PW; };
    var Y = function (v) { return MT + (1 - logPos(v, ylo, yhi)) * PH; };
    var g = "";
    [1, 3, 10, 30, 100, 300, 1000].filter(function (v) { return v >= xlo && v <= xhi; }).forEach(function (v) {
      g += '<line x1="' + X(v) + '" y1="' + MT + '" x2="' + X(v) + '" y2="' + (MT + PH) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + X(v) + '" y="' + (H - MB + 16) + '" font-size="9.5" fill="#8b949e" text-anchor="middle">' + v + "×</text>";
    });
    [1, 3, 10, 30, 100, 300].filter(function (v) { return v >= ylo && v <= yhi; }).forEach(function (v) {
      g += '<line x1="' + ML + '" y1="' + Y(v) + '" x2="' + (ML + PW) + '" y2="' + Y(v) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + (ML - 5) + '" y="' + (Y(v) + 3) + '" font-size="9.5" fill="#8b949e" text-anchor="end">' + v + "×</text>";
    });
    if (xlo < 1 && xhi > 1) g += '<line x1="' + X(1) + '" y1="' + MT + '" x2="' + X(1) + '" y2="' + (MT + PH) + '" stroke="#58a6ff" stroke-width="1" opacity="0.55"/>';
    if (ylo < 1 && yhi > 1) g += '<line x1="' + ML + '" y1="' + Y(1) + '" x2="' + (ML + PW) + '" y2="' + Y(1) + '" stroke="#58a6ff" stroke-width="1" opacity="0.55"/>';
    g += '<line x1="' + ML + '" y1="' + (MT + PH) + '" x2="' + (ML + PW) + '" y2="' + (MT + PH) + '" stroke="#30363d"/><line x1="' + ML + '" y1="' + MT + '" x2="' + ML + '" y2="' + (MT + PH) + '" stroke="#30363d"/>';
    g += '<text x="' + (ML + PW / 2) + '" y="' + (H - 4) + '" font-size="10.5" fill="#8b949e" text-anchor="middle">cost multiple of the baseline (log scale)</text>';
    g += '<text x="12" y="' + (MT + PH / 2) + '" font-size="10.5" fill="#8b949e" text-anchor="middle" transform="rotate(-90 12 ' + (MT + PH / 2) + ')">duration multiple of the baseline (lower = faster)</text>';
    pts.forEach(function (t) {
      var p = t.p, cx = X(t.cx), cy = Y(t.ty), col = famColor[p.family];
      var hollow = p.benchmark && p.benchmark.indexOf("AA") !== 0;
      var tip = shortLabel(p) + " — costs " + fmtN(t.cx, 2) + "× baseline (" + fmt$(p.cost) + "/task), takes " + fmtN(t.ty, 2) + "× as long (" + fmtT(p.time) + "/task), scores " + fmtN(t.sx, 2) + "× (" + scoreWord() + " " + fmtN(p.score, 1) + ")";
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="5.5" fill="' + col + '" fill-opacity="' + (hollow ? 0.25 : 0.9) + '" stroke="' + (S.sel === p.key ? "#fff" : col) + '" stroke-width="' + (S.sel === p.key ? 2 : 1.5) + '"' + (hollow ? ' stroke-dasharray="2.5 1.8"' : "") + "><title>" + esc(tip) + "</title></circle>";
      if (p.key === base.key) g += '<circle cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="10" fill="none" stroke="#58a6ff" stroke-width="1.5"/>';
      if (p.frontierCostTime || S.sel === p.key || p.key === base.key) {
        var anchorEnd = cx > ML + PW * 0.78;
        g += '<text data-key="' + esc(p.key) + '" x="' + (anchorEnd ? cx - 9 : cx + 9) + '" y="' + (cy + 3) + '" font-size="8.6" fill="#c9d1d9" text-anchor="' + (anchorEnd ? "end" : "start") + '" style="cursor:pointer">' + esc(shortLabel(p)) + "</text>";
      }
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  function renderMultTable(base, rows) {
    var tb = document.getElementById("xpl-mtrows");
    if (!tb) return;
    if (!base) { tb.innerHTML = ""; return; }
    var sorted = rows.slice().sort(function (a, b) { return a.cost / base.cost - b.cost / base.cost; });
    tb.innerHTML = sorted.map(function (p) {
      return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '"><td><strong>' + esc(p.family) + "</strong></td><td>" + esc(p.effort || "—") + '</td><td class="num">' + fmtN(p.score, 1) + '</td><td class="num">' + fmtN(p.score / base.score, 2) + '×</td><td class="num xpl-costcol">' + fmt$(p.cost) + '</td><td class="num xpl-costcol">' + fmtN(p.cost / base.cost, 2) + '×</td><td class="num">' + fmtT(p.time) + '</td><td class="num">' + fmtN(p.time / base.time, 2) + '×</td><td>' + esc(p.benchmark || "—") + "</td></tr>";
    }).join("");
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
    markSelectedRow();
  }
  // --- chart 6: scores only, incl. configurations with no published cost ---
  function drawScores() {
    var svg = document.getElementById("xpl-c6");
    if (!svg) return;
    var all = visPoints();
    var rows = all.filter(function (p) { return p.score != null; }).sort(function (a, b) { return b.score - a.score; });
    var note = svg.parentElement.querySelector("[data-note]");
    var top = rows.slice(0, 40);
    if (note) {
      note.textContent = "Top " + top.length + " of " + rows.length + " scored configurations (of " + all.length + " visible). Score-only view: configurations with no published cost belong here even though no cost chart can plot them.";
    }
    if (!top.length) { svg.innerHTML = ""; return; }
    var rowH = 17, topPad = 8, botPad = 22;
    var hgt = topPad + top.length * rowH + botPad;
    svg.setAttribute("viewBox", "0 0 680 " + hgt);
    var labelW = 218, plotW = 680 - labelW - 52;
    var maxScore = Math.max.apply(null, top.map(function (p) { return p.score; })) * 1.05;
    var g = "";
    top.forEach(function (p, i) {
      var y = topPad + i * rowH;
      var w = Math.max(2, (p.score / maxScore) * plotW);
      var col = famColor[p.family];
      var label = shortLabel(p) + (p.cost == null ? " (cost n/a)" : "");
      g += '<text data-key="' + esc(p.key) + '" x="' + (labelW - 6) + '" y="' + (y + 11) + '" font-size="9.5" fill="#c9d1d9" text-anchor="end" style="cursor:pointer">' + esc(label) + "</text>";
      g += '<rect data-key="' + esc(p.key) + '" x="' + labelW + '" y="' + (y + 2) + '" width="' + w.toFixed(1) + '" height="11" fill="' + col + '" fill-opacity="' + (S.sel === p.key ? 1 : 0.75) + '" style="cursor:pointer"><title>' + esc(shortLabel(p) + " — " + scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + "/task") + "</title></rect>";
      g += '<text x="' + (labelW + w + 5) + '" y="' + (y + 11) + '" font-size="9.5" fill="#8b949e">' + fmtN(p.score, 1) + "</text>";
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  // --- hidden tray (reference design's hide-a-configuration flow) ---
  function renderTray() {
    var tray = document.getElementById("xpl-tray");
    if (!tray) return;
    var chips = document.getElementById("xpl-traychips");
    if (!S.hidden.length) { tray.hidden = true; chips.innerHTML = ""; return; }
    tray.hidden = false;
    chips.innerHTML = "";
    S.hidden.forEach(function (key) {
      var p = POINTS.find(function (x) { return x.key === key; });
      if (!p) return;
      var s = document.createElement("span");
      s.className = "hchip";
      s.textContent = shortLabel(p);
      var b = document.createElement("button");
      b.type = "button";
      b.textContent = "×";
      b.title = "Bring back";
      b.addEventListener("click", function () {
        S.hidden = S.hidden.filter(function (h) { return h !== key; });
        render();
      });
      s.appendChild(b);
      chips.appendChild(s);
    });
  }
  function renderLegend() {
    var lg = document.getElementById("xpl-legend");
    if (!lg) return;
    lg.innerHTML = "";
    famList.forEach(function (fam) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "xchip" + (S.off[fam] ? " off" : "");
      b.innerHTML = '<span class="dot" style="background:' + famColor[fam] + '"></span>' + esc(fam);
      b.addEventListener("click", function () {
        if (S.off[fam]) delete S.off[fam]; else S.off[fam] = true;
        render();
      });
      lg.appendChild(b);
    });
  }
  function showDetail(html) {
    var box = document.getElementById("xpl-detail");
    if (!box) return;
    if (!html) { box.style.display = "none"; box.innerHTML = ""; return; }
    box.innerHTML = html;
    box.style.display = "block";
  }
  function selectPoint(key) {
    S.sel = key;
    var p = POINTS.find(function (x) { return x.key === key; });
    if (!p) return;
    var beaten = p.beatenBy && p.beatenBy.length
      ? p.beatenBy.slice(0, 4).join("; ") + (p.beatenBy.length > 4 ? " (+" + (p.beatenBy.length - 4) + " more)" : "")
      : "nothing on all three axes — it is on the frontier";
    var budgets = p.cost
      ? "<br>Tasks per budget at this cost: $60 → " + fmtInt(tasksAt(p.cost, 60)) + " · $90 → " + fmtInt(tasksAt(p.cost, 90)) + " · $120 → " + fmtInt(tasksAt(p.cost, 120)) + " · $180 → " + fmtInt(tasksAt(p.cost, 180))
      : "";
    var cap = goCapFor(p);
    var goLine = cap && p.cost
      ? '<br><span class="muted">Published OpenCode Go cap for this model: $' + cap + "/mo → ~" + fmtInt(Math.floor(cap / p.cost)) + " tasks at this AA cost (docs reading — the fleet's workspaces appear to meter pooled).</span>"
      : "";
    var caiLine = "";
    if (p.harness != null || p.components) {
      var pct = function (v) { return v == null ? "—" : fmtN(v * 100, 1) + "%"; };
      caiLine = "<br>" +
        (p.harness ? "Harness: " + esc(p.harness) + " · " : "") +
        (p.tokensPerTask != null ? fmtInt(p.tokensPerTask) + " tokens/task" : "") +
        (p.components ? "<br>Components: DeepSWE " + pct(p.components.deepSwe) + " · Terminal-Bench " + pct(p.components.terminalBench) + " · SWE-Atlas-QnA " + pct(p.components.sweAtlas) : "");
    }
    showDetail(
      "<strong>" + esc(p.family) + " — " + esc(p.effort || "—") + "</strong> <span class='muted'>" + esc(p.creator) + "</span><br>" +
      scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + " per task" +
      (p.ttft != null ? " · TTFT " + fmtT(p.ttft) : "") + (p.tps != null ? " · " + p.tps + " tok/s" : "") + budgets + "<br>" +
      '<span class="muted">Beaten on score + cost + time at once by: ' + esc(beaten) + "</span>" + caiLine + goLine +
      '<br><button type="button" id="xpl-hide" class="xpl-ctl-btn">Hide this configuration</button>',
    );
    var hideBtn = document.getElementById("xpl-hide");
    if (hideBtn) {
      hideBtn.addEventListener("click", function () {
        if (S.hidden.indexOf(p.key) < 0) S.hidden.push(p.key);
        S.sel = null;
        showDetail(null);
        render();
      });
    }
    drawCharts();
    markSelectedRow();
  }
  function selectCatalog(id) {
    S.sel = id;
    var e = (CATALOG || []).find(function (x) { return x.id === id; });
    if (!e) return;
    showDetail(
      "<strong>" + esc(e.name) + "</strong> <span class='muted'><code>" + esc(e.id) + "</code></span><br>" +
      (e.plan ? "Plan: <strong>" + esc(e.plan) + "</strong>" + (e.multiplier ? " — " + e.multiplier + "× list value" : "") + "<br>" : "") +
      (e.planNote ? '<span class="muted">' + esc(e.planNote) + "</span><br>" : "") +
      "Pool seats: " + (e.routes.length ? e.routes.map(function (r) { return "<code>" + esc(r) + "</code>"; }).join(" ") : '<span class="muted">none — no route sends it work</span>') +
      (e.goCapUsd != null && e.aaCostPerTask ? '<br><span class="muted">Published OpenCode Go cap: $' + e.goCapUsd + "/mo → ~" + fmtInt(Math.floor(e.goCapUsd / e.aaCostPerTask)) + " AA tasks at the measured cost (docs reading — the fleet's workspaces appear to meter pooled).</span>" : "") +
      (e.disabled ? '<br><span class="muted">Provider is disabled in the fleet config — catalogued but not callable.</span>' : "") +
      (function () {
        var proj = project(e);
        if (!proj.band) return "";
        var P = S.pj;
        return '<br><span class="muted">Your projected task (ESTIMATED, NOT MEASURED): ' + P.turns + " turns × " + fmtInt(Number(P.inputTokens) || 0) + " input + " + fmtInt(Number(P.answerTokens) || 0) + " answer + " + fmtInt(Number(P.thinkingTokens) || 0) + " thinking tokens/turn · " + P.cacheReadPct + "% cache reads · rate window: " + esc(proj.band.window || "flat") + " (" + fmt$(proj.band.input) + " in / " + fmt$(proj.band.output) + " out per 1M) → " + (proj.cost != null ? "~" + fmt$(proj.cost) + "/task" : "cost unknown — a required rate is not recorded") + (proj.time != null ? " · ~" + fmtT(proj.time) + "/task" : "") + ".</span>";
      })(),
    );
    markSelectedRow();
  }
  function markSelectedRow() {
    document.querySelectorAll(".xpl-table tr").forEach(function (tr) {
      tr.style.background = tr.getAttribute("data-sel") === S.sel ? "#1f6feb22" : "";
    });
  }
  // --- Pareto frontier tables (Site 1's Pareto frontiers tab) ------------
  // Each list keeps variants not dominated on its named dimensions;
  // percentile ranks use the current filtered population (100% = best).
  function pfBind(tb) {
    if (!tb) return;
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
  }
  function pfMetricCell(v, pct, fmt) {
    return v == null ? "—" : fmt(v) + (pct != null ? ' <span class="muted">· ' + pct + "%</span>" : "");
  }
  function pfNameCell(p) {
    return '<td><strong>' + esc(p.family) + '</strong> <span class="muted">' + esc(p.effort || "") + (p.harness ? " · " + esc(p.harness) : "") + "</span></td>";
  }
  function renderPareto() {
    var rows = visPoints();
    var scores = rows.map(function (p) { return p.score; });
    var times = rows.map(function (p) { return p.time; });
    var costs = rows.map(function (p) { return p.cost; });
    function stdRow(p) {
      return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '">' + pfNameCell(p) +
        '<td class="num">' + pfMetricCell(p.score, pctRank(scores, p.score, true), function (v) { return fmtN(v, 1); }) + "</td>" +
        '<td class="num">' + pfMetricCell(p.time, pctRank(times, p.time, false), fmtT) + "</td>" +
        '<td class="num xpl-costcol">' + pfMetricCell(p.cost, pctRank(costs, p.cost, false), fmt$) + "</td></tr>";
    }
    var gScore = function (p) { return p.score; }, gTime = function (p) { return p.time; }, gCost = function (p) { return p.cost; };
    var ssFrontier = paretoSet(rows, [{ get: gScore, better: "high" }, { get: gTime, better: "low" }]);
    var byScore = ssFrontier.slice().sort(function (a, b) { return cmpDesc(a.score, b.score) || cmpAsc(a.time, b.time); });
    var bySpeed = ssFrontier.slice().sort(function (a, b) { return cmpAsc(a.time, b.time) || cmpDesc(a.score, b.score); });
    var cut = Number(S.pfCut || 50);
    var three = paretoSet(rows, [{ get: gScore, better: "high" }, { get: gCost, better: "low" }, { get: gTime, better: "low" }])
      .filter(function (p) {
        if (!cut) return true;
        var ps = pctRank(scores, p.score, true), pt = pctRank(times, p.time, false);
        return ps != null && pt != null && ps >= cut && pt >= cut;
      })
      .sort(function (a, b) { return cmpAsc(a.cost, b.cost) || cmpDesc(a.score, b.score); });
    var psFrontier = paretoSet(rows, [{ get: gCost, better: "low" }, { get: gTime, better: "low" }])
      .sort(function (a, b) { return cmpAsc(a.cost, b.cost) || cmpAsc(a.time, b.time); });
    var tb;
    tb = document.getElementById("xpl-pf-score"); if (tb) { tb.innerHTML = byScore.map(stdRow).join(""); pfBind(tb); }
    tb = document.getElementById("xpl-pf-speed"); if (tb) { tb.innerHTML = bySpeed.map(stdRow).join(""); pfBind(tb); }
    tb = document.getElementById("xpl-pf-price"); if (tb) { tb.innerHTML = three.map(stdRow).join(""); pfBind(tb); }
    tb = document.getElementById("xpl-pf-ps");
    if (tb) {
      tb.innerHTML = psFrontier.map(function (p) {
        return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '">' + pfNameCell(p) +
          '<td class="num">' + pfMetricCell(p.time, pctRank(times, p.time, false), fmtT) + "</td>" +
          '<td class="num xpl-costcol">' + pfMetricCell(p.cost, pctRank(costs, p.cost, false), fmt$) + "</td>" +
          '<td class="num">' + (p.score == null ? "—" : fmtN(p.score, 1)) + "</td></tr>";
      }).join("");
      pfBind(tb);
    }
    var note = document.getElementById("xpl-pf-count");
    if (note) {
      note.textContent = "Score–speed frontier: " + ssFrontier.length + " variants · three-way frontier" +
        (cut ? " (score and speed at or above the " + cut + "th percentile)" : " (no percentile cutoff)") + ": " + three.length +
        " · price–speed trade-offs: " + psFrontier.length + ". Missing measures are excluded only from a frontier that needs them.";
    }
    markSelectedRow();
  }
  // --- Recommendations (Site 1's shortlist builder) ------------------------
  // Score bands are non-overlapping, from the visible population, before
  // tradeoff limits. Within a band: absolute limits first, then relative
  // price/speed limits against the cheapest and fastest near-top models;
  // survivors rank by (relative time)^speed-weight x (relative
  // price)^(1-speed-weight); scores inside the window break ties.
  function computeRecs() {
    var R = S.rec;
    var rows = visPoints();
    var scored = rows.filter(function (p) { return p.score != null; });
    if (!scored.length) return { bands: [], empty: "No scored configurations are visible under the current filters." };
    var lo = Math.min.apply(null, scored.map(function (p) { return p.score; }));
    var hi = Math.max.apply(null, scored.map(function (p) { return p.score; }));
    var n = Math.max(2, Math.min(8, Number(S.bandCount) || 3));
    var w = (hi - lo) / n || 1;
    var bands = [];
    for (var i = 0; i < n; i++) {
      var bLo = lo + i * w, bHi = i === n - 1 ? hi : lo + (i + 1) * w;
      var members = scored.filter(function (p) { return p.score >= bLo && (i === n - 1 ? p.score <= bHi : p.score < bHi); });
      if (!members.length) continue;
      var best = Math.max.apply(null, members.map(function (p) { return p.score; }));
      var decision = {};
      var candidates = [];
      members.forEach(function (p) {
        if (p.cost == null || p.time == null) decision[p.key] = "Missing selected cost or speed";
        else candidates.push(p);
      });
      var ref = candidates.slice().sort(function (a, b) { return cmpDesc(a.score, b.score) || cmpAsc(a.cost, b.cost); })[0] || null;
      var maxCost = R.maxCost === "" || R.maxCost == null ? null : Number(R.maxCost);
      var maxTime = R.maxSec === "" || R.maxSec == null ? null : Number(R.maxSec);
      var nearTop = [];
      candidates.forEach(function (p) {
        if (p.score < best - Number(R.allow)) decision[p.key] = "Below near-top score floor";
        else if (maxCost != null && p.cost > maxCost) decision[p.key] = "Above the absolute price limit";
        else if (maxTime != null && p.time > maxTime) decision[p.key] = "Above the absolute time limit";
        else nearTop.push(p);
      });
      var picks = [];
      var cheapest = null, fastest = null, priceCeil = null, timeCeil = null;
      if (nearTop.length) {
        cheapest = Math.min.apply(null, nearTop.map(function (p) { return p.cost; }));
        fastest = Math.min.apply(null, nearTop.map(function (p) { return p.time; }));
        priceCeil = cheapest * (1 + Number(R.priceAllow) / 100);
        timeCeil = fastest * Number(R.slowAllow);
        var survivors = [];
        nearTop.forEach(function (p) {
          if (p.cost > priceCeil) decision[p.key] = "Above relative price allowance";
          else if (p.time > timeCeil) decision[p.key] = "Slower than relative speed limit";
          else survivors.push(p);
        });
        var kept = survivors.filter(function (p) {
          if (R.dominance === "none") return true;
          var dominated = survivors.some(function (q) {
            if (q === p) return false;
            if (R.dominance === "lab" && q.creator !== p.creator) return false;
            return q.score >= p.score && q.cost <= p.cost && q.time <= p.time && (q.score > p.score || q.cost < p.cost || q.time < p.time);
          });
          if (dominated) decision[p.key] = R.dominance === "lab" ? "Dominated within its lab" : "Dominated on score, price and speed";
          return !dominated;
        });
        var sw = Number(R.speedPriority) / 100;
        var ranked = kept.map(function (p) {
          return { p: p, factor: Math.pow(p.time / fastest, sw) * Math.pow(p.cost / cheapest, 1 - sw) };
        }).sort(function (a, b) { return a.factor - b.factor || cmpDesc(a.p.score, b.p.score); });
        var bestFactor = ranked.length ? ranked[0].factor : null;
        var pickedLabs = {};
        var passes = R.diversity === "labsfirst" ? 2 : 1;
        for (var pass = 0; pass < passes && picks.length < Number(R.perBand); pass++) {
          ranked.forEach(function (r) {
            if (picks.length >= Number(R.perBand)) return;
            if (picks.some(function (x) { return x.p.key === r.p.key; })) return;
            if (bestFactor != null && r.factor > bestFactor * Number(R.altWindow)) return;
            if (R.diversity === "lab" && pickedLabs[r.p.creator]) return;
            if (R.diversity === "labsfirst" && pass === 0 && pickedLabs[r.p.creator]) return;
            picks.push(r);
            pickedLabs[r.p.creator] = true;
          });
        }
        ranked.forEach(function (r) {
          if (picks.some(function (x) { return x.p.key === r.p.key; })) { decision[r.p.key] = "Recommended"; return; }
          if (decision[r.p.key]) return;
          if (bestFactor != null && r.factor > bestFactor * Number(R.altWindow)) decision[r.p.key] = "Outside the alternative window";
          else if (R.diversity === "lab" && pickedLabs[r.p.creator]) decision[r.p.key] = "Not selected — one choice per lab";
          else decision[r.p.key] = "Not selected — band cap reached";
        });
      }
      bands.push({
        label: "Score band " + (bands.length + 1) + " · " + scoreWord() + " " + fmtN(bLo, 2) + (i === n - 1 ? " to ≤ " : " to < ") + fmtN(bHi, 2),
        members: members, best: best, ref: ref, picks: picks, decision: decision,
        priceCeil: priceCeil, timeCeil: timeCeil, floor: best - Number(R.allow),
      });
    }
    return { bands: bands, empty: null };
  }
  function bandScatter(band) {
    var pts = band.members.filter(function (p) { return p.cost != null && p.time != null; });
    if (pts.length < 2) return "";
    var pickKeys = {};
    band.picks.forEach(function (r) { pickKeys[r.p.key] = true; });
    var bw = 330, bh = 180, ml = 44, mr = 8, mt = 8, mb = 24, pw = bw - ml - mr, ph = bh - mt - mb;
    var xs = pts.map(function (p) { return p.cost; }), ys = pts.map(function (p) { return p.time; });
    var xlo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, xs)) * 2) / 2) / 1.4, xhi = Math.max.apply(null, xs) * 1.5;
    var ylo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, ys)) * 2) / 2) / 1.4, yhi = Math.max.apply(null, ys) * 1.5;
    // Faster is DOWN: screen y grows with time (no inversion).
    var X = function (v) { return ml + logPos(v, xlo, xhi) * pw; };
    var Y = function (v) { return mt + logPos(v, ylo, yhi) * ph; };
    var g = '<svg viewBox="0 0 ' + bw + " " + bh + '" role="img" aria-label="Price versus speed in this score band" style="max-width:340px;width:100%">';
    g += '<line x1="' + ml + '" y1="' + (mt + ph) + '" x2="' + (ml + pw) + '" y2="' + (mt + ph) + '" stroke="#30363d"/><line x1="' + ml + '" y1="' + mt + '" x2="' + ml + '" y2="' + (mt + ph) + '" stroke="#30363d"/>';
    g += '<text x="' + (ml + pw / 2) + '" y="' + (bh - 4) + '" font-size="9" fill="#8b949e" text-anchor="middle">USD / task (log) — cheaper is left</text>';
    g += '<text x="10" y="' + (mt + ph / 2) + '" font-size="9" fill="#8b949e" text-anchor="middle" transform="rotate(-90 10 ' + (mt + ph / 2) + ')">time — faster is down</text>';
    pts.forEach(function (p) {
      var on = !!pickKeys[p.key], col = famColor[p.family];
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + X(p.cost).toFixed(1) + '" cy="' + Y(p.time).toFixed(1) + '" r="' + (on ? 5 : 3.6) + '" fill="' + col + '" fill-opacity="' + (on ? 0.95 : 0.4) + '" stroke="' + (on ? "#fff" : col) + '" stroke-width="' + (on ? 1.6 : 0.8) + '"><title>' + esc(shortLabel(p) + " — " + fmt$(p.cost) + "/task · " + fmtT(p.time)) + "</title></circle>";
    });
    return g + "</svg>";
  }
  function renderRecs() {
    var wrap = document.getElementById("xpl-recs");
    if (!wrap) return;
    var out = computeRecs();
    if (out.empty) { wrap.innerHTML = '<p class="muted">' + esc(out.empty) + "</p>"; return; }
    var R = S.rec;
    var summary = document.getElementById("xpl-rec-summary");
    var totalPicks = out.bands.reduce(function (a, b) { return a + b.picks.length; }, 0);
    var labs = {};
    out.bands.forEach(function (b) { b.picks.forEach(function (r) { labs[r.p.creator] = true; }); });
    if (summary) {
      summary.textContent = totalPicks + " choices across " + out.bands.length + " score bands · " + Object.keys(labs).length + " labs. Near-top: " + R.allow +
        " points · Speed priority: " + R.speedPriority + "% · At most " + R.slowAllow + "× slower · Price allowance: +" + R.priceAllow + "% · " +
        (R.diversity === "lab" ? "One per lab" : R.diversity === "labsfirst" ? "Different labs first" : "No lab grouping") + ".";
    }
    wrap.innerHTML = out.bands.map(function (band, bi) {
      var ref = band.ref;
      var cards = band.picks.map(function (r, ri) {
        var p = r.p;
        var vsRef = "";
        if (ref && ref.key !== p.key) {
          var dt = ref.time ? Math.round((1 - p.time / ref.time) * 1000) / 10 : null;
          var dc = ref.cost ? Math.round((1 - p.cost / ref.cost) * 1000) / 10 : null;
          vsRef = '<br><span class="muted">vs band-best ' + esc(shortLabel(ref)) + ': ' +
            (dt != null ? (dt >= 0 ? dt + "% less time" : Math.abs(dt) + "% more time") : "") +
            (dt != null && dc != null ? " · " : "") +
            (dc != null ? (dc >= 0 ? dc + "% less cost" : Math.abs(dc) + "% more cost") : "") + "</span>";
        } else if (ref) {
          vsRef = '<br><span class="muted">The top-scoring reference for this band.</span>';
        }
        var bestFactor = band.picks.length ? band.picks[0].factor : 1;
        return '<div class="card"><span class="badge ' + (ri === 0 ? "open" : "src") + '">' + (ri === 0 ? "Best fit" : p.creator === band.picks[0].p.creator ? "Alternative" : "Alternative lab") + "</span> " +
          '<button type="button" class="xpl-linkbtn" data-pick="' + esc(p.key) + '"><strong>' + esc(shortLabel(p)) + "</strong></button>" +
          '<span class="muted"> · ' + esc(p.creator) + (p.harness ? " · " + esc(p.harness) : "") + "</span><br>" +
          scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + "/task · " +
          fmtN(band.best - p.score, 1) + " below band best · " + fmtN(r.factor / bestFactor, 2) + "× best tradeoff" + vsRef + "</div>";
      }).join("");
      var counts = {};
      band.members.forEach(function (p) { var d = band.decision[p.key] || "Not selected"; counts[d] = (counts[d] || 0) + 1; });
      var whyRows = band.members.slice().sort(function (a, b) {
        var ra = band.decision[a.key] === "Recommended" ? 0 : 1, rb = band.decision[b.key] === "Recommended" ? 0 : 1;
        return ra - rb || cmpDesc(a.score, b.score);
      }).map(function (p) {
        return '<tr data-key="' + esc(p.key) + '"><td><strong>' + esc(p.family) + '</strong> <span class="muted">' + esc(p.effort || "") + '</span></td><td>' + esc(p.creator) + "</td><td>" + esc(band.decision[p.key] || "Not selected") + "</td></tr>";
      }).join("");
      var whyCounts = Object.keys(counts).map(function (k) { return counts[k] + " · " + k; }).join(" &nbsp;·&nbsp; ");
      return "<h3>" + esc(band.label) + " · " + band.members.length + " variants</h3>" +
        '<p class="muted">Require score ≥ ' + fmtN(band.floor, 1) + " (band best " + fmtN(band.best, 1) + ", allowance " + R.allow + " points)" +
        (band.priceCeil != null ? " · Relative price ceiling " + fmt$(band.priceCeil) + " · Relative time ceiling " + fmtT(band.timeCeil) : " · no candidate carries all three measures") + "</p>" +
        bandScatter(band) +
        '<div class="board">' + (cards || '<p class="muted">No recommendation survives the limits of this band.</p>') + "</div>" +
        '<details><summary>Why these choices?</summary><p class="muted">' + whyCounts + '</p><div class="tblwrap"><table class="xpl-table"><caption>Why these choices — decisions by model and effort</caption><thead><tr><th scope="col">Model / effort</th><th scope="col">Lab</th><th scope="col">Decision</th></tr></thead><tbody>' + whyRows + "</tbody></table></div></details>";
    }).join("");
    wrap.querySelectorAll("[data-pick]").forEach(function (b) {
      b.addEventListener("click", function () { selectPoint(b.getAttribute("data-pick")); });
    });
    wrap.querySelectorAll("tbody tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
    wrap.querySelectorAll("svg [data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
    markSelectedRow();
  }
  // --- Band comparisons (Site 1's six banded shortlists) -------------------
  // Near-leader windows at 1x: score within 2 points, time within 10%,
  // price within 25% of the band leader; the scale select multiplies all
  // three. Tables only — Site 1's lane charts are noted as a follow-up.
  function logEdges(vals, n) {
    var xs = vals.filter(function (v) { return v != null && v > 0; });
    if (!xs.length) return [];
    var mn = Math.min.apply(null, xs), mx = Math.max.apply(null, xs);
    if (mn === mx) return [mn, mx];
    var edges = [];
    for (var i = 0; i <= n; i++) edges.push(mn * Math.pow(mx / mn, i / n));
    return edges;
  }
  function scoreEdges(rows, n) {
    var xs = rows.map(function (p) { return p.score; }).filter(function (v) { return v != null; });
    if (!xs.length) return [];
    var mn = Math.min.apply(null, xs), mx = Math.max.apply(null, xs);
    var edges = [];
    for (var i = 0; i <= n; i++) edges.push(mn + ((mx - mn) * i) / n);
    return edges;
  }
  function bandIndex(v, edges) {
    for (var i = 0; i < edges.length - 1; i++) {
      if (v >= edges[i] && (i === edges.length - 2 ? v <= edges[i + 1] : v < edges[i + 1])) return i;
    }
    return -1;
  }
  function chipFor(p, leader) {
    var notes = [];
    if (leader && leader.key !== p.key) {
      var better = [];
      if (p.score != null && leader.score != null && p.score > leader.score) better.push("higher score");
      if (p.cost != null && leader.cost != null && p.cost < leader.cost) better.push("lower price");
      if (p.time != null && leader.time != null && p.time < leader.time) better.push("faster");
      if (better.length) notes.push("Better at: " + better.join(", "));
    }
    return '<button type="button" class="xchip" data-pick="' + esc(p.key) + '">' + esc(shortLabel(p)) + " — " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + " · " + fmtT(p.time) + (notes.length ? ' <span class="muted">(' + esc(notes.join("; ")) + ")</span>" : "") + "</button>";
  }
  function bandRangeLine(members) {
    var withScore = members.filter(function (p) { return p.score != null; });
    var withTime = members.filter(function (p) { return p.time != null; });
    var parts = [];
    if (withScore.length) {
      var sMin = withScore.reduce(function (a, b) { return b.score < a.score ? b : a; });
      var sMax = withScore.reduce(function (a, b) { return b.score > a.score ? b : a; });
      parts.push("score " + fmtN(sMin.score, 1) + " (" + shortLabel(sMin) + ") – " + fmtN(sMax.score, 1) + " (" + shortLabel(sMax) + ")");
    }
    if (withTime.length) {
      var tMin = withTime.reduce(function (a, b) { return b.time < a.time ? b : a; });
      var tMax = withTime.reduce(function (a, b) { return b.time > a.time ? b : a; });
      parts.push("time " + fmtT(tMin.time) + " (" + shortLabel(tMin) + ") – " + fmtT(tMax.time) + " (" + shortLabel(tMax) + ")");
    }
    return parts.join(" · ");
  }
  function bandGroup(title, blurb, bands) {
    var rowsHtml = bands.map(function (b) {
      var leader = b.choices.length ? b.choices[0] : null;
      return "<tr><th scope=\\\"row\\\"><strong>" + esc(b.label) + '</strong><br><span class="muted">' + b.choices.length + " near-leading · " + esc(bandRangeLine(b.members)) + "</span></th><td>" +
        (b.choices.length ? b.choices.map(function (p) { return chipFor(p, leader); }).join(" ") : '<span class="muted">No model meets the window of this band.</span>') + "</td></tr>";
    }).join("");
    return "<h3>" + esc(title) + '</h3><p class="muted">' + esc(blurb) + '</p><div class="tblwrap"><table class="xpl-table"><caption>' + esc(title) + '</caption><tbody>' + rowsHtml + "</tbody></table></div>";
  }
  function renderBandComparisons() {
    var wrap = document.getElementById("xpl-bandgroups");
    if (!wrap) return;
    var rows = visPoints();
    var n = Math.max(2, Math.min(8, Number(S.bandCount) || 3));
    var scale = Number(S.bandScale) || 1;
    var scoreWin = 2 * scale, timeWin = 0.10 * scale, priceWin = 0.25 * scale;
    var costEdges = logEdges(rows.map(function (p) { return p.cost; }), n);
    var timeEdges = logEdges(rows.map(function (p) { return p.time; }), n);
    var sEdges = scoreEdges(rows, n);
    function groupBy(edges, get) {
      var out = [];
      for (var i = 0; i < edges.length - 1; i++) out.push([]);
      rows.forEach(function (p) {
        var v = get(p);
        if (v == null) return;
        var idx = bandIndex(v, edges);
        if (idx >= 0) out[idx].push(p);
      });
      return out;
    }
    function costLabel(i) { return (i === 0 ? "≥ " : "> ") + fmt$(costEdges[i]) + " · ≤ " + fmt$(costEdges[i + 1]); }
    function timeLabel(i) { return fmtT(timeEdges[i]) + " – " + fmtT(timeEdges[i + 1]); }
    function scoreLabel(i) { return fmtN(sEdges[i], 1) + (i === sEdges.length - 2 ? " – ≤ " : " – < ") + fmtN(sEdges[i + 1], 1); }
    var html = "";
    // 1. Smartest within each cost band
    if (costEdges.length > 1) {
      var g1 = groupBy(costEdges, function (p) { return p.cost; }).map(function (members, i) {
        var scored = members.filter(function (p) { return p.score != null; });
        var lead = scored.length ? Math.max.apply(null, scored.map(function (p) { return p.score; })) : null;
        var choices = scored.filter(function (p) { return lead != null && p.score >= lead - scoreWin; }).sort(function (a, b) { return cmpDesc(a.score, b.score); });
        return { label: "Cost band " + (i + 1) + " · " + costLabel(i), members: members, choices: choices };
      });
      var h1 = bandGroup("Smartest within each task-cost band", "Highest scores only within each listed task-cost range; near-leaders sit within " + fmtN(scoreWin, 2) + " points of the band's best score.", g1);
      // 5. Fastest within each cost band
      var g5 = groupBy(costEdges, function (p) { return p.cost; }).map(function (members, i) {
        var timed = members.filter(function (p) { return p.time != null; });
        var lead = timed.length ? Math.min.apply(null, timed.map(function (p) { return p.time; })) : null;
        var choices = timed.filter(function (p) { return lead != null && p.time <= lead * (1 + timeWin); }).sort(function (a, b) { return cmpAsc(a.time, b.time); });
        return { label: "Cost band " + (i + 1) + " · " + costLabel(i), members: members, choices: choices };
      });
      var h5 = bandGroup("Fastest within each task-cost band", "Disjoint task-cost tiers; near-leaders are within " + Math.round(timeWin * 100) + "% of the band's fastest time.", g5);
    }
    if (sEdges.length > 1) {
      // 2. Lowest cost within each score band
      var g2 = groupBy(sEdges, function (p) { return p.score; }).map(function (members, i) {
        var priced = members.filter(function (p) { return p.cost != null; });
        var lead = priced.length ? Math.min.apply(null, priced.map(function (p) { return p.cost; })) : null;
        var choices = priced.filter(function (p) { return lead != null && p.cost <= lead * (1 + priceWin); }).sort(function (a, b) { return cmpAsc(a.cost, b.cost); });
        return { label: scoreWord() + " band " + (i + 1) + " · " + scoreLabel(i), members: members, choices: choices };
      });
      var h2 = bandGroup("Lowest task cost within each score band", "Bands split the selected score range evenly; each model appears in one score band; near-price window " + Math.round(priceWin * 100) + "%.", g2);
      // 3. Fastest within each score band
      var g3 = groupBy(sEdges, function (p) { return p.score; }).map(function (members, i) {
        var timed = members.filter(function (p) { return p.time != null; });
        var lead = timed.length ? Math.min.apply(null, timed.map(function (p) { return p.time; })) : null;
        var choices = timed.filter(function (p) { return lead != null && p.time <= lead * (1 + timeWin); }).sort(function (a, b) { return cmpAsc(a.time, b.time); });
        return { label: scoreWord() + " band " + (i + 1) + " · " + scoreLabel(i), members: members, choices: choices };
      });
      var h3 = bandGroup("Fastest within each score band", "Within " + Math.round(timeWin * 100) + "% of the band's speed leader.", g3);
    }
    if (timeEdges.length > 1) {
      // 4. Smartest within each speed band (slowest first, as Site 1)
      var g4raw = groupBy(timeEdges, function (p) { return p.time; });
      var g4 = g4raw.map(function (members, i) {
        var scored = members.filter(function (p) { return p.score != null; });
        var lead = scored.length ? Math.max.apply(null, scored.map(function (p) { return p.score; })) : null;
        var choices = scored.filter(function (p) { return lead != null && p.score >= lead - scoreWin; }).sort(function (a, b) { return cmpDesc(a.score, b.score); });
        return { label: (i === g4raw.length - 1 ? "Slowest speed band" : i === 0 ? "Fastest speed band" : "Speed band " + (i + 1)) + " · " + timeLabel(i), members: members, choices: choices };
      }).reverse();
      var h4 = bandGroup("Smartest within each speed band", "Selected speed values split into ordered, non-overlapping bands from slowest to fastest; near-leaders within the score window.", g4);
      // 6. Lowest price within each speed target
      var timedAll = rows.filter(function (p) { return p.time != null; }).map(function (p) { return p.time; }).sort(function (a, b) { return a - b; });
      var tiers = [];
      for (var ti = 0; ti < costEdges.length - 1; ti++) tiers.push(ti);
      var targetRows = [50, 75, 90, 100].map(function (q) {
        var thr = timedAll.length ? timedAll[Math.min(timedAll.length - 1, Math.ceil((q / 100) * timedAll.length) - 1)] : null;
        var cells = tiers.map(function (ci) {
          var qual = rows.filter(function (p) { return p.cost != null && p.time != null && thr != null && p.time <= thr && bandIndex(p.cost, costEdges) === ci; });
          if (!qual.length) return '<span class="muted">No model meets this target in this band.</span>';
          var cheapest = qual.slice().sort(function (a, b) { return cmpAsc(a.cost, b.cost); })[0];
          var near = qual.filter(function (p) { return p.cost <= cheapest.cost * (1 + priceWin); }).sort(function (a, b) { return cmpAsc(a.cost, b.cost); });
          return near.map(function (p) { return chipFor(p, cheapest); }).join(" ");
        });
        return "<tr><th scope=\\\"row\\\"><strong>" + (q === 100 ? "Fastest available (100th percentile)" : "At least " + q + "th percentile") + '</strong><br><span class="muted">time ≤ ' + fmtT(thr) + "</span></th>" + cells.map(function (c) { return "<td>" + c + "</td>"; }).join("") + "</tr>";
      }).join("");
      var h6 = '<h3>Lowest price within each speed target</h3><p class="muted">Each speed target is split into the selected number of log-spaced task-cost bands; nearby options within ' + Math.round(priceWin * 100) + "% of the tier's cheapest.</p>" +
        '<div class="tblwrap"><table class="xpl-table"><caption>Lowest price within each speed target</caption><thead><tr><th scope="col">Speed target</th>' + tiers.map(function (ci) { return "<th scope=\\\"col\\\">Tier " + (ci + 1) + " · " + esc(costLabel(ci)) + "</th>"; }).join("") + "</tr></thead><tbody>" + targetRows + "</tbody></table></div>";
    }
    html = (typeof h1 !== "undefined" ? h1 : "") + (typeof h2 !== "undefined" ? h2 : "") + (typeof h3 !== "undefined" ? h3 : "") + (typeof h4 !== "undefined" ? h4 : "") + (typeof h5 !== "undefined" ? h5 : "") + (typeof h6 !== "undefined" ? h6 : "");
    wrap.innerHTML = html || '<p class="muted">No banded comparisons can be built — the visible configurations lack the measures they need.</p>';
    wrap.querySelectorAll("[data-pick]").forEach(function (b) {
      b.addEventListener("click", function () { selectPoint(b.getAttribute("data-pick")); });
    });
  }
  function renderAaTable() {
    var tb = document.getElementById("xpl-rows");
    if (!tb) return;
    var rows = sortPoints(visPoints());
    var scoreHead = document.getElementById("xpl-scorehead");
    if (scoreHead) scoreHead.textContent = S.persp === "cai" ? "CAI score" : "Intelligence";
    document.getElementById("xpl-count").textContent =
      rows.length + " of " + POINTS.length + " ranked configurations · sorted by the " + S.xp + " perspective · dots are colored by model family; white-ringed dots sit on that chart's frontier, dimmed dots are beaten on its axes";
    tb.innerHTML = rows.map(function (p) {
      return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '"><td><strong>' + esc(p.family) + '</strong><br><span class="muted">' + esc(p.creator) + (p.qualStatus ? " · " + esc(p.qualStatus) : "") + '</span></td><td>' + esc(p.effort || "—") + '</td><td class="num">' + fmtN(p.score, 1) + '</td><td class="num xpl-costcol">' + fmt$(p.cost) + '</td><td class="num">' + fmtT(p.time) + '</td><td class="num">' + fmtT(p.ttft) + '</td><td class="num">' + fmtN(p.tps, 0) + '</td><td class="num xpl-costcol">' + fmtN(p.pointsPerDollar != null ? p.pointsPerDollar : p.scorePerDollar, 1) + '</td><td class="num">' + fmtN(p.pointsPerSecond != null ? p.pointsPerSecond : p.scorePerSecond, 3) + "</td></tr>";
    }).join("");
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
    markSelectedRow();
  }
  // --- task-assumptions projection (Site 2, in the Model Explorer) -------
  // ESTIMATED, NOT MEASURED: priced only from the token rates recorded
  // in the ainventory price event log (the catalog's rate bands). A
  // missing required rate yields an unknown cost, never a guess.
  // cost = turns x [uncached input + cache reads + cache writes +
  // (answer + thinking) output]; time = turns x (measured TTFT +
  // answer / measured throughput + tool overhead), borrowing the
  // family timing from either benchmark point set.
  function pjBand(e) {
    if (!e.rate || !e.rate.bands.length) return null;
    var bands = e.rate.bands;
    var win = bands.filter(function (b) { return b.window === S.pj.window; });
    var pool = win.length ? win : bands.filter(function (b) { return b.window == null || b.window === "flat"; });
    if (!pool.length) pool = bands;
    var input = Number(S.pj.inputTokens) || 0;
    for (var i = 0; i < pool.length; i++) {
      var b = pool[i];
      if ((b.minTokens == null || input >= b.minTokens) && (b.maxTokens == null || input < b.maxTokens)) return b;
    }
    return pool[pool.length - 1];
  }
  function pjTiming(e) {
    var norm = normName(e.name);
    var sets = [AA_POINTS, CAI_POINTS];
    for (var si = 0; si < sets.length; si++) {
      var cands = sets[si].filter(function (p) { return normName(p.family) === norm; });
      for (var i = 0; i < cands.length; i++) if (cands[i].tps != null) return { tps: cands[i].tps, ttft: cands[i].ttft };
    }
    return null;
  }
  function project(e) {
    var band = pjBand(e);
    if (!band) return { cost: null, time: null, band: null };
    var P = S.pj;
    var input = Number(P.inputTokens) || 0, answer = Number(P.answerTokens) || 0, thinking = Number(P.thinkingTokens) || 0;
    var turns = Number(P.turns) || 0;
    var cacheShare = (Number(P.cacheReadPct) || 0) / 100;
    var writesShare = (Number(P.writesPct) || 0) / 100;
    var uncached = input * (1 - cacheShare);
    var reads = input * cacheShare;
    var writes = uncached * writesShare;
    var missing = band.input == null || band.output == null || (reads > 0 && band.cacheRead == null) || (writes > 0 && band.cacheWrite == null);
    var cost = null;
    if (!missing) {
      var perTurn = (uncached * band.input + reads * (band.cacheRead || 0) + writes * (band.cacheWrite || 0) + (answer + thinking) * band.output) / 1000000;
      cost = turns * perTurn;
    }
    var timing = pjTiming(e);
    var time = null;
    if (timing && timing.tps) time = turns * ((timing.ttft || 0) + answer / timing.tps + (Number(P.overheadSec) || 0));
    return { cost: cost, time: time, band: band };
  }
  function renderModels() {
    var tb = document.getElementById("xpl-mrows");
    if (!tb) return;
    if (!CATALOG) return;
    var rows = sortCatalog(visCatalog());
    document.getElementById("xpl-mcount").textContent =
      rows.length + " of " + CATALOG.length + " catalog models · sorted by the " + S.xp + " perspective" +
      (S.xp === "api" ? " (effective $/task = AA list $/task ÷ the plan's multiplier, where the registry records one)" : "");
    tb.innerHTML = rows.map(function (e) {
      var ec = effCost(e);
      var proj = project(e);
      return '<tr data-sel="' + esc(e.id) + '" data-mid="' + esc(e.id) + '"><td><strong>' + esc(e.name) + '</strong><br><span class="muted"><code>' + esc(e.id) + "</code></span></td>" +
        "<td>" + esc(e.providerName) + (e.plan ? '<br><span class="badge src">' + esc(e.plan) + "</span>" : "") + (e.disabled ? ' <span class="badge stale">disabled</span>' : "") + '</td>' +
        '<td class="num">' + fmtCtx(e.contextLimit) + "</td>" +
        "<td>" + (e.variants.length ? esc(e.variants.join(", ")) : "—") + "</td>" +
        "<td>" + (e.routes.length ? e.routes.map(function (r) { return '<span class="badge open">' + esc(r) + "</span>"; }).join(" ") : '<span class="muted">—</span>') + "</td>" +
        '<td class="num">' + fmtN(e.aaScore, 1) + '</td><td class="num xpl-costcol">' + fmt$(e.aaCostPerTask) + '</td><td class="num xpl-costcol">' + (ec != null ? fmt$(ec) : "—") + '</td><td class="num">' + fmtT(e.aaTimePerTask) + '</td>' +
        '<td class="num xpl-costcol">' + (proj.cost != null ? "~" + fmt$(proj.cost) : "—") + '</td><td class="num">' + (proj.time != null ? "~" + fmtT(proj.time) : "—") + '</td><td class="num xpl-costcol">' + (e.aaScore != null && proj.cost ? fmtN(e.aaScore / proj.cost, 1) : "—") + "</td></tr>";
    }).join("");
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectCatalog(tr.getAttribute("data-mid")); });
    });
    markSelectedRow();
  }
  function buildFilterOptions() {
    var creatorSel = document.getElementById("xf-creator");
    var creators = [];
    var seenC = {};
    POINTS.forEach(function (p) {
      var k = p.creator.toLowerCase();
      if (!seenC[k]) { seenC[k] = true; creators.push(p.creator); }
    });
    creators.sort();
    creatorSel.innerHTML = '<option value="">All creators</option>' + creators.map(function (c) { return '<option value="' + esc(c) + '"' + (S.creator === c ? " selected" : "") + ">" + esc(c) + "</option>"; }).join("");
    var benchSel = document.getElementById("xf-bench");
    var benches = [];
    POINTS.forEach(function (p) { if (p.benchmark && benches.indexOf(p.benchmark) < 0) benches.push(p.benchmark); });
    benchSel.innerHTML = '<option value="">All benchmarks</option>' + benches.map(function (b) { return '<option value="' + esc(b) + '"' + (S.bench === b ? " selected" : "") + ">" + esc(b) + "</option>"; }).join("");
    var effWrap = document.getElementById("xf-efforts");
    var efforts = [];
    POINTS.forEach(function (p) { var e = p.effort || ""; if (efforts.indexOf(e) < 0) efforts.push(e); });
    efforts.sort(function (a, b) { return eord(a) - eord(b); });
    effWrap.innerHTML = "";
    efforts.forEach(function (e) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "xchip" + (S.effortAny && !S.efforts[e] ? " off" : "");
      b.textContent = e || "(none)";
      b.addEventListener("click", function () {
        if (!S.effortAny) { S.effortAny = true; S.efforts = {}; S.efforts[e] = true; }
        else if (S.efforts[e]) { delete S.efforts[e]; if (!Object.keys(S.efforts).length) S.effortAny = false; }
        else S.efforts[e] = true;
        render();
      });
      effWrap.appendChild(b);
    });
    buildFamOptions();
  }
  function buildFamOptions() {
    var sel = document.getElementById("xf-fam");
    var label = document.getElementById("xf-fam-label");
    var opts = [];
    if (S.tab === "models" && CATALOG) {
      label.textContent = "Provider";
      var seen = {};
      CATALOG.forEach(function (e) { if (!seen[e.provider]) { seen[e.provider] = true; opts.push([e.provider, e.providerName]); } });
      opts.sort(function (a, b) { return a[1] < b[1] ? -1 : 1; });
      sel.innerHTML = '<option value="">All providers</option>' + opts.map(function (o) { return '<option value="' + esc(o[0]) + '"' + (S.fam === o[0] ? " selected" : "") + ">" + esc(o[1]) + "</option>"; }).join("");
    } else {
      label.textContent = "Family";
      sel.innerHTML = '<option value="">All families</option>' + famList.slice().sort().map(function (f) { return '<option value="' + esc(f) + '"' + (S.fam === f ? " selected" : "") + ">" + esc(f) + "</option>"; }).join("");
    }
  }
  function applyTab() {
    document.querySelectorAll("[data-xtab]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-xtab") === S.tab);
    });
    document.getElementById("xtab-aa").hidden = S.tab !== "aa";
    document.getElementById("xtab-models").hidden = S.tab !== "models";
    document.querySelectorAll("[data-tabaa]").forEach(function (el) { el.hidden = S.tab !== "aa"; });
    document.querySelectorAll("[data-tabmodels]").forEach(function (el) { el.hidden = S.tab !== "models"; });
    buildFamOptions();
  }
  function render() {
    document.body.classList.toggle("xp-plan", S.xp === "plan");
    document.getElementById("xp-note").textContent = XP_NOTES[S.xp];
    document.querySelectorAll("[data-xp]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-xp") === S.xp);
    });
    document.querySelectorAll("[data-persp]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-persp") === S.persp);
    });
    var pnote = document.getElementById("xpl-persp-note");
    if (pnote) {
      pnote.textContent = S.persp === "cai"
        ? "Coding agents perspective — the Coding Agent Index observations (model + effort + agent harness): score is the CAI score, cost the published cost per coding task, time the average agent wall time per task. Kept separate from the Intelligence Index; the two are never converted."
        : "Intelligence & decode perspective — the AA Intelligence Index rankings: score is the Intelligence Index, cost and time the import's per-task figures (time excludes TTFT).";
    }
    applyTab();
    if (S.tab === "aa") { renderLegend(); renderTray(); drawCharts(); renderAaTable(); renderPareto(); renderRecs(); renderBandComparisons(); }
    else renderModels();
  }
  document.querySelectorAll("[data-xtab]").forEach(function (b) {
    b.addEventListener("click", function () {
      S.tab = b.getAttribute("data-xtab");
      S.fam = "";
      history.replaceState(null, "", S.tab === "models" ? "#models" : "#aa");
      render();
    });
  });
  document.querySelectorAll("[data-xp]").forEach(function (b) {
    b.addEventListener("click", function () { S.xp = b.getAttribute("data-xp"); render(); });
  });
  function setPerspective(persp) {
    if (S.persp === persp) return;
    S.persp = persp;
    POINTS = persp === "cai" ? CAI_POINTS : AA_POINTS;
    S.creator = ""; S.fam = ""; S.bench = ""; S.effortAny = false; S.efforts = {};
    S.off = {}; S.hidden = []; S.sel = null; S.baseline = "";
    showDetail(null);
    buildFilterOptions();
    buildBaselineOptions();
    render();
  }
  document.querySelectorAll("[data-persp]").forEach(function (b) {
    b.addEventListener("click", function () { setPerspective(b.getAttribute("data-persp")); });
  });
  var pfCutEl = document.getElementById("xf-pfcut");
  if (pfCutEl) pfCutEl.addEventListener("change", function (ev) { S.pfCut = ev.target.value; renderPareto(); });
  var bandsEl = document.getElementById("xf-bands");
  if (bandsEl) bandsEl.addEventListener("change", function (ev) { S.bandCount = Number(ev.target.value); renderRecs(); renderBandComparisons(); });
  var bandScaleEl = document.getElementById("xf-bandscale");
  if (bandScaleEl) bandScaleEl.addEventListener("change", function (ev) { S.bandScale = Number(ev.target.value); renderBandComparisons(); });
  function syncRecInputs() {
    document.querySelectorAll("[data-rec]").forEach(function (el) {
      var f = el.getAttribute("data-rec");
      if (S.rec[f] != null) el.value = S.rec[f];
    });
    var sp = document.getElementById("rec-speedpriority-val");
    if (sp) sp.textContent = S.rec.speedPriority + "% speed";
  }
  document.querySelectorAll("[data-rec]").forEach(function (el) {
    el.addEventListener("input", function () {
      S.rec[el.getAttribute("data-rec")] = el.value;
      syncRecInputs();
      renderRecs();
    });
    el.addEventListener("change", function () {
      S.rec[el.getAttribute("data-rec")] = el.value;
      renderRecs();
    });
  });
  var REC_PRESETS = {
    fast: { allow: 5, speedPriority: 95, priceAllow: 200, slowAllow: 1.5, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3 },
    balanced: { allow: 5, speedPriority: 50, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3 },
    diversity: { allow: 7, speedPriority: 60, priceAllow: 500, slowAllow: 2.5, altWindow: 3, diversity: "labsfirst", dominance: "lab", perBand: 4 },
    budget: { allow: 5, speedPriority: 15, priceAllow: 100, slowAllow: 3, altWindow: 2, diversity: "lab", dominance: "all", perBand: 3 },
    reset: { allow: 5, speedPriority: 85, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3, maxCost: "", maxSec: "" },
  };
  document.querySelectorAll("[data-recpreset]").forEach(function (b) {
    b.addEventListener("click", function () {
      var preset = REC_PRESETS[b.getAttribute("data-recpreset")];
      if (!preset) return;
      Object.keys(preset).forEach(function (k) { S.rec[k] = preset[k]; });
      syncRecInputs();
      renderRecs();
    });
  });
  function syncPjInputs() {
    document.querySelectorAll("[data-pj]").forEach(function (el) {
      var f = el.getAttribute("data-pj");
      if (S.pj[f] != null) el.value = S.pj[f];
    });
    var cv = document.getElementById("pj-cacheread-val");
    if (cv) cv.textContent = S.pj.cacheReadPct + "%";
  }
  document.querySelectorAll("[data-pj]").forEach(function (el) {
    var on = function () { S.pj[el.getAttribute("data-pj")] = el.value; syncPjInputs(); if (S.tab === "models") renderModels(); };
    el.addEventListener("input", on);
    el.addEventListener("change", on);
  });
  var PJ_PRESETS = {
    small: { turns: 2, inputTokens: 8000, answerTokens: 1200, thinkingTokens: 2000, cacheReadPct: 60, writesPct: 0, overheadSec: 8 },
    feature: { turns: 4, inputTokens: 25000, answerTokens: 3000, thinkingTokens: 6000, cacheReadPct: 50, writesPct: 5, overheadSec: 10 },
    repo: { turns: 8, inputTokens: 60000, answerTokens: 4000, thinkingTokens: 8000, cacheReadPct: 70, writesPct: 5, overheadSec: 15 },
  };
  document.querySelectorAll("[data-pjpreset]").forEach(function (b) {
    b.addEventListener("click", function () {
      var preset = PJ_PRESETS[b.getAttribute("data-pjpreset")];
      if (!preset) return;
      Object.keys(preset).forEach(function (k) { S.pj[k] = preset[k]; });
      syncPjInputs();
      if (S.tab === "models") renderModels();
    });
  });
  var recExport = document.getElementById("xpl-rec-export");
  if (recExport) {
    recExport.addEventListener("click", function () {
      var out = computeRecs();
      var lines = ["band,rank,model,effort,lab,harness,score,cost_per_task,time_per_task_s,tradeoff_multiple"];
      out.bands.forEach(function (band) {
        var bestFactor = band.picks.length ? band.picks[0].factor : 1;
        band.picks.forEach(function (r, ri) {
          lines.push([band.label, ri + 1, r.p.family, r.p.effort || "", r.p.creator, r.p.harness || "", r.p.score, r.p.cost, r.p.time, (r.factor / bestFactor).toFixed(2)].map(function (v) { return '"' + String(v == null ? "" : v).replace(/"/g, '""') + '"'; }).join(","));
        });
      });
      downloadFile("cost-explorer-shortlist.csv", lines.join("\\n"), "text/csv");
    });
  }
  document.getElementById("xf-q").addEventListener("input", function (ev) { S.q = ev.target.value.trim().toLowerCase(); render(); });
  document.getElementById("xf-creator").addEventListener("change", function (ev) { S.creator = ev.target.value; render(); });
  document.getElementById("xf-fam").addEventListener("change", function (ev) { S.fam = ev.target.value; render(); });
  document.getElementById("xf-bench").addEventListener("change", function (ev) { S.bench = ev.target.value; render(); });
  document.getElementById("xf-elig").addEventListener("change", function (ev) { S.elig = ev.target.checked; render(); });
  document.getElementById("xf-pooled").addEventListener("change", function (ev) { S.pooledOnly = ev.target.checked; render(); });
  document.getElementById("xf-measured").addEventListener("change", function (ev) { S.measuredOnly = ev.target.checked; render(); });
  document.getElementById("xf-nodis").addEventListener("change", function (ev) { S.hideDisabled = ev.target.checked; render(); });
  document.querySelectorAll("[data-sizeby]").forEach(function (cb) {
    cb.addEventListener("change", function () { S.sizeBy[cb.getAttribute("data-sizeby")] = cb.checked; drawCharts(); });
  });
  document.querySelectorAll("[data-dl]").forEach(function (b) {
    b.addEventListener("click", function () { downloadPng(b.getAttribute("data-dl")); });
  });
  document.querySelectorAll("[data-tmode]").forEach(function (b) {
    b.addEventListener("click", function () { S.tasksMode = b.getAttribute("data-tmode"); drawCharts(); });
  });
  var baselineSel = document.getElementById("xf-baseline");
  if (baselineSel) baselineSel.addEventListener("change", function (ev) { S.baseline = ev.target.value; drawCharts(); });
  // --- inequality filter rows ---------------------------------------------
  var INEQ_METRICS = [["score", "Score"], ["cost", "Task cost"], ["time", "Selected speed (time / task)"], ["tps", "Output speed (tok/s)"], ["gapScore", "Pareto gap: score gap (vs cost frontier)"], ["gapCost", "Pareto gap: price premium (vs score)"], ["gapSpeed", "Pareto gap: speed gap (vs time frontier)"]];
  function renderIneqConds() {
    var wrap = document.getElementById("xf-ineq-conds");
    if (!wrap) return;
    wrap.innerHTML = S.ineq.conds.map(function (c, i) {
      var metricOpts = INEQ_METRICS.map(function (m) { return '<option value="' + m[0] + '"' + (c.metric === m[0] ? " selected" : "") + ">" + m[1] + "</option>"; }).join("");
      var opOpts = ["<", "<=", "=", ">=", ">"].map(function (o) { return '<option value="' + o + '"' + (c.op === o ? " selected" : "") + ">" + o + "</option>"; }).join("");
      return '<span class="xpl-ineq-row"><label>Condition ' + (i + 1) + ' <select data-ineq-metric="' + i + '">' + metricOpts + '</select></label> <select data-ineq-op="' + i + '" aria-label="Operator">' + opOpts + '</select> <input type="number" step="any" data-ineq-value="' + i + '" value="' + esc(c.value) + '" style="width:7em"> <button type="button" class="xpl-ctl-btn" data-ineq-remove="' + i + '">Remove</button></span>';
    }).join("");
    wrap.querySelectorAll("[data-ineq-metric]").forEach(function (el) { el.addEventListener("change", function () { S.ineq.conds[Number(el.getAttribute("data-ineq-metric"))].metric = el.value; render(); }); });
    wrap.querySelectorAll("[data-ineq-op]").forEach(function (el) { el.addEventListener("change", function () { S.ineq.conds[Number(el.getAttribute("data-ineq-op"))].op = el.value; render(); }); });
    wrap.querySelectorAll("[data-ineq-value]").forEach(function (el) { el.addEventListener("input", function () { S.ineq.conds[Number(el.getAttribute("data-ineq-value"))].value = el.value; render(); }); });
    wrap.querySelectorAll("[data-ineq-remove]").forEach(function (el) { el.addEventListener("click", function () { S.ineq.conds.splice(Number(el.getAttribute("data-ineq-remove")), 1); renderIneqConds(); render(); }); });
  }
  var ineqModeEl = document.getElementById("xf-ineq-mode");
  if (ineqModeEl) ineqModeEl.addEventListener("change", function (ev) { S.ineq.mode = ev.target.value; render(); });
  var ineqCombineEl = document.getElementById("xf-ineq-combine");
  if (ineqCombineEl) ineqCombineEl.addEventListener("change", function (ev) { S.ineq.combine = ev.target.value; render(); });
  var ineqAddEl = document.getElementById("xf-ineq-add");
  if (ineqAddEl) ineqAddEl.addEventListener("click", function () { S.ineq.conds.push({ metric: "score", op: ">=", value: "" }); renderIneqConds(); });
  // --- saved setups + downloads ---------------------------------------------
  function captureSetup() {
    return JSON.parse(JSON.stringify({
      tab: S.tab, persp: S.persp, xp: S.xp, q: S.q, creator: S.creator, fam: S.fam, bench: S.bench,
      effortAny: S.effortAny, efforts: S.efforts, off: S.off, elig: S.elig, pooledOnly: S.pooledOnly,
      measuredOnly: S.measuredOnly, hideDisabled: S.hideDisabled, hidden: S.hidden, sizeBy: S.sizeBy,
      tasksMode: S.tasksMode, baseline: S.baseline, pfCut: S.pfCut, bandCount: S.bandCount,
      bandScale: S.bandScale, rec: S.rec, pj: S.pj, ineq: S.ineq,
    }));
  }
  function syncStaticControls() {
    var q = document.getElementById("xf-q"); if (q) q.value = S.q;
    var elig = document.getElementById("xf-elig"); if (elig) elig.checked = S.elig;
    var pooled = document.getElementById("xf-pooled"); if (pooled) pooled.checked = S.pooledOnly;
    var meas = document.getElementById("xf-measured"); if (meas) meas.checked = S.measuredOnly;
    var nodis = document.getElementById("xf-nodis"); if (nodis) nodis.checked = S.hideDisabled;
    var pf = document.getElementById("xf-pfcut"); if (pf) pf.value = S.pfCut;
    var bd = document.getElementById("xf-bands"); if (bd) bd.value = String(S.bandCount);
    var bs = document.getElementById("xf-bandscale"); if (bs) bs.value = String(S.bandScale);
    if (ineqModeEl) ineqModeEl.value = S.ineq.mode;
    if (ineqCombineEl) ineqCombineEl.value = S.ineq.combine;
    document.querySelectorAll("[data-tmode]").forEach(function (b) { b.classList.toggle("active", b.getAttribute("data-tmode") === S.tasksMode); });
    document.querySelectorAll("[data-sizeby]").forEach(function (cb) { cb.checked = !!S.sizeBy[cb.getAttribute("data-sizeby")]; });
  }
  function applySetup(st) {
    if (!st || typeof st !== "object") return;
    ["tab", "persp", "xp", "q", "creator", "fam", "bench", "effortAny", "elig", "pooledOnly", "measuredOnly", "hideDisabled", "tasksMode", "baseline", "pfCut", "bandCount", "bandScale"].forEach(function (k) { if (st[k] !== undefined) S[k] = st[k]; });
    ["efforts", "off", "hidden", "sizeBy", "rec", "pj", "ineq"].forEach(function (k) { if (st[k] !== undefined) S[k] = st[k]; });
    POINTS = S.persp === "cai" ? CAI_POINTS : AA_POINTS;
    S.sel = null;
    showDetail(null);
    buildFilterOptions();
    buildBaselineOptions();
    syncRecInputs();
    syncPjInputs();
    renderIneqConds();
    syncStaticControls();
    render();
  }
  function loadSetups() {
    try { return JSON.parse(localStorage.getItem("xpl-setups-v1") || "{}"); } catch (e) { return {}; }
  }
  function storeSetups(obj) {
    try { localStorage.setItem("xpl-setups-v1", JSON.stringify(obj)); } catch (e) { /* storage unavailable: setups stay session-only */ }
  }
  function refreshSetupList(selected) {
    var sel = document.getElementById("xf-setup-list");
    if (!sel) return;
    var names = Object.keys(loadSetups()).sort();
    sel.innerHTML = names.length ? names.map(function (n) { return '<option value="' + esc(n) + '"' + (n === selected ? " selected" : "") + ">" + esc(n) + "</option>"; }).join("") : '<option value="">(none saved)</option>';
  }
  var setupSaveEl = document.getElementById("xf-setup-save");
  if (setupSaveEl) setupSaveEl.addEventListener("click", function () {
    var nameEl = document.getElementById("xf-setup-name");
    var name = nameEl && nameEl.value.trim() ? nameEl.value.trim() : "Setup " + (Object.keys(loadSetups()).length + 1);
    var all = loadSetups();
    all[name] = captureSetup();
    storeSetups(all);
    refreshSetupList(name);
  });
  var setupLoadEl = document.getElementById("xf-setup-load");
  if (setupLoadEl) setupLoadEl.addEventListener("click", function () {
    var sel = document.getElementById("xf-setup-list");
    if (sel && sel.value) applySetup(loadSetups()[sel.value]);
  });
  var setupExportEl = document.getElementById("xf-setup-export");
  if (setupExportEl) setupExportEl.addEventListener("click", function () { downloadFile("cost-explorer-setups.json", JSON.stringify(loadSetups(), null, 2), "application/json"); });
  var setupImportEl = document.getElementById("xf-setup-import");
  if (setupImportEl) setupImportEl.addEventListener("change", function () {
    var file = setupImportEl.files && setupImportEl.files[0];
    if (!file) return;
    var reader = new FileReader();
    reader.onload = function () {
      try {
        var parsed = JSON.parse(String(reader.result));
        if (parsed && typeof parsed === "object") {
          var all = loadSetups();
          Object.keys(parsed).forEach(function (k) { all[k] = parsed[k]; });
          storeSetups(all);
          refreshSetupList();
        }
      } catch (e) { /* unreadable file: nothing imported */ }
    };
    reader.readAsText(file);
  });
  var setupResetEl = document.getElementById("xf-setup-reset");
  if (setupResetEl) setupResetEl.addEventListener("click", function () {
    applySetup({
      tab: "aa", persp: "aa", xp: "api", q: "", creator: "", fam: "", bench: "", effortAny: false, efforts: {}, off: {},
      elig: false, pooledOnly: false, measuredOnly: false, hideDisabled: false, hidden: [], sizeBy: {}, tasksMode: "per",
      baseline: "", pfCut: "50", bandCount: 3, bandScale: 1,
      rec: { allow: 5, speedPriority: 85, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3, maxCost: "", maxSec: "" },
      pj: { turns: 3, inputTokens: 20000, answerTokens: 2000, thinkingTokens: 4000, cacheReadPct: 50, writesPct: 0, overheadSec: 10, window: "peak" },
      ineq: { mode: "off", combine: "and", conds: [{ metric: "score", op: ">=", value: "" }, { metric: "cost", op: "<=", value: "" }, { metric: "time", op: "<=", value: "" }] },
    });
  });
  function csvCell(v) { return '"' + String(v == null ? "" : v).replace(/"/g, '""') + '"'; }
  var dlJsonEl = document.getElementById("xf-dl-json");
  if (dlJsonEl) dlJsonEl.addEventListener("click", function () { downloadFile("cost-explorer-data.json", JSON.stringify(DATA, null, 2), "application/json"); });
  var dlCsvEl = document.getElementById("xf-dl-csv");
  if (dlCsvEl) dlCsvEl.addEventListener("click", function () {
    var lines = [];
    if (S.tab === "models") {
      lines.push(["model_id", "name", "provider", "plan", "context", "aa_score", "aa_cost_per_task", "proj_cost_per_task", "proj_time_per_task_s"].join(","));
      sortCatalog(visCatalog()).forEach(function (e) {
        var proj = project(e);
        lines.push([e.id, e.name, e.providerName, e.plan || "", e.contextLimit, e.aaScore, e.aaCostPerTask, proj.cost, proj.time].map(csvCell).join(","));
      });
      downloadFile("cost-explorer-models-visible.csv", lines.join("\\n"), "text/csv");
    } else {
      lines.push(["family", "effort", "creator", "benchmark", "harness", "score", "cost_per_task", "time_per_task_s", "ttft_s", "tokens_per_second", "tokens_per_task"].join(","));
      sortPoints(visPoints()).forEach(function (p) {
        lines.push([p.family, p.effort || "", p.creator, p.benchmark || "", p.harness || "", p.score, p.cost, p.time, p.ttft, p.tps, p.tokensPerTask].map(csvCell).join(","));
      });
      downloadFile("cost-explorer-configurations-visible.csv", lines.join("\\n"), "text/csv");
    }
  });
  renderIneqConds();
  refreshSetupList();
  buildFilterOptions();
  buildBaselineOptions();
  render();
})();
</script>`;

function fmtCost(v: number | null): string {
  return v == null ? "—" : `$${Number(v.toFixed(4))}`;
}

function fmtSec(v: number | null): string {
  if (v == null) return "—";
  if (v < 60) return `${Number(v.toFixed(1))}s`;
  return `${Math.floor(v / 60)}m ${Math.round(v % 60)}s`;
}

function fmtTimed(v: number | null): string {
  return v == null ? '<span class="muted">not measured</span>' : fmtSec(v);
}

/** "openai|gpt-6-luna|max|reasoning|fallback-na" -> "gpt-6-luna · max". */
function variantLabel(modelVariantKey: string): string {
  const parts = modelVariantKey.split("|");
  return parts.length >= 3 ? `${parts[1]} · ${parts[2]}` : modelVariantKey;
}

function aaTimingsTable(testRuns: AaTestRun[]): string {
  const rows = testRuns
    .map(
      (r) =>
        `<tr><td><strong>${esc(variantLabel(r.modelVariantKey))}</strong><br><span class="muted">${esc(r.snapshotDate ?? "no snapshot date")}</span></td><td>${r.intelligence ?? "—"}</td><td>${fmtCost(r.costPerTaskUsd)}</td><td>${fmtTimed(r.ttftSeconds)}</td><td>${fmtTimed(r.firstAnswerSeconds)}</td><td>${fmtTimed(r.e2e500Seconds)}</td><td>${fmtTimed(r.timePerTaskSeconds)}</td><td>${r.outputSpeedTps ?? "—"}</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table><caption>Model configurations — intelligence and cost</caption><tr><th scope="col">Configuration</th><th scope="col">Intelligence</th><th scope="col">$/task</th><th scope="col">TTFT</th><th scope="col">First answer</th><th scope="col">E2E (500 tok)</th><th scope="col">Time / task</th><th scope="col">Speed (t/s)</th></tr>${rows || '<tr><td colspan="8" class="muted">none</td></tr>'}</table></div>`;
}

function aaConfigsTable(configs: AaConfig[]): string {
  const rows = configs
    .map(
      (c) =>
        `<tr><td><strong>${esc(c.modelName)}</strong></td><td>${esc(c.effort ?? "—")}</td><td>${c.intelligence ?? "—"}</td><td>${fmtCost(c.costPerTaskUsd)}</td><td>${c.speedTps ?? "—"}</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table><caption>Models — effort, intelligence, cost, and speed</caption><tr><th scope="col">Model</th><th scope="col">Effort (AA label)</th><th scope="col">Intelligence</th><th scope="col">$/task</th><th scope="col">Speed (t/s)</th></tr>${rows || '<tr><td colspan="5" class="muted">none</td></tr>'}</table></div>`;
}

function aaCaiTable(rowsIn: AaCaiRow[]): string {
  const pct = (v: number | null): string => (v == null ? "—" : `${Number((v * 100).toFixed(1))}%`);
  const rows = rowsIn
    .map(
      (r) =>
        `<tr><td><strong>${esc(r.family)}</strong><br><span class="muted">${esc(r.creator)}${r.snapshotDate ? ` · ${esc(r.snapshotDate)}` : ""}</span></td><td>${esc(r.effort ?? "—")}</td><td>${esc(r.harness ?? "—")}</td><td>${esc(r.version ?? "—")}</td><td>${r.score ?? "—"}</td><td>${pct(r.deepSwe)}</td><td>${pct(r.terminalBench)}</td><td>${pct(r.sweAtlas)}</td><td>${fmtCost(r.cost)}</td><td>${fmtTimed(r.wallSec)}</td><td>${r.tokensPerTask != null ? Math.round(r.tokensPerTask).toLocaleString("en-US") : "—"}</td></tr>`,
    )
    .join("");
  return `<h2>Coding Agent Index observations</h2>
<p class="muted">The import's Coding Agent Index rows — a model plus its agent harness and settings, checked as published. Kept strictly separate from the AA Intelligence Index rows above: no score is converted between the indexes. "Not published" costs stay blank rather than estimated. These rows power the explorer's Coding agents perspective.</p>
<div class="tblwrap"><table><caption>Model benchmark scores</caption><tr><th scope="col">Model</th><th scope="col">Effort</th><th scope="col">Harness</th><th scope="col">Version</th><th scope="col">CAI score</th><th scope="col">DeepSWE v1.1</th><th scope="col">Terminal-Bench 4.0</th><th scope="col">SWE-Atlas-QnA</th><th scope="col">$/task</th><th scope="col">Agent wall time / task</th><th scope="col">Tokens / task</th></tr>${rows || '<tr><td colspan="11" class="muted">none</td></tr>'}</table></div>`;
}

function aaNameList(names: string[]): string {
  return names.length
    ? `<div class="strip">${names.map((n) => `<span class="badge src">${esc(n)}</span>`).join("")}</div>`
    : '<p class="muted">none</p>';
}

/** JSON for a <script type="application/json"> block: `<` is the only
 * character that can break out of it, so it crosses as its escape. */
function jsonForScript(json: string): string {
  return json.replace(/</g, "\\u003c");
}

export function renderAa(section: AaFeedSection | null, provenance: Provenance): string {
  if (!section) {
    return layout(
      "Cost Explorer",
      `<h1>Cost Explorer</h1><div class="banner">The AA data has not reached the dashboard yet — the fleet-feed collector reads it from Supabase (ainventory), and no AA section is in the current feed. The data of record is the ainventory import; this page only renders it.</div>`,
      provenance,
      "/aa",
    );
  }
  if (section.status === "absent") {
    return layout(
      "Cost Explorer",
      `<h1>Cost Explorer</h1><div class="banner">The collector could not build the AA section on its last run (${fmtDate(section.generatedAt)}): ${esc(section.reason)}. No successful collection exists yet to fall back to — the page fills in on the first successful collection.</div>`,
      provenance,
      "/aa",
    );
  }
  const s: AaSection = section;
  const summary = summarizeAa(s);
  const sourcesHtml = s.sources
    .map(
      (src) =>
        `<li><code>${esc(src.name)}</code> (${esc(src.kind)}) — imported ${src.importedAt ? fmtDate(src.importedAt) : "—"} · file modified ${src.driveModifiedAt ? fmtDate(src.driveModifiedAt) : "—"}</li>`,
    )
    .join("");
  const cov = s.coverage;
  const coverageHtml = cov
    ? `<h2>Coverage — measured but not pooled</h2>
<p class="muted">The collector compares the AA-measured model names against the repo's <code>.opencode/opencode.jsonc</code>: the provider catalogs and every route pool's leaf targets (nested routes expanded). Matching is best-effort — normalization plus a small alias map in <code>src/aa.ts</code> — so a miss means "not matched", not "does not exist".</p>
<h3>In a catalog, in no pool (${cov.catalogNotPooledMeasured.length})</h3>
<p class="muted">Measured and reachable today, but no route sends them work — the actionable list.</p>${aaNameList(cov.catalogNotPooledMeasured)}
<h3>Measured, in no catalog (${cov.measuredNotInCatalog.length})</h3>${aaNameList(cov.measuredNotInCatalog)}
<details><summary>Catalog models with no AA measurement and no pool seat (${cov.catalogNotPooledUnmeasured.length})</summary>
<p class="muted">Catalog ids verbatim from the config — this is where unmeasured newcomers (e.g. <code>commandcode/reflection/beam:free</code>, the Reflection Beam seat) sit until the import measures them.</p>${aaNameList(cov.catalogNotPooledUnmeasured)}</details>`
    : `<h2>Coverage — measured but not pooled</h2><p class="muted">Coverage was not computed for this section — the collector could not read <code>.opencode/opencode.jsonc</code> on that run.</p>`;
  const catalog = s.catalog;
  const modelCards = catalog
    ? `<div class="board">
<div class="card"><strong>${catalog.length} models</strong> in the fleet's provider catalogs<br><span class="muted">from <code>.opencode/opencode.jsonc</code>, read by the same collector run as the AA data</span></div>
<div class="card"><strong>${new Set(catalog.map((e) => e.provider)).size} providers</strong> · ${catalog.filter((e) => e.disabled).length} models behind disabled providers<br><span class="muted">Disabled means catalogued but not callable — the config disables the door, the proxy layer never does</span></div>
<div class="card"><strong>${catalog.filter((e) => e.routes.length > 0).length} pooled</strong> · ${catalog.filter((e) => e.aaScore !== null).length} AA-measured<br><span class="muted">Pooled = at least one route sends it work; measured = the import has an intelligence score for its name</span></div>
</div>`
    : `<div class="banner">The collector has not carried the fleet catalog yet — it is built from the same <code>.opencode/opencode.jsonc</code> read as the coverage section and appears once a collector build that knows it has run. No catalog is shown rather than a stale one.</div>`;
  const explorerJson = jsonForScript(
    JSON.stringify({ points: aaPoints(s), caiPoints: s.cai ? aaCaiPoints(s.cai) : [], catalog: catalog ?? null }),
  );
  const body = `<h1>Cost Explorer</h1>
${s.stale ? `<div class="banner">The latest collection failed (${esc(s.staleReason ?? "unknown error")}) — showing the last good data, collected ${fmtDate(s.generatedAt)}. It refreshes automatically on the next successful collection.</div>` : ""}
<p class="muted">${s.rankings.length} ranked configurations · ${s.configs.length} AA Intelligence Index configurations · ${s.testRuns.length} timing rows${s.cai ? ` · ${s.cai.length} Coding Agent Index observations` : ""}${catalog ? ` · ${catalog.length} catalog models` : ""} · collected ${fmtDate(s.generatedAt)} from ainventory batch <code>${esc(s.batch.slice(0, 8))}…</code>. Machine-first: <a href="/api/aa.json">/api/aa.json</a>.</p>
<div class="card"><strong>Provenance</strong><br>
<span class="muted">These are the import snapshot's figures, not a live Artificial Analysis pull.${summary.snapshotVintage ? ` Test_Runs snapshots in this batch: ${esc(summary.snapshotVintage.min)} – ${esc(summary.snapshotVintage.max)}.` : ""} A new import (coding-plan-watch) flips the batch; the feed collector picks it up within minutes. The explorer renders from the feed alone — when the data pipeline lands new models (Terra, Beam), they appear here with no page change.</span>
<ul>${sourcesHtml || '<li class="muted">source files not recorded for this batch</li>'}</ul></div>
<div class="xtabs" role="tablist"><button type="button" class="xtab" data-xtab="aa">AA Explorer</button><button type="button" class="xtab" data-xtab="models">Model Explorer</button></div>
<div class="card xpl-filters" id="xpl-filters">
<input type="search" id="xf-q" placeholder="Search models, families, providers…" aria-label="Search">
<label data-tabaa>Creator <select id="xf-creator"></select></label>
<label><span id="xf-fam-label">Family</span> <select id="xf-fam"></select></label>
<label data-tabaa>Benchmark <select id="xf-bench"></select></label>
<span class="xp-seg" role="group" aria-label="Efficiency perspective"><button type="button" data-xp="plan">Plan</button><button type="button" data-xp="api">API</button><button type="button" data-xp="pertask">Per-task</button></span>
<span class="xp-seg" role="group" aria-label="Benchmark perspective" data-tabaa><button type="button" data-persp="aa">Intelligence &amp; decode</button><button type="button" data-persp="cai">Coding agents</button></span>
<p class="muted" id="xpl-persp-note" data-tabaa style="flex-basis:100%;margin:2px 0 0"></p>
<div data-tabaa style="flex-basis:100%"><span class="muted">Efforts:</span> <span id="xf-efforts"></span> <label style="margin-left:10px"><input type="checkbox" id="xf-elig"> Eligible only</label></div>
<div data-tabmodels hidden style="flex-basis:100%"><label><input type="checkbox" id="xf-pooled"> Pooled only</label> <label><input type="checkbox" id="xf-measured"> AA-measured only</label> <label><input type="checkbox" id="xf-nodis"> Hide disabled providers</label></div>
<div data-tabaa style="flex-basis:100%"><details><summary>Model selection and advanced filters — inequality conditions</summary>
<label>Filter mode <select id="xf-ineq-mode"><option value="off" selected>Off</option><option value="keep">Keep matching models</option><option value="reject">Keep only rejected models</option></select></label>
<label>Combine conditions <select id="xf-ineq-combine"><option value="and" selected>Match all (AND)</option><option value="or">Match any (OR)</option></select></label>
<span class="muted">Pareto-gap metrics compare against the observed frontier of the current scope: score gap (score vs cost), price premium (cost vs score), speed gap (score vs time).</span>
<div id="xf-ineq-conds"></div>
<button type="button" class="xpl-ctl-btn" id="xf-ineq-add">+ Add condition</button>
</details></div>
<div style="flex-basis:100%">
<label>Settings name <input type="text" id="xf-setup-name" placeholder="Name this setup"></label>
<button type="button" class="xpl-ctl-btn" id="xf-setup-save">Save setup</button>
<label>Saved setups <select id="xf-setup-list"></select></label>
<button type="button" class="xpl-ctl-btn" id="xf-setup-load">Load</button>
<button type="button" class="xpl-ctl-btn" id="xf-setup-export">Export settings</button>
<label class="xpl-ctl-btn">Import <input type="file" id="xf-setup-import" accept="application/json" hidden></label>
<button type="button" class="xpl-ctl-btn" id="xf-setup-reset">Reset all settings</button>
<button type="button" class="xpl-ctl-btn" id="xf-dl-json">Download joined data (JSON)</button>
<button type="button" class="xpl-ctl-btn" id="xf-dl-csv">Download visible CSV</button>
</div>
<p class="muted" id="xp-note" style="flex-basis:100%;margin:2px 0 0"></p>
</div>
<div class="card xpl-detail" id="xpl-detail"></div>
<div class="card" id="xpl-tray" hidden><strong>Hidden</strong> <span id="xpl-traychips"></span></div>
<section id="xtab-aa">
<div id="xpl-legend" class="strip"></div>
<div class="xpl-charts" id="xpl-charts">
<div class="chart" data-chart="xpl-c1"><h3>Completion time × Score</h3><span class="xpl-ctl"><label><input type="checkbox" data-sizeby="xpl-c1"> size dots by cost</label><button type="button" data-dl="xpl-c1">Download PNG</button></span><svg id="xpl-c1" viewBox="0 0 680 400" role="img" aria-label="Completion time by score scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c2"><h3>Cost × Score</h3><span class="xpl-ctl"><label><input type="checkbox" data-sizeby="xpl-c2"> size dots by time</label><button type="button" data-dl="xpl-c2">Download PNG</button></span><svg id="xpl-c2" viewBox="0 0 680 400" role="img" aria-label="Cost by score scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c3"><h3>Cost × Completion time</h3><span class="xpl-ctl"><label><input type="checkbox" data-sizeby="xpl-c3"> size dots by score</label><button type="button" data-dl="xpl-c3">Download PNG</button></span><svg id="xpl-c3" viewBox="0 0 680 400" role="img" aria-label="Cost by completion time scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c4"><h3>Tasks completed vs duration</h3><span class="xpl-ctl"><button type="button" data-tmode="per">Seconds per task</button><button type="button" data-tmode="tot">Total time to finish all tasks</button><button type="button" data-dl="xpl-c4">Download PNG</button></span><svg id="xpl-c4" viewBox="0 0 680 400" role="img" aria-label="Tasks completed by duration scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c5"><h3>Multiples of the baseline</h3><span class="xpl-ctl"><label>Baseline <select id="xf-baseline"></select></label><button type="button" data-dl="xpl-c5">Download PNG</button></span><svg id="xpl-c5" viewBox="0 0 680 400" role="img" aria-label="Cost and duration multiples scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c6"><h3>Scores — every measured configuration</h3><span class="xpl-ctl"><button type="button" data-dl="xpl-c6">Download PNG</button></span><svg id="xpl-c6" viewBox="0 0 680 400" role="img" aria-label="Scores bar chart"></svg><p class="muted" data-note></p></div>
</div>
<p class="muted">Time is seconds to finish one task (log scale), not tokens/sec — a model can stream fast and still finish slowly if it burns many tokens, and per-task time here excludes time to first token (shown separately where timed). Thin lines join each model's effort rungs (DeepSeek ladders dotted); the white dashed line is that chart's Pareto frontier; white-ringed dots sit on it, dimmed dots are beaten on its axes. Bracket splits: cost $0.15 / $0.75 / $2.50, time 1m / 5m / 15m. Only measured configurations are plotted — nothing is estimated, so configurations AA never timed appear in the tables, not the time charts. DeepSeek prices split peak / off-peak: peak is 01:00–04:00 and 06:00–10:00 UTC Monday–Friday at double the off-peak token rates, so the same runs appear at two costs. Click a dot or a row for details.</p>
<h2>Multiples table</h2>
<p class="muted">Every visible configuration with all three measures, as multiples of the baseline chosen above, sorted by cost multiple.</p>
<div class="tblwrap"><table class="xpl-table"><caption>Multiples table — every visible configuration as multiples of the baseline</caption><thead><tr><th scope="col">Model</th><th scope="col">Effort</th><th scope="col" class="num">Score</th><th scope="col" class="num">Score ×</th><th scope="col" class="num xpl-costcol">Cost/task</th><th scope="col" class="num xpl-costcol">Cost ×</th><th scope="col" class="num">Time/task</th><th scope="col" class="num">Time ×</th><th scope="col">Benchmark</th></tr></thead><tbody id="xpl-mtrows"></tbody></table></div>
<h2>Configurations</h2>
<p class="muted" id="xpl-count"></p>
<div class="tblwrap"><table class="xpl-table"><caption>Configurations — intelligence, cost, and speed per model</caption><thead><tr><th scope="col">Model</th><th scope="col">Effort</th><th scope="col" class="num" id="xpl-scorehead">Intelligence</th><th scope="col" class="num xpl-costcol">$/task</th><th scope="col" class="num">Time/task</th><th scope="col" class="num">TTFT</th><th scope="col" class="num">Speed (t/s)</th><th scope="col" class="num xpl-costcol">Score / $</th><th scope="col" class="num">Score / s</th></tr></thead><tbody id="xpl-rows"></tbody></table></div>
<h2>Pareto frontiers</h2>
<p class="muted">Each list keeps variants that are not dominated on its named dimensions: another variant cannot be at least as good on every dimension and strictly better on one. The first two lists are the same score–speed frontier in different priority orders. Percentile ranks use the current filtered models; 100% is best. Missing measures are excluded only from a frontier that needs them.</p>
<label>Minimum score and speed percentile in the price shortlist <select id="xf-pfcut"><option value="50" selected>At least median · 50th percentile</option><option value="60">At least 60th percentile</option><option value="75">Top quarter · 75th percentile</option><option value="0">No cutoff · full Pareto frontier</option></select></label>
<p class="muted" id="xpl-pf-count"></p>
<h3>Score first, with competitive time</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — score first, with competitive time</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Score · value / percentile</th><th scope="col" class="num">Time / task · value / percentile</th><th scope="col" class="num xpl-costcol">Cost / task · value / percentile</th></tr></thead><tbody id="xpl-pf-score"></tbody></table></div>
<h3>Time first, with competitive score</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — time first, with competitive score</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Score · value / percentile</th><th scope="col" class="num">Time / task · value / percentile</th><th scope="col" class="num xpl-costcol">Cost / task · value / percentile</th></tr></thead><tbody id="xpl-pf-speed"></tbody></table></div>
<h3>Price first, strong in score and time</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — price first, strong in score and time</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Score · value / percentile</th><th scope="col" class="num">Time / task · value / percentile</th><th scope="col" class="num xpl-costcol">Cost / task · value / percentile</th></tr></thead><tbody id="xpl-pf-price"></tbody></table></div>
<h3>Price–speed trade-offs</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — price and speed trade-offs</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Time / task</th><th scope="col" class="num xpl-costcol">Cost / task</th><th scope="col" class="num">Score · context</th></tr></thead><tbody id="xpl-pf-ps"></tbody></table></div>
<h2>Recommendations</h2>
<p class="muted">Build a useful shortlist: first require a near-top score in each band, then trade price against speed and choose distinct labs. Score bands are non-overlapping and calculated from the filtered population before tradeoff limits; absolute limits apply first, then relative price/speed limits against the cheapest and fastest near-top models passing them. Unknown selected values cannot be recommended.</p>
<div class="card xpl-filters">
<span role="group" aria-label="Recommendation presets"><button type="button" class="xpl-ctl-btn" data-recpreset="fast">Fast near-top</button><button type="button" class="xpl-ctl-btn" data-recpreset="balanced">Balanced</button><button type="button" class="xpl-ctl-btn" data-recpreset="diversity">More lab diversity</button><button type="button" class="xpl-ctl-btn" data-recpreset="budget">Tight budget</button><button type="button" class="xpl-ctl-btn" data-recpreset="reset">Reset tradeoffs</button></span>
<label>Score bands <select id="xf-bands"><option value="2">2 tiers</option><option value="3" selected>3 tiers</option><option value="4">4 tiers</option><option value="5">5 tiers</option><option value="6">6 tiers</option><option value="8">8 tiers</option></select></label>
<label>Allow score below band best <input type="number" data-rec="allow" value="5" min="0" step="1" style="width:4em"> points</label>
<label>Favor price ← → favor speed <input type="range" data-rec="speedPriority" min="0" max="100" step="5" value="85"> <span id="rec-speedpriority-val">85% speed</span></label>
<label>Extra price above cheapest near-top <input type="number" data-rec="priceAllow" value="500" min="0" step="50" style="width:5em"> %</label>
<label>Slowest allowed vs fastest near-top <input type="number" data-rec="slowAllow" value="2" min="1" step="0.1" style="width:4em"> ×</label>
<label>Allow alternatives within <input type="number" data-rec="altWindow" value="2" min="1" step="0.1" style="width:4em"> × best tradeoff</label>
<label>Lab diversity <select data-rec="diversity"><option value="lab" selected>One choice per lab</option><option value="labsfirst">Different labs first</option><option value="none">No lab grouping</option></select></label>
<label>Remove dominated models <select data-rec="dominance"><option value="lab" selected>Within each lab</option><option value="all">Across all labs · strict Pareto</option><option value="none">Do not remove</option></select></label>
<label>At most per band <select data-rec="perBand"><option value="1">1</option><option value="2">2</option><option value="3" selected>3</option><option value="4">4</option><option value="5">5</option></select></label>
<label>Maximum USD / task <input type="number" data-rec="maxCost" placeholder="none" min="0" step="0.1" style="width:5em"></label>
<label>Maximum seconds / task <input type="number" data-rec="maxSec" placeholder="none" min="0" step="10" style="width:5em"></label>
<button type="button" class="xpl-ctl-btn" id="xpl-rec-export">Export shortlist CSV</button>
</div>
<p class="muted" id="xpl-rec-summary"></p>
<div id="xpl-recs"></div>
<h2>Band comparisons</h2>
<p class="muted">Best choices within each score and cost band; shortlists follow the current model and effort filters. Each band row shows the observed range of its other two measures with the endpoint configurations named.</p>
<label>Near-leader window <select id="xf-bandscale"><option value="0.5">0.5× · Very tight</option><option value="0.75">0.75× · Tight</option><option value="1" selected>1× · Standard (score ±2 · time within 10% · price within 25%)</option><option value="1.5">1.5× · Generous</option><option value="2">2× · Wide</option><option value="3">3× · Broad</option></select></label>
<div id="xpl-bandgroups"></div>
<h2>Timing — Test_Runs</h2>
<p class="muted">Per-configuration timing from the import's Test_Runs data. "Not measured" means the import carries no value for that cell — nothing is estimated.</p>
${aaTimingsTable(s.testRuns)}
<h2>Measured configurations</h2>
<p class="muted">The AA Intelligence Index rows of the latest import batch, effort labels verbatim from the import.</p>
${aaConfigsTable(s.configs)}
${s.cai ? aaCaiTable(s.cai) : ""}
</section>
<section id="xtab-models" hidden>
<h2>The fleet's model universe</h2>
<p class="muted">Every model in the fleet config's provider catalogs: the plan each door belongs to and its effective value multiplier where the registry records one, the pools that seat it, and its AA measurement when the import has one. Click a row for the plan note.</p>
${modelCards}
<h3>Task projection — estimated, not measured</h3>
<p class="muted">The task-assumptions projection from the ChatGPT coding explorer: set the shape of a task and every model with a recorded token rate gets an estimated cost and time, marked ~. Prices come only from the ainventory price event log joined onto this catalog (the band your per-turn input falls in); a model with no recorded rate — or missing a rate the assumptions need — shows —, never a guess. Time borrows the family's measured TTFT and throughput. Cache-storage hours are not modelled: the price log carries no storage rate.</p>
<div class="card xpl-filters">
<span role="group" aria-label="Task presets"><button type="button" class="xpl-ctl-btn" data-pjpreset="small">Small patch</button><button type="button" class="xpl-ctl-btn" data-pjpreset="feature">Feature</button><button type="button" class="xpl-ctl-btn" data-pjpreset="repo">Repo task</button></span>
<label>Turns <input type="number" data-pj="turns" value="3" min="1" style="width:4em"></label>
<label>Input / turn <input type="number" data-pj="inputTokens" value="20000" step="1000" style="width:6em"></label>
<label>Answer / turn <input type="number" data-pj="answerTokens" value="2000" step="500" style="width:5em"></label>
<label>Thinking / turn <input type="number" data-pj="thinkingTokens" value="4000" step="500" style="width:5em"></label>
<label>Cache read share <input type="range" data-pj="cacheReadPct" min="0" max="100" step="5" value="50"> <span id="pj-cacheread-val">50%</span></label>
<label>Writes / uncached <input type="number" data-pj="writesPct" value="0" min="0" max="100" style="width:4em"> %</label>
<label>Tool overhead / turn <input type="number" data-pj="overheadSec" value="10" min="0" style="width:4em"> s</label>
<label>Rate window <select data-pj="window"><option value="peak" selected>Peak (conservative)</option><option value="offpeak">Off-peak</option></select></label>
<span class="muted">The window applies to scheduled rates only (DeepSeek halves every token rate off-peak).</span>
</div>
<p class="muted" id="xpl-mcount"></p>
<div class="tblwrap"><table class="xpl-table"><caption>Model catalog — providers, plans, and projected costs</caption><thead><tr><th scope="col">Model</th><th scope="col">Provider / plan</th><th scope="col" class="num">Context</th><th scope="col">Variants</th><th scope="col">Pool seats</th><th scope="col" class="num">AA score</th><th scope="col" class="num xpl-costcol">AA $/task</th><th scope="col" class="num xpl-costcol">Eff. $/task</th><th scope="col" class="num">AA time/task</th><th scope="col" class="num xpl-costcol">Proj. $/task ~</th><th scope="col" class="num">Proj. time/task ~</th><th scope="col" class="num xpl-costcol">AA score / proj. $</th></tr></thead><tbody id="xpl-mrows"></tbody></table></div>
</section>
${coverageHtml}
<script type="application/json" id="xpl-data">${explorerJson}</script>`;
  return layout("Cost Explorer", body, provenance, "/aa", AA_EXPLORER_SCRIPT);
}

// --- Sessions / identity registry page (#906 §3) ------------------------------

const REGISTRY_SOURCE_ORDER = ["opencode-laptop", "opencode-sandbox", "jules", "claude-code", "cron"];

export function renderSessions(
  records: SessionRecord[],
  provenance: Provenance,
  detail?: ProvenanceDetail,
  act?: { actionsByObject?: Map<string, DashAction[]>; identities?: AuthorIdentity[] },
  view?: RegistryView,
): string {
  const bySource = new Map<string, SessionRecord[]>();
  for (const r of records) bySource.set(r.source, [...(bySource.get(r.source) ?? []), r]);
  const totalTokens = records.reduce((sum, r) => sum + (r.tokensTotal ?? 0), 0);
  const groups = [...bySource.entries()]
    .sort((a, b) => REGISTRY_SOURCE_ORDER.indexOf(a[0]) - REGISTRY_SOURCE_ORDER.indexOf(b[0]))
    .map(
      ([source, list]) =>
        `<h2>${esc(source)} (${list.length})</h2>${list
          .map((r) =>
            sessionRecordLine(r, {
              actions: act?.actionsByObject?.get(sessionObjectRef(r.id)),
              identities: act?.identities,
            }),
          )
          .join("")}`,
    )
    .join("");
  // #1209: when the server hands us a RegistryView, the page is one
  // windowed/filtered/paginated slice of the store — say so, with the
  // hidden counts, a search/filter form, and pagination links. Without
  // a view (older callers/tests) the page renders what it was given.
  let viewHtml = "";
  if (view) {
    const q = view.query;
    const windowLabel = q.windowDays === null ? "all history" : `last ${q.windowDays} days`;
    const filterBits = [
      q.q ? `search "${esc(q.q)}"` : "",
      q.agent ? `agent "${esc(q.agent)}"` : "",
      q.task ? `task "${esc(q.task)}"` : "",
    ].filter(Boolean);
    const hiddenBits = [
      view.hiddenByWindow > 0 ? `${view.hiddenByWindow} hidden by the ${windowLabel} window` : "",
      view.hiddenByPagination > 0 ? `${view.hiddenByPagination} more on other pages` : "",
    ].filter(Boolean);
    const toggle =
      q.windowDays === null
        ? `<a href="/sessions${registryQueryString(q, { windowDays: 7, offset: 0 })}">last 7 days</a>`
        : `<a href="/sessions${registryQueryString(q, { windowDays: null, offset: 0 })}">all history</a>`;
    const prev =
      q.offset > 0
        ? `<a href="/sessions${registryQueryString(q, { offset: Math.max(0, q.offset - q.limit) })}">← prev</a>`
        : "";
    const next =
      view.remainingAfterPage > 0
        ? `<a href="/sessions${registryQueryString(q, { offset: q.offset + q.limit })}">next →</a>`
        : "";
    viewHtml = `<div class="card"><strong>${view.matched} of ${view.totalAll} sessions</strong> · window: ${esc(windowLabel)}${filterBits.length ? ` · ${filterBits.join(" · ")}` : ""}${hiddenBits.length ? `<br><span class="muted">${hiddenBits.join(" · ")}</span>` : ""}${view.pinnedIncluded ? `<br><span class="muted">A deep-linked session outside this view is pinned at the top of the list.</span>` : ""}<br>${toggle}${prev ? ` · ${prev}` : ""}${next ? ` · ${next}` : ""}
<form method="get" action="/sessions"><label>Search <input type="search" name="q" value="${esc(q.q ?? "")}"></label> <label>Agent <input type="text" name="agent" value="${esc(q.agent ?? "")}"></label> <label>Task ref <input type="text" name="task" value="${esc(q.task ?? "")}"></label> <label>Window <select name="window"><option value="7"${q.windowDays === 7 ? " selected" : ""}>last 7 days</option><option value="1"${q.windowDays === 1 ? " selected" : ""}>last 24 hours</option><option value="30"${q.windowDays === 30 ? " selected" : ""}>last 30 days</option><option value="90"${q.windowDays === 90 ? " selected" : ""}>last 90 days</option><option value="all"${q.windowDays === null ? " selected" : ""}>all history</option></select></label> <button type="submit">Filter</button></form></div>`;
  }
  const body = `<h1>Sessions — identity registry</h1>
${viewHtml}<p class="muted">Every agent/session the fleet can see, with its stable id, the agent identity it ran as, its parent session, and its token total where the source reports one. Task links are loose (from the session's PR, title, or launch record) and improve over time. ${records.length} records shown · ${totalTokens.toLocaleString()} tokens recorded on this page. Machine-first: <a href="/api/registry.json">/api/registry.json</a>.</p>
<p class="muted">Coverage honesty: OpenCode sessions register from their servers; Jules from the keeper's state; Muse sessions only when a launch record was appended at spawn (<code>scripts/record-claude-launch.ts</code>) — a Muse session that never wrote one is invisible here, and the coverage note on Needs you says which sources are wired.</p>
${groups || '<p class="muted">No sessions recorded yet.</p>'}`;
  return layout(
    "Sessions",
    body,
    provenance,
    "/sessions",
    act?.identities ? ACTION_SCRIPT : "",
    detail ? freshnessChips(detail, ["registry"]) : "",
  );
}

// --- All pages (the manifest as a page) ---------------------------------------

export function renderPages(provenance: Provenance): string {
  const rows = PAGES.map(
    (p) =>
      `<tr><td><a href="${p.route}">${esc(p.title)}</a></td><td><code>${esc(p.route)}</code></td><td>${p.api ? `<a href="${p.api}"><code>${esc(p.api)}</code></a>` : '<span class="muted">—</span>'}</td><td>${esc(p.blurb)}</td></tr>`,
  ).join("");
  return layout(
    "All pages",
    `<h1>All pages</h1><p class="muted">Every registered page, from the manifest in <code>src/pages.ts</code> — the nav bar and the page dropdown render from the same list. Adding a page is one manifest entry + a render function + a route (recipe in the README).</p><div class="tblwrap"><table><caption>All registered pages</caption><tr><th scope="col">Page</th><th scope="col">Route</th><th scope="col">JSON twin</th><th scope="col">What it is</th></tr>${rows}</table></div>`,
    provenance,
    "/pages",
  );
}

// --- Health & topology (#1147 page, #1137 table) ---------------------------

/**
 * The /health page: one view assembled by src/health.ts from live
 * probes (cached 60 s), the feed, the section diagnosis, and this
 * process's render registry — the same view /api/health serves, so
 * the page and the API cannot disagree. States are named per row; a
 * failed probe, a stale feed, and an unrendered page all say so.
 */
export function renderHealth(view: HealthView, provenance: Provenance): string {
  const nowMs = Date.parse(view.generatedAt);
  const probedAge = (iso?: string): string =>
    iso ? ` · probed ${ageMsText(Math.max(0, nowMs - Date.parse(iso)))}` : "";
  const serviceRows = view.services
    .map((s) => {
      const port = s.def.port !== null ? String(s.def.port) : "—";
      const upWhen = s.def.upWhen ? `<br><span class="muted">up = ${esc(s.def.upWhen)}</span>` : "";
      const notes = s.def.notes ? `<br><span class="muted">${esc(s.def.notes)}</span>` : "";
      return `<tr><td><strong>${esc(s.def.name)}</strong>${upWhen}${notes}</td><td><code>${port}</code></td><td>${esc(s.def.bind)}</td><td><code>${esc(s.def.runsAs)}</code></td><td><span class="dot ${s.state}"></span><strong>${esc(s.state)}</strong> — ${esc(s.detail)}${probedAge(s.probedAt)}</td><td>${esc(s.def.consumers)}</td><td>${esc(s.def.breaksIfDown)}</td></tr>`;
    })
    .join("");
  const sectionRows = view.feed.sections.length
    ? view.feed.sections
        .map(
          (s) =>
            `<tr><td><code>${esc(s.key)}</code></td><td>${s.present ? "present" : '<span class="badge stale">absent</span>'}</td><td>${s.ageMs !== undefined ? esc(ageMsText(s.ageMs)) : "—"}</td><td>${s.note ? esc(s.note) : ""}</td></tr>`,
        )
        .join("")
    : `<tr><td colspan="4" class="muted">No readable fleet feed — sections cannot be reported.</td></tr>`;
  const orch = view.jobs.orchestration;
  const orchLine = orch.generatedAt
    ? `last run <code>${esc(orch.runId ?? "unknown")}</code> · ${esc(ageMsText(orch.ageMs ?? 0))} · state <strong>${esc(orch.state)}</strong>${orch.failedChecks ? ` · failed checks: ${esc(orch.failedChecks.join(", "))}` : ""} · next expected ~${esc(orch.nextExpectedAt ?? "")} (hourly cadence — an expectation, not a promise)`
    : `no run on record · state <strong>${esc(orch.state)}</strong>${orch.failedChecks ? ` · failed checks: ${esc(orch.failedChecks.join(", "))}` : ""}`;
  const coll = view.jobs.collector;
  const drift = view.feed.checkout;
  const driftLine =
    drift.state === "behind"
      ? ` · <span class="badge stale">collector checkout behind origin/main by ${drift.behindBy ?? "?"} commit(s)</span> — the collector is running old code`
      : drift.state === "in-sync"
        ? ` · collector checkout in sync with origin/main`
        : ` · collector checkout drift unknown: ${esc(drift.reason ?? "no reason given")}`;
  const collLine = coll.generatedAt
    ? `feed generated ${esc(ageMsText(coll.ageMs ?? 0))} · file written ${coll.fileMtime ? esc(coll.fileMtime) : "unknown"}${driftLine}`
    : "no readable fleet feed";
  const pageRows = view.pages
    .map((p) => {
      const last =
        p.lastRenderedAt === null
          ? `<span class="muted">not rendered since this process started (boot ${esc(view.bootAt)})</span>`
          : `${esc(ageMsText(p.ageMs ?? 0))} · ${p.ok ? "ok" : '<span class="badge stale">last render failed</span>'}`;
      return `<tr><td><a href="${esc(p.route)}">${esc(p.title)}</a></td><td><code>${esc(p.route)}</code></td><td>${last}</td></tr>`;
    })
    .join("");
  const cov = view.itemsCoverage;
  const coverageLine =
    cov && cov.total > 0
      ? `<strong>${cov.resolvedLive} of ${cov.total}</strong> board items resolved from live data (project pool / projector) · <strong>${cov.fixture}</strong> resolved from fixtures (rendered hollow + "unverified") · coverage ratio <strong>${cov.ratio !== null ? cov.ratio.toFixed(2) : "—"}</strong> — measured on the health prime's board resolution pass, the same pass that sets the items facet provenance (#1141).`
      : `not measured yet — coverage is computed on the health prime's board resolution pass (#1267); a pass that resolved no tagged items reports nothing rather than inventing a ratio.`;
  const facets = view.facets
    ? freshnessChips(view.facets, [
        "board",
        "roadmap",
        "items",
        "comments",
        "quota",
        "usage",
        "attention",
        "orchestration",
        "dns",
        "registry",
        "aa",
      ])
    : `<p class="muted">This source does not report structured source facets.</p>`;
  const degraded = view.degraded
    ? `<div class="flag">Something below is degraded — each row names its own state; nothing on this page is smoothed over.</div>`
    : "";
  const modeNote =
    view.mode === "fixtures"
      ? `<div class="banner">Fixtures mode — no live probes are made in this deployment; every service reports “fixtures”.</div>`
      : "";
  const body = `<h1>Health</h1>
<p class="muted">Every stack service probed live (results cached 60 s), every feed section aged, the last job runs, and this dashboard's own render history — assembled from the same view <a href="/api/health">/api/health</a> serves, at ${esc(view.generatedAt)}.</p>
${degraded}${modeNote}
<h2 id="topology">Services &amp; topology</h2>
<p class="muted">What runs where: the component, its port and bind as actually deployed, what consumes it, and what breaks without it. State is probed, never assumed.</p>
<div class="tblwrap"><table><caption>Services, ports, and topology</caption><tr><th scope="col">Service</th><th scope="col">Port</th><th scope="col">Bind</th><th scope="col">Runs as</th><th scope="col">State</th><th scope="col">Consumed by</th><th scope="col">If it is down</th></tr>${serviceRows}</table></div>
<h2>Feed sections</h2>
<div class="tblwrap"><table><caption>Feed sections — presence and age</caption><tr><th scope="col">Section</th><th scope="col">Present</th><th scope="col">Age</th><th scope="col">Note</th></tr>${sectionRows}</table></div>
<h2>Jobs</h2>
<div class="card"><strong>Orchestration pass</strong><br>${orchLine}</div>
<div class="card"><strong>Fleet-feed collector</strong><br>${collLine}</div>
<h2>Item resolution coverage</h2>
<div class="card">${coverageLine}</div>
<h2>Dashboard comments</h2>
<div class="card"><strong>${view.comments.pendingMessages} message${view.comments.pendingMessages === 1 ? "" : "s"} awaiting triage</strong> · <strong>${view.comments.pendingGithub} GitHub comment${view.comments.pendingGithub === 1 ? "" : "s"} awaiting posting</strong> · ${view.comments.total} comment${view.comments.total === 1 ? "" : "s"} in the store (#1145 — a pending count that does not drain means work arrives faster than it is cleared, or the triage/poster loop has stopped; check the loop's last run before acting)</div>
<h2>Source facets</h2>
${facets}
<h2>Pages</h2>
<p class="muted">Render history of this process — a page nobody has opened since boot is labelled unrendered, not green.</p>
<div class="tblwrap"><table><caption>Pages — last rendered</caption><tr><th scope="col">Page</th><th scope="col">Route</th><th scope="col">Last rendered</th></tr>${pageRows}</table></div>`;
  return layout("Health", body, provenance, "/health");
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

/** #1220: the coverage panel — Project #4 pool vs backlog reality per
 * repo (open issues and gate labels from the feed, pool size and unset
 * Status/Area from the pool), with the week-over-week delta against the
 * stored baseline in parentheses when a baseline exists. */
function coverageSectionHtml(coverage: CoverageSnapshotFile | null | undefined): string {
  if (!coverage) return "";
  const { current, baseline } = coverage;
  const delta = diffCoverage(current, baseline);
  const cell = (repo: string, v: number | null, key: "openIssues" | "gateNow" | "gateNext" | "poolItems" | "poolUnsetStatus" | "poolUnsetArea"): string => {
    if (v === null) return "—";
    const d = delta.get(repo)?.[key];
    return d === undefined || d === null || d === 0 ? `${v}` : `${v} <span class="muted">(${d > 0 ? "+" : ""}${d})</span>`;
  };
  const rows = current.repos
    .map(
      (r) =>
        `<tr><td>${esc(r.repo)}</td><td>${cell(r.repo, r.openIssues, "openIssues")}</td><td>${cell(r.repo, r.gateNow, "gateNow")}</td><td>${cell(r.repo, r.gateNext, "gateNext")}</td><td>${cell(r.repo, r.poolItems, "poolItems")}</td><td>${cell(r.repo, r.poolUnsetStatus, "poolUnsetStatus")}</td><td>${cell(r.repo, r.poolUnsetArea, "poolUnsetArea")}</td></tr>`,
    )
    .join("");
  const note = baseline
    ? `Week-over-week delta vs the ${esc(fmtDate(baseline.generatedAt))} snapshot in parentheses. Snapshot stored ${esc(fmtDate(current.generatedAt))}; it rotates into the baseline after a week.`
    : `First snapshot stored ${esc(fmtDate(current.generatedAt))} — the week-over-week delta appears after the next weekly rotation.`;
  return `<h2>Coverage — pool vs backlog (#1220)</h2>
<table><caption>Coverage — pool vs backlog by repository</caption><thead><tr><th scope="col">Repo</th><th scope="col">Open issues</th><th scope="col">now gate</th><th scope="col">next gate</th><th scope="col">Pool items</th><th scope="col">Unset Status</th><th scope="col">Unset Area</th></tr></thead><tbody>${rows}</tbody></table>
<p class="muted">${note} Open issues and gate labels come from the collector's Search API census in the fleet feed; pool counts from Project #4 (— = that source was unavailable for the repo, never a real zero). The same report is in <a href="/api/projects.json">/api/projects.json</a> as \`coverage\`.</p>`;
}

export function renderProjects(
  pool: ProjectPoolItem[],
  provenance: Provenance,
  opts: {
    area?: string | null;
    orchestration?: OrchestrationSection | null;
    detail?: ProvenanceDetail;
    actionsByObject?: Map<string, DashAction[]>;
    identities?: AuthorIdentity[];
    coverage?: CoverageSnapshotFile | null;
  } = {},
): string {
  const areas = [...new Set(pool.map((p) => p.area).filter((a): a is string => Boolean(a)))].sort();
  // #1143: Area-unset items get an explicit "(unset)" bucket, mirroring
  // the /program index banner — without it they are silently mixed into
  // "all" with no count and no way to view them as the gap they are.
  const UNSET_AREA = "(unset)";
  const unsetCount = pool.filter((p) => !p.area).length;
  const area =
    opts.area === UNSET_AREA && unsetCount > 0 ? UNSET_AREA : opts.area && areas.includes(opts.area) ? opts.area : null;
  const shown = area === UNSET_AREA ? pool.filter((p) => !p.area) : area ? pool.filter((p) => p.area === area) : pool;
  const chips = [
    `<a href="/projects"${!area ? ' class="active"' : ""}>all (${pool.length})</a>`,
    ...areas.map(
      (a) =>
        `<a href="/projects?area=${encodeURIComponent(a)}"${a === area ? ' class="active"' : ""}>${esc(a)} (${pool.filter((p) => p.area === a).length})</a>`,
    ),
    ...(unsetCount > 0
      ? [
          `<a href="/projects?area=${encodeURIComponent(UNSET_AREA)}"${area === UNSET_AREA ? ' class="active"' : ""}>${UNSET_AREA} (${unsetCount})</a>`,
        ]
      : []),
  ].join("");
  if (!pool.length) {
    return layout(
      "Projects",
      `<h1>Projects — GitHub Project pool</h1><div class="banner">No Project pool in the current fleet feed — the collector embeds it from the org Project <a href="${PROJECT_URL}">Developing Today (#4)</a> when a Projects token is available on the sandbox. Items on the board/roadmap fall back to the projector/fixtures chain until the pool lands.</div>${coverageSectionHtml(opts.coverage)}<p class="muted">Live services: ${serviceHomeLinks()
        .map((s) => `<a href="${esc(s.url)}">${esc(s.label)} ↗</a>`)
        .join(" · ")}</p>`,
      provenance,
      "/projects",
    );
  }
  const areaWeights = areas
    .map((a) => {
      const items = pool.filter((p) => p.area === a);
      const w = summarizeWeights(items);
      return `<div class="card"><strong>${esc(a)}</strong> — ${esc(weightedLine(w))}<br><span class="muted">${esc(handsLine(w))}</span></div>`;
    })
    .join("");
  const shownWeights = summarizeWeights(shown);
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
      // #1154: the status-request control + the store's visible state.
      const objectRef = ghObjectRef(p.ref);
      const statusCtl = opts.identities
        ? `${actionBadgesHtml(opts.actionsByObject?.get(objectRef))}${statusRequestHtml(objectRef, p.ref, p.status ?? null, { identities: opts.identities })}`
        : "";
      return `<div class="card"><strong>${esc(p.title)}</strong>${clashBadge}<br>${statusBadge} ${meta}<br><span class="muted"><a href="${esc(p.issueUrl)}">${esc(p.repo)}#${p.number}</a> · <a href="${esc(p.url)}">Project item</a> · <a href="${esc(p.projectUrl)}">Project #4</a>${p.updatedAt ? ` · updated ${fmtDate(p.updatedAt)}` : ""}</span>${statusCtl}</div>`;
    })
    .join("");
  const body = `<h1>Projects — GitHub Project pool</h1>
<p class="muted">${pool.length} items from the org Project <a href="${PROJECT_URL}">Developing Today (#4)</a>, read live by the fleet-feed collector (GraphQL, read-only) and carried in the feed. This pool is the resolution source for board/roadmap GitHub items. Where the Project Status and the derived state disagree, the card shows ⚠ with both values. Machine-first: <a href="/api/projects.json">/api/projects.json</a>.</p>
<div class="chips">${chips}</div>
${unsetCount > 0 ? `<div class="banner">${unsetCount} pool item${unsetCount === 1 ? "" : "s"} carry no Area — they belong to no project page until Area is set on the Project. Setting Area is the orchestrator's reconciliation job; this count keeps the gap visible. <a href="/projects?area=${encodeURIComponent(UNSET_AREA)}">View the (unset) bucket →</a></div>` : ""}
${coverageSectionHtml(opts.coverage)}
<p class="muted">Weighted progress (effort xs=1 · s=4 · m=12 · l=40, #1108): <strong>${esc(weightedLine(shownWeights))}</strong><br>${esc(handsLine(shownWeights))}</p>
${area ? "" : `<h2>Per-Area weighted progress</h2>${areaWeights}`}
<p class="muted">Live services: ${serviceHomeLinks()
    .map((s) => `<a href="${esc(s.url)}">${esc(s.label)} ↗</a>`)
    .join(" · ")}</p>
${cards || '<p class="muted">No items in this area.</p>'}`;
  return layout(
    "Projects",
    body,
    provenance,
    "/projects",
    opts.identities ? ACTION_SCRIPT : "",
    opts.detail ? freshnessChips(opts.detail, ["items"]) : "",
  );
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
      return item
        ? itemBadge(item)
        : `<span class="badge closed" title="The current source cannot resolve this ref">unresolved ${esc(r)}</span>`;
    })
    .join(" ");
  const links = (e.links ?? []).map((l) => `<a href="${esc(l.url)}">${esc(l.label)}</a>`).join(" · ");
  const meta = [refs, links].filter(Boolean).join(" ");
  return `<div class="card"><strong>${esc(e.title)}</strong><br>${esc(e.detail)}${meta ? `<br>${meta}` : ""}</div>`;
}

function programEntries(entries: ProgramEntry[], items: Map<RefKey, GhItem>): string {
  return entries.map((e) => programEntryHtml(e, items)).join("");
}

function countList(list: { name: string; count: number }[]): string {
  return list.length ? list.map((c) => `${esc(c.name)}: ${c.count}`).join(" · ") : '<span class="muted">none</span>';
}

function trackedTable(tracked: TrackedIssue[]): string {
  const rows = tracked
    .map(({ ref, item }) =>
      item
        ? `<tr><td>${itemBadge(item)}</td><td>${esc(item.title)}</td><td><span class="badge ${item.state === "open" ? "open" : item.state === "merged" ? "merged" : "closed"}">${esc(item.state)}</span></td><td>${fmtDate(item.updatedAt)}</td></tr>`
        : `<tr><td><span class="badge closed">unresolved</span></td><td><code>${esc(ref)}</code></td><td><span class="muted">unresolved — the current source cannot resolve this ref</span></td><td>—</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table><caption>Tracked issues</caption><tr><th scope="col">Tracked issue</th><th scope="col">Title</th><th scope="col">State</th><th scope="col">Updated</th></tr>${rows}</table></div>`;
}

function poolSummaryHtml(live: ProgramLive): string {
  return live.pool.total
    ? `<div class="card"><strong><a href="/projects">${live.pool.total} items</a></strong> in the org Project pool<br><span class="muted">By status — ${countList(live.pool.byStatus)}</span><br><span class="muted">By area — ${countList(live.pool.byArea)}</span></div>`
    : `<div class="banner">No Project pool in the current feed — the collector embeds it from the org Project when a Projects token is available. Pool summaries appear here when it lands; tracked issues still resolve through the projector/fixtures chain.</div>`;
}

function orchestrationHtml(live: ProgramLive): string {
  return live.orchestration
    ? `<div class="card"><strong>Orchestrator</strong> — latest run <code>${esc(live.orchestration.runId)}</code>, published ${fmtDate(live.orchestration.generatedAt)}.<br><span class="muted">The tiers and goal measurements it publishes are on <a href="/attention">Needs you</a> and <a href="/today">Today</a>.</span></div>`
    : `<div class="card"><strong>Orchestrator</strong> — <span class="muted">no orchestration section in the current feed yet (not measured). The Today view derives and labels itself until one lands.</span></div>`;
}

function projectCard(s: ProjectSummary): string {
  const poolLine =
    s.poolArea === null
      ? `<span class="muted">No dedicated Project Area yet.</span>`
      : `<strong><a href="/projects?area=${encodeURIComponent(s.poolArea)}">${s.poolTotal} pool items</a></strong> <span class="muted">(${esc(s.poolArea)})</span>${s.poolTotal ? `<br><span class="muted">Weighted — ${esc(weightedLine(s.weights))}</span><br><span class="muted">${esc(handsLine(s.weights))}</span><br><span class="muted">By status — ${countList(s.poolByStatus)}</span>` : ""}`;
  const roadmapLine =
    s.roadmapArea === null
      ? `<span class="muted">No Grist roadmap area yet.</span>`
      : `<span class="muted">${s.roadmapRows} roadmap row${s.roadmapRows === 1 ? "" : "s"} (${esc(s.roadmapArea)})</span>`;
  const homeLine = s.homeDoc
    ? `<br><span class="muted">Home doc: <a href="${esc(s.homeDoc.url)}">${esc(s.homeDoc.path)}</a></span>`
    : "";
  return `<div class="card"><strong><a href="${esc(s.route)}">${esc(s.title)}</a></strong><br>${poolLine}<br>${roadmapLine}${homeLine}<br><a href="${esc(s.route)}">Project page →</a></div>`;
}

/** /program as the projects index (#1121): one card per configured
 * project from the registry, the whole-pool + orchestrator live blocks
 * from #1046, and the unset-Area count as a visible gap. The full
 * pm-dashboard document moved to /program/pm-dashboard — every link
 * that pointed at /program still lands here. */
export function renderProgram(
  data: ProgramData,
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  detail?: ProvenanceDetail,
): string {
  const { program, live } = data;
  const projects = data.projects ?? [];
  const unset = live.pool.byArea.find((c) => c.name === "(unset)")?.count ?? 0;
  const cards = projects.map(projectCard).join("");
  const unsetHtml =
    live.pool.total && unset
      ? `<div class="banner">${unset} pool item${unset === 1 ? "" : "s"} carry no Area — they belong to no project page until Area is set on the Project. Setting Area is the orchestrator's reconciliation job; this count keeps the gap visible.</div>`
      : "";
  const body = `<h1>Program status — projects</h1>
<p class="muted">Every project the fleet runs, documented the same way: a description, its live Project-pool and roadmap sections, its tracked issues resolved live, and its roadmap — all driven by the project registry in <code>src/project-config.ts</code>. Pick a project for its full page.</p>
<p class="muted">Source of truth: the <a href="${esc(program.designRecordUrl)}">design record</a> in developing-today/code · the org Project <a href="${esc(program.projectUrl)}">Developing Today (#4)</a> · Machine-first: <a href="/api/program.json">/api/program.json</a>.</p>
<p class="muted">What runs where — every stack service, its port and bind, and its live health: <a href="/health#topology">Health &amp; topology →</a></p>
<h2>Projects</h2>
<div class="board">${cards || '<p class="muted">No projects configured.</p>'}</div>
${unsetHtml}
<h2>Live status</h2>
<p class="muted">Derived at request time from the Project pool and the latest orchestration section — not written by hand.</p>
${poolSummaryHtml(live)}
${orchestrationHtml(live)}
<h2>The PM dashboard's own record</h2>
<p class="muted">This dashboard's architecture, what's built, what's next, and its long-term roadmap live on its project page: <a href="/program/pm-dashboard">PM dashboard →</a>. Its tracked issues, resolved live:</p>
${trackedTable(live.tracked)}`;
  return layout(
    "Program status",
    body,
    provenance,
    "/program",
    "",
    detail ? freshnessChips(detail, ["orchestration", "items", "roadmap"]) : "",
  );
}

// --- Per-project page (#1121, registry in src/project-config.ts) -------------

const POOL_ITEM_CAP = 24;

/** The #2400 graph section of a project page: blocked weight + longest
 * blocked chain and the depth-graph SVG, from the part-1 edge contract
 * (#2399). Graceful absence is explicit: a pool with no effort labels
 * and no edges says "not yet sized/graphed" (never a fake 0%); a sized
 * pool without edges says "not yet graphed" only. */
function poolGraphHtml(items: ProjectPoolItem[], allW: ReturnType<typeof summarizeWeights>): string {
  const edges = hasGraphEdges(items);
  if (!edges && allW.totalWeight === 0) {
    return `<div class="banner">Not yet sized/graphed: no pool item for this project carries an effort/* label or an issue graph edge yet, so there is no weighted or graph view to draw. The counts above are raw item counts — not a 0% weighted score.</div>`;
  }
  if (!edges) {
    return `<p class="muted">Not yet graphed: pool items carry no parent / blocked-by edges yet, so blocked weight and the depth graph appear once the collector sees edges. Weighted progress above is from effort labels alone.</p>`;
  }
  const blocked = blockedSummary(items);
  const chainHtml =
    blocked.longestChain.length >= 2
      ? ` · longest blocked chain: ${blocked.longestChain.map((r) => `<a href="${esc(refUrl(r))}">${esc(r)}</a>`).join(" → ")} (${blocked.longestChain.length - 1} link${blocked.longestChain.length === 2 ? "" : "s"})`
      : "";
  const blockedHtml = blocked.blockedCount
    ? `<p class="muted"><strong>Blocked weight: ${blocked.blockedWeight}</strong> across ${blocked.blockedCount} open item${blocked.blockedCount === 1 ? "" : "s"} waiting on an unresolved blocker${blocked.blockedUnsizedCount ? ` (${blocked.blockedUnsizedCount} of them unsized — counted, never zero-weighted)` : ""}${chainHtml}.</p>`
    : `<p class="muted"><strong>Blocked weight: 0</strong> — no open item is waiting on an unresolved blocker.</p>`;

  const layout = depthLayout(items);
  const pos = new Map(layout.nodes.map((n) => [n.ref, n]));
  const X = (depth: number) => 16 + depth * 180;
  const Y = (row: number) => 16 + row * 26;
  const width = X(layout.maxDepth) + 340;
  const height = Y(layout.nodes.length - 1) + 16;
  const edgeLines = layout.edges
    .map((e) => {
      const a = pos.get(e.from)!;
      const b = pos.get(e.to)!;
      return `<line x1="${X(a.depth)}" y1="${Y(a.row)}" x2="${X(b.depth)}" y2="${Y(b.row)}" stroke="#8b949e" stroke-width="1"/>`;
    })
    .join("");
  const nodeShapes = layout.nodes
    .map((n) => {
      const label = `#${n.number} ${n.title.length > 34 ? `${n.title.slice(0, 33)}…` : n.title}`;
      const dash = n.mode === "hybrid" ? ` stroke-dasharray="3 2"` : "";
      const strokeW = n.mode === "physical" ? 3 : 1.5;
      const shape = n.epic
        ? `<rect x="${X(n.depth) - 6}" y="${Y(n.row) - 6}" width="12" height="12" fill="${n.state === "open" ? "#8957e5" : "#6e7781"}" stroke="#0d1117" stroke-width="${strokeW}"${dash}/>`
        : `<circle cx="${X(n.depth)}" cy="${Y(n.row)}" r="6" fill="${n.state === "open" ? "#bf8700" : "#6e7781"}" stroke="#0d1117" stroke-width="${strokeW}"${dash}/>`;
      return `${shape}<a href="${esc(n.issueUrl)}"><text x="${X(n.depth) + 11}" y="${Y(n.row) + 4}" font-size="11" fill="currentColor">${esc(label)}</text></a>`;
    })
    .join("");
  const truncated = layout.truncated
    ? `<p class="muted">Depth graph shows ${layout.nodes.length} of ${items.length} items (cap) — the full set is in the cards below.</p>`
    : "";
  const svg = layout.nodes.length
    ? `<svg viewBox="0 0 ${width} ${height}" width="100%" role="img" aria-label="Issue depth graph" style="max-width:100%;height:auto">${edgeLines}${nodeShapes}</svg><p class="muted">Depth = sub-issue nesting from the Project pool. Squares are epics; amber = open, grey = closed. Border: thick solid = mode/physical, dashed = mode/hybrid, thin = online or no mode.</p>${truncated}`
    : "";
  return `<h3>Blocked + depth graph</h3>${blockedHtml}${svg}`;
}

export function renderProgramProject(
  detail: ProjectDetail,
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  provDetail?: ProvenanceDetail,
): string {
  const { config } = detail;
  const linkBits: string[] = [];
  if (config.homeDoc) {
    linkBits.push(
      `Home doc: <a href="${esc(config.homeDoc.url)}">${esc(config.homeDoc.path)}</a> <span class="muted">(${esc(config.homeDoc.repo)})</span>`,
    );
  } else {
    linkBits.push(`<span class="muted">No home doc configured for this project yet.</span>`);
  }
  if (config.statusReport) {
    linkBits.push(`Status report: <a href="${esc(config.statusReport.url)}">${esc(config.statusReport.label)}</a>`);
  } else {
    linkBits.push(`<span class="muted">No status report yet for this project.</span>`);
  }
  for (const l of config.dashboardLinks ?? []) {
    linkBits.push(
      `<a href="${esc(l.route)}">${esc(l.label)}</a>${l.note ? ` <span class="muted">(${esc(l.note)})</span>` : ""}`,
    );
  }
  linkBits.push(`<a href="/api/program-project.json?id=${encodeURIComponent(config.id)}">JSON twin</a>`);

  const archHtml = config.architecture?.length
    ? `<h2>What this is — architecture</h2>${programEntries(config.architecture, items)}`
    : "";

  let poolHtml: string;
  if (config.poolArea === null) {
    poolHtml = `<div class="banner">No dedicated Project Area yet — this project's items are not in the org Project pool under their own Area. ${detail.unsetPoolCount} pool item${detail.unsetPoolCount === 1 ? "" : "s"} currently carry no Area at all. Its work is tracked in the tracked issues above and its home doc / links, where configured.</div>`;
  } else if (!detail.poolItems.length) {
    poolHtml = `<div class="banner">No pool items with Area <code>${esc(config.poolArea)}</code> in the current feed — the collector embeds the pool from the org Project when a Projects token is available, and reconciliation assigns Area.</div>`;
  } else {
    const shown = detail.poolItems.slice(0, POOL_ITEM_CAP);
    const cards = shown
      .map((p) => {
        const statusBadge = p.status
          ? `<span class="badge ${p.status === "Done" ? "merged" : p.status === "Todo" ? "closed" : "implementing"}">${esc(p.status)}</span>`
          : `<span class="badge">no status</span>`;
        const meta = [
          p.horizon ? `<span class="badge src">${esc(p.horizon)}</span>` : "",
          p.kind ? `<span class="badge src">${esc(p.kind)}</span>` : "",
          `<span class="badge ${p.state === "open" ? "open" : "closed"}">issue ${esc(p.state)}</span>`,
        ].join(" ");
        return `<div class="card"><strong>${esc(p.title)}</strong><br>${statusBadge} ${meta}<br><span class="muted"><a href="${esc(p.issueUrl)}">${esc(p.repo)}#${p.number}</a> · <a href="${esc(p.url)}">Project item</a>${p.updatedAt ? ` · updated ${fmtDate(p.updatedAt)}` : ""}</span></div>`;
      })
      .join("");
    const more =
      detail.poolItems.length > POOL_ITEM_CAP
        ? `<p class="muted">Showing ${POOL_ITEM_CAP} of ${detail.poolItems.length} — the full set is on <a href="/projects?area=${encodeURIComponent(config.poolArea)}">/projects</a>.</p>`
        : "";
    const allW = summarizeWeights(detail.poolItems);
    const epics = detail.poolItems.filter((p) => p.kind === "Epic");
    const epicW = summarizeWeights(epics);
    const epicRows = epics
      .map((p) => {
        const w = effortWeight(p.labels);
        return `<div class="card"><strong>${esc(p.title)}</strong><br><span class="muted">weight: ${w === null ? "unsized" : w} · issue ${esc(p.state)}${p.status ? ` · status: ${esc(p.status)}` : ""} · <a href="${esc(p.issueUrl)}">${esc(p.repo)}#${p.number}</a></span></div>`;
      })
      .join("");
    const epicHtml = epics.length
      ? `<h3>Epics — weighted progress</h3><p class="muted"><strong>${esc(weightedLine(epicW))}</strong></p>${epicRows}`
      : "";
    poolHtml = `<p class="muted"><strong>${esc(weightedLine(allW))}</strong><br>${esc(handsLine(allW))}<br><strong>${detail.poolItems.length} items</strong> with Area <code>${esc(config.poolArea)}</code> — by status: ${countList(detail.poolByStatus)}. Full pool view: <a href="/projects?area=${encodeURIComponent(config.poolArea)}">/projects?area=${esc(config.poolArea)}</a>.</p>${epicHtml}${poolGraphHtml(detail.poolItems, allW)}${cards}${more}`;
  }

  let roadmapHtml: string;
  if (config.roadmapArea === null) {
    roadmapHtml = `<div class="banner">No Grist roadmap area yet — this project has no rows in the Program roadmap table. Its horizons live in its home doc and tracked issues.</div>`;
  } else if (!detail.roadmapRows.length) {
    roadmapHtml = `<div class="banner">No roadmap rows with area <code>${esc(config.roadmapArea)}</code> in the current source.</div>`;
  } else {
    const rows = detail.roadmapRows
      .map((r) => {
        const links = r.links
          .map((l) => `<a class="badge src" href="${esc(refUrl(l.ref))}">${esc(l.ref)}</a>`)
          .join(" ");
        return `<div class="card"><strong>${esc(r.title)}</strong><br><span class="badge src">${esc(r.status)}</span> <span class="badge src">${esc(r.horizon)}</span>${links ? ` ${links}` : ""}${r.notes ? `<br><span class="muted">${esc(r.notes)}</span>` : ""}<br><span class="muted"><a href="/discuss/${encodeURIComponent(r.subject)}">💬 comments</a> · <a href="/roadmap">roadmap</a></span></div>`;
      })
      .join("");
    roadmapHtml = `<p class="muted">${detail.roadmapRows.length} row${detail.roadmapRows.length === 1 ? "" : "s"} with area <code>${esc(config.roadmapArea)}</code> in Grist's Program roadmap.</p>${rows}`;
  }

  const repoCardsHtml = detail.repoCards.length
    ? `<h2>Per-repo cards</h2>
<p class="muted">One card per working repo. Open-issue counts are a dated GitHub-search snapshot (date on each card), never a live claim; the pool items and roadmap rows are split live from the same sections above, by repo.</p>${detail.repoCards
        .map(({ card, poolItems, roadmapRows, tracked }) => {
          const refs = tracked
            .map(
              ({ ref, item }) =>
                `<a class="badge src" href="${esc(refUrl(ref))}">${esc(ref)}</a>${item ? ` <span class="badge ${item.state === "open" ? "open" : "closed"}">${esc(item.state)}</span>` : ""}`,
            )
            .join(" ");
          return `<div class="card"><strong><a href="https://github.com/${esc(card.repo)}">${esc(card.repo)}</a></strong><br>${esc(card.role)}<br><span class="muted">Open issues (GitHub search, ${esc(card.openIssuesAsOf)}): <strong>${card.openIssues}</strong> · fleet pool items in this repo: ${poolItems.length} · roadmap rows: ${roadmapRows.length}</span>${refs ? `<br>${refs}` : ""}</div>`;
        })
        .join("")}`
    : "";

  const sweepHealthHtml = config.sweepHealth?.length
    ? `<h2>Sweep health — published fleet state</h2>
<p class="muted">Built only from already-published fleet data — the orchestration section this aggregator serves and the fleet's tracked issues. No new data pipeline feeds this section.</p>${programEntries(config.sweepHealth, items)}`
    : "";

  const ed = config.editorial;
  const editorialHtml = ed
    ? `${ed.done.length ? `<h2>What's built</h2>${programEntries(ed.done, items)}` : ""}${ed.next.length ? `<h2>What's next</h2>${programEntries(ed.next, items)}` : ""}${ed.longTerm.length ? `<h2>Long-term roadmap</h2>${programEntries(ed.longTerm, items)}` : ""}${ed.gaps.length ? `<h2>Known gaps</h2>${programEntries(ed.gaps, items)}` : ""}`
    : "";

  const body = `<h1>${esc(config.title)}</h1>
<p class="muted">${esc(config.intro)}</p>
<p class="muted">${linkBits.join(" · ")}</p>
<p class="muted"><a href="/program">← all projects</a></p>
${archHtml}
<h2>Live status — tracked issues</h2>
<p class="muted">Resolved at request time from the same Source as every other view. A ref the current source cannot resolve says so — never an invented state.</p>
${trackedTable(detail.tracked)}
<h2>Project pool — live</h2>
${poolHtml}
<h2>Roadmap — live</h2>
${roadmapHtml}
${repoCardsHtml}
${sweepHealthHtml}
${editorialHtml}`;
  return layout(
    config.title,
    body,
    provenance,
    "/program",
    "",
    provDetail ? freshnessChips(provDetail, ["orchestration", "items", "roadmap"]) : "",
  );
}
