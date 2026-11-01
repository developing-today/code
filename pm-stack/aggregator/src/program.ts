// The program-status page (/program, #1046): the dashboard documenting
// ITSELF — what the PM stack is, what's built, what's next, and the
// long-term roadmap. Drew asked for it 2026-10-26: until now this
// record lived scattered across the design record, the slice issues,
// the org Project README, and agent memory — nowhere on the dashboard.
//
// Two halves, deliberately split:
//   - PROGRAM below is EDITORIAL: the architecture, the done list, the
//     next list, the long-term roadmap, the known gaps. It is the one
//     file to edit when the program changes; every entry carries its
//     tracker refs so a reader can verify it against GitHub.
//   - The LIVE half (buildProgramLive) is DERIVED at request time from
//     the Source — tracked-issue states via items(), the Project pool
//     summary via projects(), orchestration freshness via
//     orchestration() — so the page's status claims cannot silently
//     drift from the systems they describe. Anything a source cannot
//     resolve renders as "unresolved", never as an invented state.
import type { GhItem, OrchestrationSection, ProjectPoolItem, RefKey } from "./model.ts";
import { refUrl } from "./model.ts";
import { PROJECT_URL } from "./projects.ts";

export interface ProgramLink {
  label: string;
  url: string;
}

export interface ProgramEntry {
  title: string;
  detail: string;
  /** Tracker refs ("owner/repo#N") a reader can verify this entry against. */
  refs?: RefKey[];
  links?: ProgramLink[];
}

export interface ProgramDoc {
  intro: string;
  designRecordUrl: string;
  projectUrl: string;
  architecture: ProgramEntry[];
  done: ProgramEntry[];
  next: ProgramEntry[];
  longTerm: ProgramEntry[];
  gaps: ProgramEntry[];
}

const CODE = "developing-today/code";

const issue = (n: number): RefKey => `${CODE}#${n}`;
const issueLink = (n: number): ProgramLink => ({ label: `#${n}`, url: refUrl(issue(n), "issue") });
const prLink = (n: number): ProgramLink => ({ label: `PR #${n}`, url: refUrl(issue(n), "pr") });

/** The issues whose live state the /program page tracks. Resolved via
 * Source.items() at request time; a ref the source cannot resolve is
 * rendered "unresolved" — the fixtures resolve almost none of them. */
export const TRACKED_REFS: RefKey[] = [issue(898), issue(899), issue(902), issue(906), issue(910), issue(911)];

