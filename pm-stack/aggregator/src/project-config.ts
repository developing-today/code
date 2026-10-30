// The project registry (#1120): one config entry per project is the
// single source the /program index, the /program/<id> pages, and their
// JSON twins all read. Adding a project is a config change here — not
// a code change per project (recipe in the README, "Add a project").
//
// A config says WHERE a project's live data lives (its org Project #4
// Area, its Grist roadmap area), WHERE its home doc lives, WHICH issues
// state the page resolves live, and any EDITORIAL sections (done / next
// / long-term / gaps) in the same ProgramEntry shape as src/program.ts.
// Everything countable is derived from the Source at request time by
// the builders below — no count or issue state is written by hand in a
// config. A project with no Area / roadmap area yet says so honestly
// (null), and the page renders the gap instead of an empty section.
import type { GhItem, ProjectPoolItem, RefKey, RoadmapRow } from "./model.ts";
import { refUrl } from "./model.ts";
import { PROGRAM, TRACKED_REFS, type ProgramEntry, type TrackedIssue } from "./program.ts";

export interface ProjectHomeDoc {
  repo: string;
  path: string;
  url: string;
}

export interface ProjectDashboardLink {
  label: string;
  /** A dashboard route ("/quota") or an absolute https URL (a source doc). */
  route: string;
  /** Optional caveat rendered beside the link (source-doc warnings). */
  note?: string;
}

export interface ProjectStatusReport {
  label: string;
  url: string;
}

export interface ProjectEditorial {
  done: ProgramEntry[];
  next: ProgramEntry[];
  longTerm: ProgramEntry[];
  gaps: ProgramEntry[];
}

export interface ProjectConfig {
  /** Slug used in /program/<id>. */
  id: string;
  title: string;
  /** One-paragraph description of the project. */
  intro: string;
  /** Org Project #4 Area value; null = no dedicated Area yet. */
  poolArea: string | null;
  /** Grist "Program roadmap" area value (RoadmapRow.area); null = none yet. */
  roadmapArea: string | null;
  homeDoc?: ProjectHomeDoc;
  /** Latest dated status report (spec layer 2), when one exists. */
  statusReport?: ProjectStatusReport;
  dashboardLinks?: ProjectDashboardLink[];
  /** Issues whose live state the page resolves via Source.items(). */
  trackedRefs: RefKey[];
  /** Extra architecture entries (the pm-dashboard doc renders its from here). */
  architecture?: ProgramEntry[];
  editorial?: ProjectEditorial;
}

const CODE = "developing-today/code";
const NIX = "dezren39/nix";
const blob = (path: string): string => `https://github.com/${CODE}/blob/main/${path}`;
const tree = (path: string): string => `https://github.com/${CODE}/tree/main/${path}`;
const home = (path: string): ProjectHomeDoc => ({ repo: CODE, path, url: blob(path) });
const ref = (repo: string, n: number): RefKey => `${repo}#${n}`;
const code = (n: number): RefKey => ref(CODE, n);

