// Fixture source: realistic data shaped exactly like the live readers'
// output, derived from the design record's data-flow section and the
// as-built phases. The roadmap rows are the real Grist seed
// (pm-stack/grist/seed/roadmap.json); registry links, live item state and
// comments are hand-written and illustrative — numbers that the design
// record names (#752, #754, #762, #763, nix #501–#508, opencode #365) are
// real refs, the rest are made up and say so in their titles' context.
import seed from "../../../grist/seed/roadmap.json" with { type: "json" };
import {
  type BoardItem,
  type Comment,
  type GhItem,
  type Link,
  type Provenance,
  type RefKey,
  type RoadmapRow,
  refsInText,
  refUrl,
} from "../model.ts";
import type { Source } from "../source.ts";

const GRIST_DOC = "czYHYKgG9SHcm9CPwiKEq1"; // "Program roadmap"
const CODE = "developing-today/code";
const OPENCODE = "developing-today/opencode";
const OPENCHAMBER = "developing-today/openchamber";
const NIX = "dezren39/nix";

function gh(
  ref: RefKey,
  kind: GhItem["kind"],
  title: string,
  state: GhItem["state"],
  extra: Partial<GhItem> = {},
): GhItem {
  return {
    ref,
    kind,
    url: refUrl(ref, kind),
    title,
    state,
    labels: [],
    author: "dezren39",
    updatedAt: "2026-10-24T18:00:00Z",
    ...extra,
  };
}

export const FIXTURE_ITEMS: GhItem[] = [
  gh(`${CODE}#752`, "pr", "PM stack Phase 1 — Kaneo execution board", "merged", {
    updatedAt: "2026-10-24T13:40:00Z",
    reviewDecision: "approved",
  }),
  gh(`${CODE}#754`, "issue", "Kaneo sync check (throwaway)", "closed", {
    updatedAt: "2026-10-24T12:58:00Z",
  }),
  gh(`${CODE}#762`, "issue", "Add the PM-stack design record to doc/README.md's starting points", "open", {
    labels: ["ready-for-agent"],
    updatedAt: "2026-10-24T15:10:00Z",
  }),
  gh(`${CODE}#763`, "pr", "docs: add the PM-stack design record to doc/README.md", "open", {
    reviewDecision: "review_required",
    updatedAt: "2026-10-24T15:32:00Z",
  }),
  // The PM-stack wave (illustrative numbers).
  gh(`${CODE}#900`, "pr", "pm-stack: connect opencode, openchamber, nix as Kaneo projects", "open", {
    draft: true,
    labels: ["status:in-progress"],
  }),
  gh(`${CODE}#901`, "pr", "pm-stack: projector + link registry (Turso)", "open", {
    draft: true,
  }),
  gh(`${CODE}#902`, "pr", "pm-stack: aggregator UI (WIP board + roadmap)", "open", {
    reviewDecision: "review_required",
  }),
  gh(`${OPENCODE}#365`, "issue", "Actions outage: workflows not starting on the fork", "open", {
    labels: ["status:in-progress", "priority:high"],
    updatedAt: "2026-10-24T09:12:00Z",
  }),
  gh(`${OPENCODE}#371`, "pr", "ci: restore Actions on the fork (runner labels)", "open", {
    reviewDecision: "changes_requested",
    updatedAt: "2026-10-24T16:45:00Z",
  }),
  gh(`${OPENCHAMBER}#210`, "pr", "deps: bump embedded OpenCode 2 to the fork build v2.0.30", "merged", {
    updatedAt: "2026-10-23T21:05:00Z",
  }),
  gh(`${OPENCHAMBER}#214`, "issue", "Embedded OpenCode: session list stalls after fork bump", "open", {
    labels: ["bug"],
  }),
  gh(`${NIX}#501`, "issue", "Basement cluster — 3–5 node Talos + Cozystack via TALM", "open", {
    labels: ["cluster", "design"],
  }),
  gh(`${NIX}#502`, "issue", "Hardware inventory + firmware/BMC audit", "open", {
    labels: ["cluster", "status:in-progress"],
  }),
  gh(`${NIX}#507`, "issue", "OCI Always Free — run the technical maximum", "open", {
    labels: ["cloud", "blocked"],
  }),
  gh(`${NIX}#512`, "pr", "hosts: SMART baseline for the boxed HDDs", "open", { draft: true }),
];

const link = (ref: RefKey, relation: Link["relation"] = "tracks"): Link => ({ ref, relation });