export const PROGRAM: ProgramDoc = {
  intro:
    "This dashboard is the aggregator of the PM stack: the one screen Drew lives in while the fleet works in GitHub, Kaneo, and Pad underneath. This page is the program's own record — what it is, what is built, what is next — with the status sections below derived live from the tracked issues, the Project pool, and the orchestrator, so they cannot drift quietly.",
  designRecordUrl: `https://github.com/developing-today/code/blob/main/doc/2026-10-24T11-05-38Z_design_project-management-stack/2026-10-24T11-05-38Z_design_project-management-stack.md`,
  projectUrl: PROJECT_URL,
  architecture: [
    {
      title: "GitHub — source of truth",
      detail:
        "Issues and PRs across the four working repos (developing-today/code, opencode, openchamber, dezren39/nix) are where work actually lives. Agents work only in GitHub issues.",
    },
    {
      title: "Org Project #4 — the canonical layer",
      detail:
        "One org Project, “Developing Today”, with Status / Area / Horizon / Kind fields. Area is the split dimension: splitting into multiple projects later is a filtered view, not a migration. The dashboard reads this pool on /projects.",
      links: [{ label: "Project #4", url: PROJECT_URL }],
    },
    {
      title: "Orchestrator — single writer of Project status",
      detail:
        "The hourly orchestration job (#899) is the only writer to the Project: it reconciles item status from issue/PR/session signals and auto-adds label-gated issues (now / next / epic). Nobody else writes the Project.",
      refs: [issue(899)],
    },
    {
      title: "Kaneo — per-repo execution board",
      detail:
        "The execution board under the dashboard's board view, one project per repo, running our fork (developing-today/kaneo) so multi-repo linking is on our schedule.",
    },
    {
      title: "ready-for-agent — dispatch",
      detail: "The dispatch layer that hands ready work to agent sessions.",
    },
    {
      title: "Pad — planning",
      detail:
        "The standalone planning layer for shared plans and docs; no board-card or roadmap-row Pad links are currently wired.",
    },
    {
      title: "Grist — roadmap & registers",
      detail:
        "The roadmap table and the registers (the DNS register behind /dns is #902's, rendered from Grist-side data).",
      refs: [issue(902)],
    },
    {
      title: "Projector + link registry",
      detail:
        "Projects GitHub state (items, PR-interior comments, cross-repo links) into the read model every view consumes — the cross-repo linking Kaneo cannot do.",
    },
    {
      title: "This aggregator — a read-mostly projection",
      detail:
        "Disposable by design: it never writes back to GitHub, Kaneo, Pad, or Grist (its only writes are dashboard-local comments), and if it rots, every layer underneath still works standalone.",
    },
  ],
  done: [
    {
      title: "Foundation phases + Wave 3",
      detail:
        "Phase 1 (Kaneo + our GitHub App on code) and Phase 2 (ready-for-agent, Pad, Grist), then Wave 3: connecting opencode/openchamber/nix as Kaneo projects, the Kaneo fork's first slice (multi-repo) merged, the projector + link registry, and the aggregator UI itself.",
    },
    {
      title: "Slice 1 — the aggregator live",
      detail: "The first dashboard: cross-repo WIP board + roadmap, live on the laptop (loopback + tailnet only).",
      links: [prLink(892)],
    },
    {
      title: "Slice 2 — mobile, tiers, horizons, Today",
      detail:
        "Mobile-first views; NEEDS USER vs NEEDS ORCHESTRATOR with a why-you reason and staleness expiry; roadmap horizons quarter/month/week/3-day/today; the /today view rendering the orchestrator's goal measurements; session source labels (OpenCode sandbox/laptop, Jules).",
      refs: [issue(898)],
      links: [prLink(901)],
    },
    {
      title: "Orchestrator — the hourly job",
      detail:
        "Reviews the needs-orchestrator / needs-decision queues, pushes work back into motion, escalates to Drew only when genuinely required, and measures the top goals against lined-up tasks (priority ladder: today > tomorrow > next 3 days > longer goals > incoming; dependency awareness beats ladder order).",
      refs: [issue(899)],
      links: [prLink(900)],
    },
    {
      title: "Slice 3 — pluggable pages, comments, session registry",
      detail:
        "The page manifest this page is registered in; the /dns register page; dashboard comments on items (message-to-Muse, plain note, post-as-GitHub-comment, always attributed); the agent/session identity registry.",
      refs: [issue(906)],
      links: [prLink(920)],
    },
    {
      title: "Oct 26 fixes — accuracy pass",
      detail:
        "/today crash fixed on both sides of the data contract; /quota corrected to current console reads with Meta shown walled, not healthy; the board gained its in-flight lane from the orchestrator's lined-up work; the board/roadmap GitHub layer switched from fixtures to the live Project pool.",
      links: [prLink(989), prLink(990), prLink(991), prLink(992), prLink(994)],
    },
    {
      title: "Org Project #910 — all 6 stages complete",
      detail:
        "Project created with its fields and backfill (27/27 items); Board and Roadmap views created (Start/Target date fields added); Stage 4 orchestrator reconciliation wired as the single writer; Stage 5 the dashboard reads the Project pool. Proof it works: the first fully automatic reconciliation pass set #910 itself to Done, and auto-added the nix epic #672.",
      refs: [issue(910)],
      links: [{ label: "Project #4", url: PROJECT_URL }],
    },
    {
      title: "Program pages generalized to every project",
      detail:
        "This page stopped being a single hard-coded document: projects are configured in a registry (src/project-config.ts), /program is the projects index, and every configured project — pm-dashboard, the basement cluster, id, the repo fleet, mcpx, the cloud program — has its own page under /program/<id> with live pool, roadmap, and tracked-issue sections. Epic #1129 tracks the remaining per-project build-out.",
      refs: [issue(1120), issue(1121)],
    },
  ],
  next: [
    {
      title: "Wire the work-picking jobs to the orchestrator",
      detail:
        "The last unimplemented program item: the fleet's work-picking jobs (issue sweeps, the issue creator) should consult the orchestrator's published priorities and blocked-by data first, falling back to their current rules when it is stale. Today only the orchestration pass itself reads it.",
      refs: [issue(911)],
    },
    {
      title: "GitHub comment projection off fixtures",
      detail:
        "Item pages render the GitHub comment stream live from the projector (deployed 2026-10-27, #1199); dashboard-native comments work too.",
    },
    {
      title: "Project Roadmap dates",
      detail:
        "The Project's Roadmap view has Start date / Target date fields, but items carry no date values yet, so no timeline bars draw. Dates should be proposed from each item's Horizon by the reconciliation pass, not invented by hand.",
      refs: [issue(910)],
    },
  ],
  longTerm: [
    {
      title: "PM-primary → general ops console",
      detail:
        "The dashboard grows beyond project management via pluggable pages — the DNS register was the first non-PM page; new registers and ops views register the same way this page did.",
      refs: [issue(906)],
    },
    {
      title: "Project splits are views, not migrations",
      detail:
        "The one org Project splits later by filtering on Area — the dashboard can show the whole pool or one area, and the underlying items never move.",
      refs: [issue(910)],
    },
    {
      title: "Deeper Kaneo fork",
      detail:
        "Full multi-repo projects on our fork, plus PR-interior sync and unified comment streams across repos — the fork exists so this is on our schedule, not upstream's.",
    },
    {
      title: "Laptop → Cozystack / k8s lab cluster",
      detail:
        "The stack is laptop-hosted containers for now, everything declared in developing-today/code; the eventual home is the basement Cozystack cluster. The aggregator being a disposable projection makes that move cheap.",
    },
  ],
  gaps: [
    {
      title: "Muse sessions have no runtime registry",
      detail:
        "Muse sessions are visible only via launch records appended at spawn (scripts/record-claude-launch.ts) — a session that never wrote one is invisible to the dashboard. OpenCode (both machines) and Jules are wired; this gap is also stated on the Needs you coverage note.",
      refs: [issue(898)],
    },
    {
      title: "GitHub comment projection is fixture-backed",
      detail: "See What's next — the dashboard's own comment store is live; the projected GitHub stream is not.",
    },
  ],
};