export const PROJECT_CONFIGS: ProjectConfig[] = [
  {
    id: "pm-dashboard",
    title: "PM dashboard",
    intro: PROGRAM.intro,
    poolArea: "pm-dashboard",
    roadmapArea: "PM stack",
    homeDoc: home(
      "doc/2026-10-24T11-05-38Z_design_project-management-stack/2026-10-24T11-05-38Z_design_project-management-stack.md",
    ),
    dashboardLinks: [
      { label: "Projects pool", route: "/projects" },
      { label: "Today", route: "/today" },
      { label: "Needs you", route: "/attention" },
    ],
    trackedRefs: TRACKED_REFS,
    architecture: PROGRAM.architecture,
    editorial: { done: PROGRAM.done, next: PROGRAM.next, longTerm: PROGRAM.longTerm, gaps: PROGRAM.gaps },
  },
  {
    id: "lab-cluster",
    title: "Basement cluster",
    intro:
      "A home lab built from the ~12 Threadripper machines in the basement: the first 3–5 become a Talos Linux cluster running Cozystack, bootstrapped declaratively with TALM, with CloudNativePG Postgres, RustFS, NATS, FluxCD, a Cloudflare tunnel and Tailscale on top. Hard rules: no HashiCorp anything, and everything bootstrappable from config — no hand-built state. Nothing is physically installed yet; basement power and space gate the first nodes, and egress (2 Gbps symmetric fiber) is the scarce resource, not compute.",
    poolArea: "lab-cluster",
    roadmapArea: "Basement cluster",
    homeDoc: home("planning/roadmaps/basement-cluster.md"),
    statusReport: {
      label: "Status report — 2026-10-27",
      url: tree("doc/2026-10-27T03-14-08Z_status_basement-cluster"),
    },
    dashboardLinks: [{ label: "Projects pool (lab-cluster)", route: "/projects?area=lab-cluster" }],
    trackedRefs: [
      code(1080),
      code(1081),
      code(1083),
      code(1084),
      code(1085),
      code(1086),
      code(1091),
      code(1087),
      code(1088),
      code(1090),
      code(1092),
      code(1093),
    ],
    editorial: {
      done: [
        {
          title: "Project structure seeded",
          detail:
            "The home doc (planning/roadmaps/basement-cluster.md) was created from the 2026-10-26 project-structure spec, the 12 epic issues below were filed and cross-linked, and the first dated status record was written (2026-10-27, linked from the home doc).",
        },
        {
          title: "v1.9-era test cluster marked historical",
          detail:
            "The earlier Talos test cluster was only a test and has been torn down; its layout docs are historical reference, not a live cluster.",
        },
      ],
      next: [
        {
          title: "Hardware inventory & first-node selection (Now, hybrid)",
          detail: "Gather hardware stats for all ~12 machines and pick the first 3–5 cluster nodes.",
          refs: [code(1080)],
        },
        {
          title: "Firmware / BIOS / BMC audit (Now, hybrid)",
          detail:
            "Confirm firmware versions and every setting are identical across the core machines, verified by routine KVM screenshots.",
          refs: [code(1081)],
        },
        {
          title: "Basement power & space (Now, physical)",
          detail: "Dedicated 20A circuits, cleanup and rack layout, so the first nodes can actually run.",
          refs: [code(1083)],
        },
        {
          title: "Network fabric, storage assembly, TALM + Talos bootstrap (Next)",
          detail:
            "Switching and a new addressing plan replacing the torn-down test layout; HDD RAID 10 pools and NVMe tiers validated per-drive; prove the declarative bootstrap flow including per-machine drives that are all different.",
          refs: [code(1084), code(1085), code(1086)],
        },
      ],
      longTerm: [
        {
          title: "Cozystack platform + platform services (Later)",
          detail:
            "Install Cozystack and settle its upgrade path; then CNPG Postgres, RustFS (against Cozystack's bundled SeaweedFS), NATS, FluxCD, Cloudflare tunnel and Tailscale.",
          refs: [code(1087), code(1088)],
        },
        {
          title: "Observability, backup & recovery (Later)",
          detail: "Metrics, an etcd and TALM recovery vault, restore drills, and a power-return boot order.",
          refs: [code(1090)],
        },
        {
          title: "Cloud extension (Next horizon, long tail)",
          detail: "The OCI always-free fleet, egress offload, and cold tiering to R2 alongside the home cluster.",
          refs: [code(1091)],
        },
        {
          title: "First workload: the id app on FluxCD (Later)",
          detail: "Run the id app as the cluster's first real workload.",
          refs: [code(1092)],
        },
      ],
      gaps: [
        {
          title: "Nothing physically installed yet",
          detail:
            "Basement power and space block the full fleet; the target is 3–5 machines first. All current progress is design, inventory, and preparation.",
        },
        {
          title: "OCI tenancy stuck in provisioning",
          detail:
            "The OCI tenancy has been stuck in provisioning since 2026-10-21 under Oracle ticket CPCOE-29381. That external blocker gates the cloud-extension epic until Oracle finishes provisioning or resolves the ticket.",
          refs: [code(1091)],
        },
        {
          title: "ESP32 wing is deliberately an idea",
          detail:
            "A low-end section of ESP32 sensors and LoRa alongside the compute fleet — a side project of a side project for now.",
          refs: [code(1093)],
        },
      ],
    },
  },
  {
    id: "id",
    title: "id platform",
    intro:
      "id is the peer-to-peer CLI built on Iroh. Its current growth area is the worlds platform: sandboxed Wasm programs (written in Roc) that run in shared, journaled worlds and render to browsers and SSH terminals. Drew's order (2026-10-26): Phase 12 is the goal, and the work needed to get to Phase 12 comes first — priority labels encode that order, and the dashboard mirrors it.",
    poolArea: "id-train",
    roadmapArea: "id",
    homeDoc: home("planning/roadmaps/id.md"),
    dashboardLinks: [
      { label: "Projects pool (id-train)", route: "/projects?area=id-train" },
      {
        label: "pkgs/id/ARCHITECTURE.md",
        route: blob("pkgs/id/ARCHITECTURE.md"),
        note: "source caveat: it still frames id mainly as a file-sharing CLI; the worlds platform is barely covered",
      },
      { label: "pkgs/id/README.md", route: blob("pkgs/id/README.md") },
      {
        label: "pkgs/id/TODO.md",
        route: blob("pkgs/id/TODO.md"),
        note: "the file itself warns that it is idle planning — do not decide from it",
      },
    ],
    trackedRefs: [
      code(534),
      code(796),
      code(1075),
      code(1076),
      code(1077),
      code(1078),
      code(807),
      code(557),
      code(806),
      code(812),
      code(793),
      code(1235),
      code(1239),
      code(1240),
    ],
    editorial: {
      done: [
        {
          title: "Phases 9–11 delivered; validation in progress",
          detail:
            "The home doc's careful wording is the status: Phases 9 and 10 were delivered faster than they were verified, and several Phase 11 items are claimed done on contradictory records. Dedicated validation audits (#1075–#1078, under umbrella #796) are in progress — delivered does not yet mean validated.",
          refs: [code(796), code(1075), code(1076), code(1077), code(1078)],
        },
        {
          title: "Phase 13 shells started",
          detail:
            "F13.1 (shell contract) and F13.2 (SSH frame transport) are done; the rest of Phase 13 waits behind Phase 12 foundations.",
          refs: [code(783), code(784)],
        },
      ],
      next: [
        {
          title: "The path to Phase 12 (Now)",
          detail:
            "A CI signal that can be trusted, validation of the delivered phases, journal format management (must land before any new journal entry type), cost visibility baselines, and the Phase 12 designs run through the research workflow. The ordered prerequisite queue is #1235.",
          refs: [code(229), code(1075), code(534), code(1235)],
        },
        {
          title: "Phase 12 — platform flexibility (Next)",
          detail:
            "Make the platform something other people can build on without changing its core: worlds as data (world.toml), a provider registry, first new providers, more guest languages, program upgrade with state migration, many worlds per node, journal replication, portals.",
          refs: [code(534), code(807), code(557), code(806)],
        },
      ],
      longTerm: [
        {
          title: "Phase 13 — client shells",
          detail:
            "SSH-over-iroh reachability, presentation capability tiers, native client shells on the shell contract, client-side capability providers, multi-server topology, and a world directory (worlds as places).",
          refs: [code(788), code(812)],
        },
        {
          title: "Worlds on Cloudflare Workers; group encryption",
          detail:
            "A pkarr option for running worlds on Workers (design merged; implementation deferred), and group key agreement so a world's documents are encrypted for current members only.",
          refs: [code(793), code(1070)],
        },
      ],
      gaps: [
        {
          title: "CI trust is the standing gap",
          detail:
            "The Nix check had no recorded success in 392 runs and the fast lane's exit (median PR feedback under 15 min) has never been measured. A red check must mean something before Phase 12 stacks on it.",
          refs: [code(229)],
        },
        {
          title: "Coverage: this page's pool is a subset of the id project",
          detail:
            "The live sections above draw on the Project #4 pool (Area=id-train) — a curated subset, not the whole project. GitHub search counted 327 open app/id issues on 2026-10-29, far more than the pool carries; read the pool as the prioritized slice, and the open app/id backlog as the project's full surface. Widening pool coverage is the orchestrator's label-gated auto-add plus an explicit backfill decision, tracked on the issue.",
          refs: [code(1125)],
        },
      ],
    },
  },
  {
    id: "repo-fleet",
    title: "Repo fleet",
    intro:
      "The four working repos and the automation that keeps them healthy: developing-today/code, developing-today/opencode, developing-today/openchamber, and dezren39/nix. Issue sweeps, the issue creator, PR shepherding, Jules, and the hourly orchestration job work these queues; this project tracks the fleet-level work — CI, automation, and triage — while each project's own checks live in its own roadmap.",
    poolArea: "fleet-ops",
    roadmapArea: "Repo fleet",
    homeDoc: home("planning/roadmaps/repo-and-ci.md"),
    dashboardLinks: [
      { label: "Board", route: "/" },
      { label: "Needs you", route: "/attention" },
      { label: "Projects pool (fleet-ops)", route: "/projects?area=fleet-ops" },
    ],
    trackedRefs: [
      code(911),
      code(899),
      code(287),
      code(288),
      code(1233),
      code(1130),
      code(1131),
      code(1234),
      code(1185),
      code(1187),
      code(1190),
      code(310),
      code(311),
    ],
    editorial: {
      done: [
        {
          title: "The fleet's jobs are live",
          detail:
            "Issue sweeps across all four repos, the hourly orchestration pass (queue triage, goal measurement, Project reconciliation), and the Jules keeper all run on schedule.",
          refs: [code(899)],
        },
        {
          title: "Two earlier CI items are closed",
          detail:
            "The intermittent nix flake check out-of-disk failure (#257) and the roughly two-hour id workflow gate (#292) are closed in GitHub. They are recorded here as completed work, not presented as upcoming; keeping the id gate fast is now a standing measurement watch.",
          refs: [code(257), code(292)],
        },
      ],
      next: [
        {
          title: "Work-picking jobs consult the orchestrator",
          detail:
            "The last unimplemented item of the original PM program: sweeps and the creator should read the orchestrator's published priorities and blocked-by data first, falling back to their current rules when it is stale.",
          refs: [code(911)],
        },
        {
          title: "CI hygiene and stale-PR triage (from the repo-and-ci roadmap)",
          detail:
            "Three Jules workflow files become one and the Go build cache is keyed by go.sum hash; in the same program, triage the overlapping Jules Bolt/Palette drafts and the stale Renovate/platform PRs instead of letting them age in place.",
          refs: [code(287), code(288), code(310), code(311)],
        },
        {
          title: "Queue fleet reliability",
          detail:
            "Give every repo queue lane-style long-budget shifts, and certify shift completion from queue state rather than run self-report, so the fleet can run without operator repairs.",
          refs: [code(1233), code(1130), code(1131)],
        },
        {
          title: "OpenChamber session dispatch + babysitter",
          detail:
            "Run larger code tasks as OpenChamber sessions on plan pools, with admission control, hand-backs, stall nudges, and harvesting; one babysitter watches sessions, and the OpenClaw comment loop is the pilot for session control by issue comment.",
          refs: [code(1234), code(1185), code(1187), code(1190)],
        },
      ],
      longTerm: [
        {
          title: "Fleet health judged from published state",
          detail:
            "The long-term direction is a fleet whose queue reliability, lifecycle labels, and sweep outcomes are visible from published queue and orchestration state — the same data this dashboard consumes — rather than from job self-reports.",
          refs: [code(1233), code(1131)],
        },
      ],
      gaps: [
        {
          title: "The pool is a fleet-level slice, not the fleet backlog (2026-10-27)",
          detail:
            "As of 2026-10-27, this page's fleet-ops pool carries 4 items, while GitHub issue search counts 665 open issues across the four repos (developing-today/code 372, developing-today/opencode 51, developing-today/openchamber 64, dezren39/nix 178). Most work stays in each repo's own backlog; this page tracks the fleet-level program only.",
        },
        {
          title: "Home doc scope is CI/automation only",
          detail:
            "planning/roadmaps/repo-and-ci.md covers workflows, Jules, and PR triage. Per-project CI detail lives in each project's own roadmap, not here.",
        },
      ],
    },
  },
  {
    id: "mcpx",
    title: "mcpx",
    intro:
      "mcpx is the Go MCP gateway and Code Mode runner in pkgs/mcpx: it pools MCP servers behind one daemon, runs TypeScript scripts against them, and hands a running daemon to its successor during upgrades. This repo is the only source for mcpx — the vendored copy in dezren39/nix was deleted 2026-10-19 — and the OpenCode and OpenChamber forks consume the result.",
    poolArea: null,
    roadmapArea: null,
    homeDoc: home("planning/roadmaps/mcpx.md"),
    dashboardLinks: [{ label: "Sessions", route: "/sessions" }],
    trackedRefs: [code(308), code(309), code(237), code(290), code(864)],
    editorial: {
      done: [
        {
          title: "Single source established",
          detail:
            "The dezren39/nix vendored copy was deleted on 2026-10-19 (dezren39/nix#468) and now builds mcpx from a pinned source-only input of this repo; OpenCode and OpenChamber consume that build.",
        },
      ],
      next: [
        {
          title: "Consumption decision + backlog re-homing (Now)",
          detail:
            "What remains of the single-source work: the downstream consumption decision, and re-homing the large mcpx backlog that still lives in dezren39/nix into this repo.",
          refs: [code(308), code(309)],
        },
        {
          title: "Darwin takeover tests; orphan-session test (Now)",
          detail:
            "Takeover/upgrade/handoff is unsupported on darwin and its Linux-only tests break the darwin build; the orphan-session test flakes in the Nix build and hides its own failure reason.",
          refs: [code(237), code(290)],
        },
        {
          title: "JSON Schema validation of tool input/output (Next)",
          detail: "Seven conformance rows depend on it; tracked in the nix-side backlog until re-homed.",
          refs: [ref(NIX, 254)],
        },
      ],
      longTerm: [
        {
          title: "Supervisor-owned stdio pipes; session resume",
          detail:
            "A supervisor that outlives the daemon makes upgrades a plain front-end restart; upstream sessions should resume after a cold restart instead of re-initialising every HTTP upstream.",
        },
        {
          title: "Write policy, principals, and HTTP auth",
          detail:
            "Ask/allow/deny per call with the exact call shown when asking; a bearer token for /v1 and /mcp; a principal per surface with per-operation permissions.",
        },
      ],
      gaps: [
        {
          title: "No Project Area or roadmap area yet",
          detail:
            "mcpx has no dedicated org Project Area and no Grist roadmap area, so this page's live pool/roadmap sections are honestly empty; its work is tracked in GitHub issues and planning/roadmaps/mcpx.md. The integration decision is tracked in #1127.",
          links: [{ label: "#1127", url: refUrl(code(1127), "issue") }],
        },
      ],
    },
  },
  {
    id: "cloud-program",
    title: "Cloud program & SaaS",
    intro:
      "Every cloud account, plan, and subscription the fleet runs on — Cloudflare, Turso, OCI, the AI plans — and what each costs against its allotment. The Usage & billing hub renders the master tracker (workspace: ai-subscriptions) with dated reads and pace verdicts; the account-connection campaign wired API/OAuth access for nearly everything. The standing aim is ~100% of paid allotments, with pricing revalidated against official docs.",
    poolArea: null,
    roadmapArea: "Cloud program",
    dashboardLinks: [
      { label: "Usage & billing", route: "/quota" },
      { label: "DNS register", route: "/dns" },
    ],
    trackedRefs: [code(967), code(1047)],
    editorial: {
      done: [
        {
          title: "Usage & billing hub live",
          detail:
            "The /quota hub renders the master tracker: pools, SaaS services, PAYG lines, charges evidence, and per-service detail pages.",
          refs: [code(967)],
        },
        {
          title: "Account-connection campaign (mostly)",
          detail:
            "Services were connected via API keys/OAuth everywhere possible, tokens vaulted in Bitwarden/1Password, no new MFA unless forced. The known remainder is Plaid (below).",
        },
      ],
      next: [
        {
          title: "Hub build-out under its epic",
          detail:
            "Open checks, live collectors per service, daily + event-driven console reads, dashboard provenance, and the free-services completion pass.",
          refs: [code(1047)],
        },
        {
          title: "OCI signup resolution",
          detail:
            "The Oracle Cloud signup is still pending; the CPCOE-29381 follow-up is scheduled. The target is the Always-Free allocation (a hard allocation, not metered billing) — the account is not to be upgraded to pay-as-you-go.",
        },
        {
          title: "Free-services pass — adoption decisions",
          detail:
            "The 2026-10-26 SaaS review list — boats.cloud, quic.cloud, Backblaze B2, OVHcloud, Railway, Render, DigitalOcean, Hetzner, Heroku, GCP, AWS — was evaluated on 2026-10-29 in the workspace tracker (ai-subscriptions/free-services-to-check.md), each service with an overage verdict under the no-surprise-bills guardrail. What remains is the per-service adopt/skip decision; boats.cloud stays unverified and untouched (no official site resolves).",
        },
      ],
      longTerm: [
        {
          title: "Every plan paced against its reset",
          detail:
            "Usage percentages refreshed on a daily cadence, allotments aimed at ~100% use, and any claimed plan change diffed against the dated evidence baseline before the tracker moves.",
        },
        {
          title: "Egress offload",
          detail:
            "Bulk egress rides zero-egress lanes (Cloudflare R2 charges no egress; Backblaze B2 egress is free via listed partners such as Cloudflare) so metered clouds carry compute and storage, not bandwidth bills.",
        },
        {
          title: "Always-free maximization",
          detail:
            "Always-free allocations (OCI Always Free, the Cloudflare/Turso/Supabase free tiers, vendor free plans that wall rather than bill) are filled first, and paid allotments are paced to ~100% use before any new spend is added.",
        },
      ],
      gaps: [
        {
          title: "No Project Area yet",
          detail:
            "This program has a Grist roadmap area (its rows render in the live roadmap section above), but it is not in the org Project pool under its own Area yet — its work is tracked in the ai-subscriptions workspace tracker and its hub epic, so this page's live pool section is honestly empty.",
        },
        {
          title: "Plaid / Citi still not linked",
          detail:
            "Plaid blocks Muse's browser, so linking Citi (Costco, Diamond, Double Cash) needs the user's own browser session. Several vendor meters are also console-only, so some /quota numbers are dated manual reads rather than live meters.",
        },
      ],
    },
  },
];

