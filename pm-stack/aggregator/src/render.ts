// Pure HTML rendering. Every view function takes already-assembled data and
// returns a full HTML string — nothing here does I/O, which keeps the smoke
// tests cheap (render from fixtures and assert on output).
//
// Split by view into the render-*.ts modules (#13051); this file is the
// barrel — every public name is re-exported here so existing callers keep
// importing the same names from "./render.ts".

export {
  esc,
  fmtDate,
  fmtCentral,
  TAILNET_URL,
  FRAG_WATCHDOG_MS,
  FRAG_WATCHDOG_SCRIPT,
  freshnessChips,
  navLinks,
  pageDropdown,
  themeToggle,
  renderSearch,
  sessionObjectRef,
  ghObjectRef,
  tierItemObjectRef,
  commentBadge,
  authorBadge,
  dashCommentHtml,
  authorOptionsHtml,
  commentFormHtml,
  commentThreadHtml,
  relTime,
  actionBadgesHtml,
  cardDomId,
  actionHistoryHtml,
  cardCommentHtml,
  DISMISS_REASONS,
  DEESCALATE_REASONS,
  quickBarHtml,
  triageActionsHtml,
  whyUserBadge,
  needsUserBodyHtml,
  needsUserFormsHtml,
  delegationDefaultHtml,
  PROJECT_STATUS_VALUES,
  statusRequestHtml,
  sessionVerbSupport,
  sessionActionsHtml,
  sessionRecordIsLive,
  coverageHtml,
  briefPanelHtml,
} from "./render-core.ts";
export type { CommentFormOpts, ActionFormOpts } from "./render-core.ts";
export {
  renderBoardBody,
  renderBoard,
  renderBoardShell,
  buildRoadmapDrill,
  forecastSummaryHtml,
  renderRoadmap,
  eventTableHtml,
  FIXTURE_COMMENTS_CAUSE,
  commentsFixtureSourced,
  fixtureCommentsBannerHtml,
  renderItem,
  sessionRecordLine,
} from "./render-board.ts";
export type { RoadmapDrill, ItemExtras } from "./render-board.ts";
export { fmtMetricValue, freshnessStamp, renderQuota, historyChartHtml, renderServiceDetail } from "./render-quota.ts";
export {
  questionsSectionHtml,
  renderAttention,
  needsDrewCensusHtml,
  attentionSections,
  attentionFragment,
  renderAttentionShell,
} from "./render-attention.ts";
export type { AttentionAct } from "./render-attention.ts";
export {
  renderTodayBody,
  renderToday,
  renderTodayShell,
  renderQueueBody,
  renderQueue,
  renderQueueShell,
  deriveOverview,
  renderOverviewBody,
  renderOverview,
  renderOverviewShell,
} from "./render-today.ts";
export type {
  OverviewData,
  OverviewInProgressSignal,
  OverviewInProgressItem,
  OverviewSummary,
} from "./render-today.ts";
export { renderDns } from "./render-dns.ts";
export { renderAa } from "./render-aa.ts";
export {
  renderSessions,
  renderConflicts,
  renderPages,
  renderHealth,
  renderDiscuss,
  renderCommentsQueue,
  statusTaxonomyHtml,
} from "./render-views.ts";
export {
  projectsBodyHtml,
  renderProjects,
  renderProgram,
  phase12PanelHtml,
  phaseTrackerPanelHtml,
  buildHealthPanelHtml,
  f12ReadinessPanelHtml,
  queuePanelHtml,
  renderProgramProject,
} from "./render-program.ts";
export { renderAlerts, renderNotificationPreferences, renderNotifications, renderDigest } from "./render-alerts.ts";
export type { AlertsPageView, NotificationPreferencesView, NotificationsPageView } from "./render-alerts.ts";