// --- Live half (derived, never hard-coded) ---------------------------------

export interface TrackedIssue {
  ref: RefKey;
  /** The resolved item, or null when the source cannot resolve the ref. */
  item: GhItem | null;
}

export interface PoolSummary {
  total: number;
  /** Counts keyed by Project Status value; unset counts under "(unset)". */
  byStatus: { name: string; count: number }[];
  /** Counts keyed by Project Area value; unset counts under "(unset)". */
  byArea: { name: string; count: number }[];
}

export interface OrchestrationFreshness {
  generatedAt: string;
  runId: string;
}

export interface ProgramLive {
  tracked: TrackedIssue[];
  pool: PoolSummary;
  orchestration: OrchestrationFreshness | null;
}

function counts(values: (string | null)[]): { name: string; count: number }[] {
  const map = new Map<string, number>();
  for (const v of values) {
    const key = v ?? "(unset)";
    map.set(key, (map.get(key) ?? 0) + 1);
  }
  return [...map.entries()]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

/** Assemble the live half of the page from already-fetched Source data.
 * Pure: absent items stay null, an empty pool yields zero counts, and a
 * null orchestration section stays null — the render layer words each
 * of those honestly. */
export function buildProgramLive(
  items: Map<RefKey, GhItem>,
  pool: ProjectPoolItem[],
  orchestration: OrchestrationSection | null,
): ProgramLive {
  return {
    tracked: TRACKED_REFS.map((ref) => ({ ref, item: items.get(ref) ?? null })),
    pool: {
      total: pool.length,
      byStatus: counts(pool.map((p) => p.status)),
      byArea: counts(pool.map((p) => p.area)),
    },
    orchestration: orchestration ? { generatedAt: orchestration.generatedAt, runId: orchestration.runId } : null,
  };
}

/** The /api/program.json shape: the editorial document plus the live half.
 * `projects` (the multi-project index summaries, #1121) is attached by
 * the server from src/project-config.ts; optional so the builder and
 * its tests keep working without the registry. */
export interface ProgramData {
  program: ProgramDoc;
  live: ProgramLive;
  projects?: import("./project-config.ts").ProjectSummary[];
}

export function buildProgramData(
  items: Map<RefKey, GhItem>,
  pool: ProjectPoolItem[],
  orchestration: OrchestrationSection | null,
): ProgramData {
  return { program: PROGRAM, live: buildProgramLive(items, pool, orchestration) };
}

/** Every ref the page wants resolved (tracked + every entry's refs),
 * de-duplicated, in first-seen order — the server resolves exactly this. */
export function programRefs(): RefKey[] {
  const out: RefKey[] = [];
  const seen = new Set<RefKey>();
  const add = (r: RefKey) => {
    if (!seen.has(r)) {
      seen.add(r);
      out.push(r);
    }
  };
  for (const r of TRACKED_REFS) add(r);
  for (const section of [PROGRAM.architecture, PROGRAM.done, PROGRAM.next, PROGRAM.longTerm, PROGRAM.gaps]) {
    for (const e of section) for (const r of e.refs ?? []) add(r);
  }
  return out;
}