export function projectConfigById(id: string): ProjectConfig | undefined {
  return PROJECT_CONFIGS.find((c) => c.id === id);
}

export function projectRoute(id: string): string {
  return `/program/${id}`;
}

// --- Validation (strict in tests, never throws at render) ------------------

const REF_PATTERN = /^[^/\s]+\/[^#\s]+#\d+$/;
const ID_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

function checkRefs(refs: RefKey[], errors: string[], where: string): void {
  for (const r of refs) {
    if (!REF_PATTERN.test(r)) errors.push(`${where}: malformed ref "${r}" (want owner/repo#N)`);
  }
}

function checkEntries(entries: ProgramEntry[] | undefined, errors: string[], where: string): void {
  for (const e of entries ?? []) checkRefs(e.refs ?? [], errors, where);
}

/** Validate a registry: returns human-readable errors (empty = valid).
 * Catches duplicate ids, duplicate pool/roadmap areas, malformed slugs
 * and refs, home docs / status reports without a usable URL, and
 * dashboard links that are neither routes nor https URLs. Pure — the
 * shipped registry is asserted valid in tests. */
export function validateProjectConfigs(configs: ProjectConfig[]): string[] {
  const errors: string[] = [];
  const seenIds = new Map<string, number>();
  const seenPoolAreas = new Map<string, string>();
  const seenRoadmapAreas = new Map<string, string>();
  configs.forEach((c, i) => {
    const where = `configs[${i}] (${c.id || "?"})`;
    if (!ID_PATTERN.test(c.id)) errors.push(`${where}: id is not a slug`);
    seenIds.set(c.id, (seenIds.get(c.id) ?? 0) + 1);
    if (c.poolArea !== null) {
      const prev = seenPoolAreas.get(c.poolArea);
      if (prev) errors.push(`${where}: poolArea "${c.poolArea}" also used by ${prev}`);
      seenPoolAreas.set(c.poolArea, c.id);
    }
    if (c.roadmapArea !== null) {
      const prev = seenRoadmapAreas.get(c.roadmapArea);
      if (prev) errors.push(`${where}: roadmapArea "${c.roadmapArea}" also used by ${prev}`);
      seenRoadmapAreas.set(c.roadmapArea, c.id);
    }
    if (!c.title.trim()) errors.push(`${where}: title is empty`);
    if (!c.intro.trim()) errors.push(`${where}: intro is empty`);
    if (c.homeDoc && !/^https:\/\//.test(c.homeDoc.url)) errors.push(`${where}: homeDoc.url is not an https URL`);
    if (c.statusReport) {
      if (!c.statusReport.label.trim()) errors.push(`${where}: statusReport.label is empty`);
      if (!/^https:\/\//.test(c.statusReport.url)) errors.push(`${where}: statusReport.url is not an https URL`);
    }
    for (const l of c.dashboardLinks ?? []) {
      if (!l.label.trim()) errors.push(`${where}: dashboardLinks label is empty`);
      if (!l.route.startsWith("/") && !/^https:\/\//.test(l.route))
        errors.push(`${where}: dashboardLinks route "${l.route}" is neither a dashboard route nor an https URL`);
    }
    checkRefs(c.trackedRefs, errors, where);
    checkEntries(c.architecture, errors, where);
    checkEntries(c.editorial?.done, errors, where);
    checkEntries(c.editorial?.next, errors, where);
    checkEntries(c.editorial?.longTerm, errors, where);
    checkEntries(c.editorial?.gaps, errors, where);
  });
  for (const [id, n] of seenIds) {
    if (n > 1) errors.push(`duplicate project id "${id}" (${n} entries)`);
  }
  return errors;
}

// --- Live builders (derived from Source data, never hard-coded) ------------

export interface Count {
  name: string;
  count: number;
}

function counts(values: (string | null)[]): Count[] {
  const map = new Map<string, number>();
  for (const v of values) {
    const key = v ?? "(unset)";
    map.set(key, (map.get(key) ?? 0) + 1);
  }
  return [...map.entries()]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

export interface ProjectSummary {
  id: string;
  title: string;
  route: string;
  intro: string;
  poolArea: string | null;
  roadmapArea: string | null;
  /** Pool items in this project's Area; 0 when poolArea is null. */
  poolTotal: number;
  poolByStatus: Count[];
  /** Grist rows in this project's roadmapArea; 0 when null. */
  roadmapRows: number;
  homeDoc: ProjectHomeDoc | null;
}

export function buildProjectSummary(
  config: ProjectConfig,
  pool: ProjectPoolItem[],
  rows: RoadmapRow[],
): ProjectSummary {
  const mine = config.poolArea === null ? [] : pool.filter((p) => p.area === config.poolArea);
  return {
    id: config.id,
    title: config.title,
    route: projectRoute(config.id),
    intro: config.intro,
    poolArea: config.poolArea,
    roadmapArea: config.roadmapArea,
    poolTotal: mine.length,
    poolByStatus: counts(mine.map((p) => p.status)),
    roadmapRows: config.roadmapArea === null ? 0 : rows.filter((r) => r.area === config.roadmapArea).length,
    homeDoc: config.homeDoc ?? null,
  };
}

export function buildProjectSummaries(
  configs: ProjectConfig[],
  pool: ProjectPoolItem[],
  rows: RoadmapRow[],
): ProjectSummary[] {
  return configs.map((c) => buildProjectSummary(c, pool, rows));
}

/** Pool items carrying no Area — they belong to no project page until
 * Area is set; the index surfaces this count so the gap stays visible. */
export function unsetPoolCount(pool: ProjectPoolItem[]): number {
  return pool.filter((p) => p.area === null).length;
}

export interface ProjectDetail {
  config: ProjectConfig;
  route: string;
  poolItems: ProjectPoolItem[];
  poolByStatus: Count[];
  roadmapRows: RoadmapRow[];
  tracked: TrackedIssue[];
  /** Whole-pool items with no Area (the gap note for null-area projects). */
  unsetPoolCount: number;
}

export function buildProjectDetail(
  config: ProjectConfig,
  items: Map<RefKey, GhItem>,
  pool: ProjectPoolItem[],
  rows: RoadmapRow[],
): ProjectDetail {
  const poolItems =
    config.poolArea === null
      ? []
      : pool
          .filter((p) => p.area === config.poolArea)
          .sort((a, b) => (a.status ?? "").localeCompare(b.status ?? "") || a.title.localeCompare(b.title));
  return {
    config,
    route: projectRoute(config.id),
    poolItems,
    poolByStatus: counts(poolItems.map((p) => p.status)),
    roadmapRows: config.roadmapArea === null ? [] : rows.filter((r) => r.area === config.roadmapArea),
    tracked: config.trackedRefs.map((ref) => ({ ref, item: items.get(ref) ?? null })),
    unsetPoolCount: unsetPoolCount(pool),
  };
}

/** Every ref any project page wants resolved (tracked + editorial +
 * architecture), de-duplicated, first-seen order — the server resolves
 * exactly this set for the project routes. */
export function allProjectRefs(configs: ProjectConfig[] = PROJECT_CONFIGS): RefKey[] {
  const out: RefKey[] = [];
  const seen = new Set<RefKey>();
  const add = (r: RefKey) => {
    if (!seen.has(r)) {
      seen.add(r);
      out.push(r);
    }
  };
  for (const c of configs) {
    for (const r of c.trackedRefs) add(r);
    const sections = [
      c.architecture ?? [],
      c.editorial?.done ?? [],
      c.editorial?.next ?? [],
      c.editorial?.longTerm ?? [],
      c.editorial?.gaps ?? [],
    ];
    for (const entries of sections) for (const e of entries) for (const r of e.refs ?? []) add(r);
  }
  return out;
}