export const FIXTURE_BOARD: BoardItem[] = [
  {
    id: "kaneo:code/fx0dispatchtrial0000107",
    title: "Dispatch trial: doc/README.md starting point (#762)",
    repo: CODE,
    source: "kaneo",
    status: "in-review",
    priority: "low",
    updatedAt: "2026-10-24T15:32:00Z",
    kaneoUrl: "https://kaneo.security.cab/",
    links: [link(`${CODE}#762`), link(`${CODE}#763`, "implements")],
  },
  {
    id: "kaneo:code/fx0aggregatorui00000110",
    title: "Aggregator UI — cross-repo WIP board + roadmap view",
    repo: CODE,
    source: "kaneo",
    status: "in-progress",
    priority: "high",
    updatedAt: "2026-10-25T09:00:00Z",
    kaneoUrl: "https://kaneo.security.cab/",
    // The cross-repo case Kaneo cannot express: a code task linking an
    // openchamber PR (the UI's host) alongside its own repo's PRs.
    links: [link(`${CODE}#902`, "implements"), link(`${CODE}#901`, "relates"), link(`${OPENCHAMBER}#210`, "relates")],
    notes: [{ title: "PM-stack design & phase plan", url: "http://127.0.0.1:7777/" }],
  },
  {
    id: "kaneo:code/fx0connectrepos00000111",
    title: "Connect the remaining repos as Kaneo projects",
    repo: CODE,
    source: "kaneo",
    status: "in-progress",
    priority: "medium",
    kaneoUrl: "https://kaneo.security.cab/",
    links: [link(`${CODE}#900`, "implements"), link(`${OPENCODE}#365`, "relates")],
  },
  {
    id: `gh:${OPENCODE}#365`,
    title: "Actions outage on the opencode fork",
    repo: OPENCODE,
    source: "github",
    status: "in-review",
    priority: "high",
    updatedAt: "2026-10-24T16:45:00Z",
    links: [link(`${OPENCODE}#365`), link(`${OPENCODE}#371`, "implements")],
  },
  {
    id: `gh:${OPENCHAMBER}#214`,
    title: "Embedded OpenCode session list stalls",
    repo: OPENCHAMBER,
    source: "github",
    status: "in-progress",
    links: [link(`${OPENCHAMBER}#214`), link(`${OPENCHAMBER}#210`, "relates")],
  },
  {
    id: `gh:${NIX}#502`,
    title: "Hardware inventory + firmware/BMC audit",
    repo: NIX,
    source: "github",
    status: "in-progress",
    links: [link(`${NIX}#502`), link(`${NIX}#512`, "implements")],
  },
  {
    id: "kaneo:code/fx0syncthrowaway0000106",
    title: "Kaneo sync check (throwaway #754)",
    repo: CODE,
    source: "kaneo",
    status: "done",
    updatedAt: "2026-10-24T12:58:00Z",
    kaneoUrl: "https://kaneo.security.cab/",
    links: [link(`${CODE}#754`), link(`${CODE}#752`, "relates")],
  },
];

// Registry links for roadmap rows, on top of any GitHub URL in Source/Notes.
const ROADMAP_LINKS: Record<string, RefKey[]> = {
  "PM stack Phase 2 — ready-for-agent, Pad, roadmap table layer": [`${CODE}#762`, `${CODE}#763`],
  "PM stack — connect opencode, openchamber, nix as Kaneo projects": [`${CODE}#900`],
  "PM stack — projector + link registry (Turso)": [`${CODE}#901`],
  "PM stack — aggregator UI (cross-repo WIP board + roadmap view)": [`${CODE}#902`, `${OPENCHAMBER}#210`],
  "Hardware inventory + firmware/BMC audit": [`${NIX}#512`],
  "Repo fleet — developing-today/opencode": [`${OPENCODE}#365`, `${OPENCODE}#371`],
  "Repo fleet — developing-today/openchamber": [`${OPENCHAMBER}#214`],
};

type SeedRow = { Title: string; Area: string; Status: string; Horizon: string; Source: string; Notes: string };

export const FIXTURE_ROADMAP: RoadmapRow[] = (seed.rows as SeedRow[]).map((r, i) => {
  const refs = new Set([...refsInText(`${r.Source} ${r.Notes}`), ...(ROADMAP_LINKS[r.Title] ?? [])]);
  return {
    id: i + 1,
    subject: `grist:${GRIST_DOC}/Roadmap/${i + 1}`,
    title: r.Title,
    area: r.Area,
    status: r.Status,
    horizon: r.Horizon,
    source: r.Source,
    notes: r.Notes,
    links: [...refs].map((ref) => link(ref)),
  };
});

const c = (
  id: string,
  ref: RefKey,
  kind: Comment["kind"],
  author: string,
  createdAt: string,
  body: string,
  extra: Partial<Comment> = {},
): Comment => ({
  id,
  ref,
  kind,
  author,
  createdAt,
  body,
  url: `${refUrl(ref, kind === "issue_comment" ? "issue" : "pr")}#${id}`,
  ...extra,
});

export const FIXTURE_COMMENTS: Comment[] = [
  c(
    "ic-1",
    `${CODE}#754`,
    "issue_comment",
    "developing-today[bot]",
    "2026-10-24T12:41:00Z",
    "Comment from Kaneo (sync check b).",
  ),
  c("ic-2", `${CODE}#754`, "issue_comment", "dezren39", "2026-10-24T12:44:00Z", "Comment from GitHub (sync check c)."),
  c(
    "rv-1",
    `${CODE}#752`,
    "review",
    "dezren39",
    "2026-10-24T13:35:00Z",
    "Phase 1 as-built reads true against the live stack.",
    { reviewState: "approved" },
  ),
  c(
    "ic-3",
    `${CODE}#762`,
    "issue_comment",
    "ready-for-agent",
    "2026-10-24T15:11:00Z",
    "Work item created; implementing with opencode-go/kimi-k3.1-pro.",
  ),
  c(
    "rc-1",
    `${CODE}#763`,
    "review_comment",
    "dezren39",
    "2026-10-24T15:40:00Z",
    "Exactly the one line — good. Keep the link relative.",
    { path: "doc/README.md", line: 42 },
  ),
  c(
    "rv-2",
    `${CODE}#902`,
    "review",
    "muse",
    "2026-10-25T10:05:00Z",
    "Board renders; checking the roadmap drill-down next.",
    { reviewState: "commented" },
  ),
  c(
    "rc-2",
    `${CODE}#902`,
    "review_comment",
    "muse",
    "2026-10-25T10:07:00Z",
    "Strip order should match the design: implementing → review → merged.",
    { path: "pm-stack/aggregator/src/render/board.ts", line: 30 },
  ),
  c(
    "ic-4",
    `${CODE}#901`,
    "issue_comment",
    "dezren39",
    "2026-10-25T08:30:00Z",
    "Registry keys are repo-qualified refs, per the design record.",
  ),
  c(
    "ic-5",
    `${OPENCHAMBER}#210`,
    "issue_comment",
    "dezren39",
    "2026-10-23T21:00:00Z",
    "Bumped; the aggregator will be opened from here.",
  ),
  c(
    "ic-6",
    `${OPENCODE}#365`,
    "issue_comment",
    "dezren39",
    "2026-10-24T09:12:00Z",
    "Still no runs on main; suspect runner labels after the org move.",
  ),
  c(
    "rv-3",
    `${OPENCODE}#371`,
    "review",
    "muse",
    "2026-10-24T16:45:00Z",
    "The self-hosted label is still wrong on the macOS job.",
    { reviewState: "changes_requested" },
  ),
  c(
    "rc-3",
    `${OPENCODE}#371`,
    "review_comment",
    "muse",
    "2026-10-24T16:46:00Z",
    "`runs-on` should be the fleet label, not `macos-latest`.",
    { path: ".github/workflows/ci.yml", line: 18 },
  ),
  c(
    "ic-7",
    `${NIX}#502`,
    "issue_comment",
    "dezren39",
    "2026-10-22T19:00:00Z",
    "BIOS/BMC identical on the core machines; HDD SMART baseline still owed.",
  ),
  c("ic-8", `${NIX}#507`, "issue_comment", "dezren39", "2026-10-21T14:00:00Z", "Oracle ticket CPCOE-29381 still open."),
];

export function fixtureSource(): Source {
  const items = new Map(FIXTURE_ITEMS.map((i) => [i.ref, i]));
  const provenance: Provenance = {
    board: "fixtures",
    roadmap: "fixtures (Grist seed rows)",
    items: "fixtures",
    comments: "fixtures",
  };
  return {
    board: async () => FIXTURE_BOARD,
    roadmap: async () => FIXTURE_ROADMAP,
    items: async (refs) => new Map(refs.flatMap((r) => (items.has(r) ? [[r, items.get(r) as GhItem]] : []))),
    comments: async (refs) => {
      const wanted = new Set(refs);
      return FIXTURE_COMMENTS.filter((x) => wanted.has(x.ref));
    },
    provenance: () => provenance,
  };
}
