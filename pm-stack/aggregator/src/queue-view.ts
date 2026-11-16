// The cross-project queue view is a projection of sources the aggregator
// already reads: the board, the Project pool, and the orchestration section.
// It owns no persistence or new feed contract.
import { DONE_BOARD_WINDOW_MS, isRetiredDoneTableTitle } from "./board-policy.ts";
import type { BoardItem, GhItem, OrchestrationSection, ProjectPoolItem, RefKey } from "./model.ts";
import { isValidRef, refUrl, refsInText } from "./model.ts";
import { projectDisagreement } from "./projects.ts";
import { FEED_STALE_MS } from "./feed.ts";
import type { HealthView } from "./health.ts";

export interface QueueRow {
  id: string;
  title: string;
  ref?: RefKey;
  url?: string;
  repo: string;
  area?: string;
  epic?: { ref: RefKey; title: string; url?: string };
  status: string;
  updatedAt?: string;
  ageDays?: number;
  blockedBy?: string[];
  worker?: string;
  horizon?: string;
  why?: string;
  source: string;
  /** Which in-progress signals fired for this row (reconciled set only). */
  signals?: InProgressSignal[];
  /** Project Status vs derived state disagreement, when they clash. */
  disagreement?: { project: string; derived: string } | null;
}

export interface QueueView {
  generatedAt: string;
  inProgress: QueueRow[];
  blocked: QueueRow[];
  doneRecently: QueueRow[];
  nextUp: QueueRow[];
}

export interface QueueViewInput {
  board: BoardItem[];
  pool: ProjectPoolItem[];
  items: Map<RefKey, GhItem>;
  orchestration: OrchestrationSection | null;
  now?: Date;
}

/** Whether any source used by the queue projection is stale or unavailable.
 * The projection's generatedAt is intentionally not used: it only records
 * when this process assembled the view, not when its inputs were collected. */
export function queueInputsStale(health: Pick<HealthView, "mode" | "facets" | "feed" | "jobs"> | null): boolean {
  if (!health) return true;
  // Fixture mode is an intentional static source, not a failed live read.
  if (health.mode === "fixtures") return false;

  const facetStale = (key: "board" | "items") => {
    const facet = health.facets?.[key];
    return !facet || facet.state !== "live" || (facet.ageMs !== undefined && facet.ageMs >= FEED_STALE_MS);
  };
  const projects = health.feed.sections.find((section) => section.key === "projects");
  const projectsStale =
    !projects ||
    !projects.present ||
    projects.status === "error" ||
    projects.status === "skipped" ||
    projects.ageMs === undefined ||
    projects.ageMs >= FEED_STALE_MS;

  return facetStale("board") || facetStale("items") || projectsStale || health.jobs.orchestration.state !== "live";
}

const DAY = 86_400_000;
const HORIZON_ORDER: Record<string, number> = {
  today: 0,
  tomorrow: 1,
  "next-3-days": 2,
  "3d": 2,
  week: 3,
  longer: 4,
  month: 4,
  quarter: 4,
  incoming: 5,
};
function norm(value: string | null | undefined): string {
  return (value ?? "").trim().toLowerCase().replace(/[ _]+/g, "-");
}

function recent(updatedAt: string | undefined, now: number): boolean {
  if (!updatedAt) return false;
  const time = Date.parse(updatedAt);
  return Number.isFinite(time) && time <= now && now - time <= DONE_BOARD_WINDOW_MS;
}

function repoForRef(ref: string | undefined, fallback = "unknown"): string {
  const match = ref?.match(/^([^#]+)#\d+$/);
  return match?.[1] ?? fallback;
}

function statusMatches(status: string | null | undefined, choices: string[]): boolean {
  return choices.includes(norm(status));
}

export type InProgressSignal = "project-status" | "active-session" | "open-pr";

export interface ReconciledInProgressItem {
  ref: RefKey;
  title: string;
  repo: string;
  url: string;
  signals: InProgressSignal[];
  disagreement: { project: string; derived: string } | null;
}

export interface ReconciledInProgressInput {
  pool: ProjectPoolItem[];
  board: BoardItem[];
  items?: Map<RefKey, GhItem>;
  orchestration?: OrchestrationSection | null;
}

export function reconciledInProgress(data: ReconciledInProgressInput): ReconciledInProgressItem[] {
  const poolByRef = new Map(data.pool.map((item) => [item.ref, item]));
  const rows = new Map<
    RefKey,
    {
      ref: RefKey;
      title: string;
      repo: string;
      url: string;
      signals: Set<InProgressSignal>;
    }
  >();
  const ensure = (ref: RefKey, title?: string, repo?: string, url?: string) => {
    const project = poolByRef.get(ref);
    const item = data.items?.get(ref);
    const existing = rows.get(ref);
    if (existing) return existing;
    const hashAt = ref.lastIndexOf("#");
    const row = {
      ref,
      title: project?.title ?? item?.title ?? title ?? ref,
      repo: project?.repo ?? (hashAt >= 0 ? ref.slice(0, hashAt) : (repo ?? "")),
      url: project?.issueUrl ?? item?.url ?? url ?? refUrl(ref),
      signals: new Set<InProgressSignal>(),
    };
    rows.set(ref, row);
    return row;
  };
  const signal = (ref: RefKey, kind: InProgressSignal, title?: string, repo?: string, url?: string) => {
    ensure(ref, title, repo, url).signals.add(kind);
  };

  for (const item of data.pool) {
    if (item.status === "In Progress") signal(item.ref, "project-status", item.title, item.repo, item.issueUrl);
    if (item.contentKind === "pr" && item.state === "open")
      signal(item.ref, "open-pr", item.title, item.repo, item.issueUrl);
  }
  for (const goal of data.orchestration?.goals ?? []) {
    for (const task of goal.linedUp) {
      if (task.status !== "in-progress") continue;
      if (task.ref) {
        // #6205: a lined-up ref is feed data, not necessarily a GitHub
        // ref (goal ids, bare numbers). Signaling an invalid ref makes
        // ensure() fall back to refUrl(), whose parseRef throws and
        // kills the whole overview — skip refs the shared rule rejects.
        if (isValidRef(task.ref)) signal(task.ref, "active-session", task.title);
        continue;
      }
      for (const ref of refsInText(task.title)) signal(ref, "active-session", task.title);
      for (const match of task.title.matchAll(/([\w.-]+\/[\w.-]+)#(\d+)/g)) {
        const ref = `${match[1]}#${match[2]}`;
        // #6205: same guard for title-scraped refs — \d+ admits
        // numbers parseRef rejects as unsafe integers.
        if (isValidRef(ref)) signal(ref, "active-session", task.title);
      }
    }
  }
  for (const boardItem of data.board) {
    const openPr = boardItem.links.some((link) => {
      const item = data.items?.get(link.ref);
      return item?.kind === "pr" && item.state === "open";
    });
    for (const link of boardItem.links) {
      const item = data.items?.get(link.ref);
      const title = item?.title ?? boardItem.title;
      const repo = item?.ref.slice(0, link.ref.lastIndexOf("#")) ?? boardItem.repo;
      if (boardItem.source === "orchestrator" && boardItem.orchStatus === "in-progress") {
        signal(link.ref, "active-session", title, repo, item?.url);
      }
      // A board card ties its linked issue and PR to the same work item.
      if (openPr) signal(link.ref, "open-pr", title, repo, item?.url);
    }
  }

  const additionalInProgressRefs = new Set<RefKey>();
  for (const row of rows.values()) {
    if (row.signals.has("active-session") || row.signals.has("open-pr")) additionalInProgressRefs.add(row.ref);
  }
  const signalOrder: InProgressSignal[] = ["project-status", "active-session", "open-pr"];
  return [...rows.values()]
    .map((row) => {
      const project = poolByRef.get(row.ref);
      const disagreement = project ? projectDisagreement(project, data.orchestration, additionalInProgressRefs) : null;
      return {
        ref: row.ref,
        title: row.title,
        repo: row.repo,
        url: row.url,
        signals: signalOrder.filter((item) => row.signals.has(item)),
        disagreement,
      };
    })
    .sort((a, b) => a.repo.localeCompare(b.repo) || a.title.localeCompare(b.title) || a.ref.localeCompare(b.ref));
}

export function buildQueueView(input: QueueViewInput): QueueView {
  const now = input.now ?? new Date();
  const nowMs = now.getTime();
  const poolByRef = new Map(input.pool.map((item) => [item.ref, item]));
  const itemByRef = input.items;

  const blockersForRefs = (refs: string[] | undefined): string[] =>
    (refs ?? []).map((ref) => poolByRef.get(ref)?.title ?? itemByRef.get(ref)?.title ?? ref);
  const epicFor = (poolItem: ProjectPoolItem | undefined) => {
    const ref = poolItem?.epicRef;
    if (!ref) return undefined;
    const parent = poolByRef.get(ref);
    const url = parent?.issueUrl ?? parent?.url ?? (isValidRef(ref) ? refUrl(ref) : undefined);
    return { ref, title: parent?.title ?? ref, ...(url ? { url } : {}) };
  };
  const row = (args: {
    ref?: RefKey;
    title: string;
    repo?: string;
    status: string;
    updatedAt?: string;
    blockedBy?: string[];
    worker?: string;
    horizon?: string;
    why?: string;
    source: string;
    id?: string;
    url?: string;
    signals?: InProgressSignal[];
    disagreement?: { project: string; derived: string } | null;
  }): QueueRow => {
    const projectItem = args.ref ? poolByRef.get(args.ref) : undefined;
    const ghItem = args.ref ? itemByRef.get(args.ref) : undefined;
    const repo = projectItem?.repo ?? (args.ref ? repoForRef(args.ref, args.repo) : (args.repo ?? "unknown"));
    const updatedAt = args.updatedAt ?? projectItem?.updatedAt ?? ghItem?.updatedAt;
    const age = updatedAt ? Date.parse(updatedAt) : NaN;
    const parent = epicFor(projectItem);
    return {
      id: args.id ?? (args.ref ? `gh:${args.ref}` : `${repo}:${args.title}`),
      title: args.title,
      ...(args.ref ? { ref: args.ref } : {}),
      url:
        projectItem?.issueUrl ??
        ghItem?.url ??
        args.url ??
        (args.ref && isValidRef(args.ref) ? refUrl(args.ref, ghItem?.kind) : undefined),
      repo,
      ...(projectItem?.area ? { area: projectItem.area } : {}),
      ...(parent ? { epic: parent } : {}),
      status: args.status,
      ...(updatedAt ? { updatedAt } : {}),
      ...(Number.isFinite(age) ? { ageDays: Math.max(0, Math.floor((nowMs - age) / DAY)) } : {}),
      ...(args.blockedBy?.length ? { blockedBy: [...new Set(args.blockedBy)] } : {}),
      ...(args.worker ? { worker: args.worker } : {}),
      ...(args.horizon ? { horizon: args.horizon } : {}),
      ...(args.why ? { why: args.why } : {}),
      source: args.source,
      ...(args.signals ? { signals: args.signals } : {}),
      ...(args.disagreement !== undefined ? { disagreement: args.disagreement } : {}),
    };
  };

  const unique = (rows: QueueRow[]): QueueRow[] => {
    const byKey = new Map<string, QueueRow>();
    for (const candidate of rows) {
      const key = candidate.ref ? `ref:${candidate.ref}` : `title:${norm(candidate.title)}:${candidate.repo}`;
      const existing = byKey.get(key);
      if (!existing) {
        byKey.set(key, candidate);
        continue;
      }
      byKey.set(key, {
        ...candidate,
        area: candidate.area ?? existing.area,
        epic: candidate.epic ?? existing.epic,
        updatedAt: candidate.updatedAt ?? existing.updatedAt,
        ageDays: candidate.ageDays ?? existing.ageDays,
        url: candidate.url ?? existing.url,
        blockedBy: [...new Set([...(existing.blockedBy ?? []), ...(candidate.blockedBy ?? [])])],
        worker: candidate.worker ?? existing.worker,
        why: candidate.why ?? existing.why,
        signals: candidate.signals ?? existing.signals,
        disagreement: candidate.disagreement !== undefined ? candidate.disagreement : existing.disagreement,
      });
    }
    return [...byKey.values()];
  };

  const inProgress = unique([
    ...input.board
      .filter(
        (item) =>
          item.status === "in-progress" ||
          item.status === "in-review" ||
          (item.status === "in-flight" && item.orchStatus === "in-progress"),
      )
      .map((item) =>
        row({
          ref: item.links[0]?.ref,
          title: item.title,
          repo: item.repo,
          status: item.orchStatus ?? item.status,
          updatedAt: item.updatedAt,
          blockedBy: item.blockedBy,
          source: item.source,
          id: item.id,
          url: item.kaneoUrl,
        }),
      ),
    ...input.pool
      .filter((item) => {
        const labels = [...(item.labels ?? []), ...(itemByRef.get(item.ref)?.labels ?? [])].map(norm);
        return (
          statusMatches(item.status, ["in-progress", "in-review", "in-flight"]) ||
          labels.includes("status/in-progress") ||
          labels.includes("status/in-review")
        );
      })
      .map((item) =>
        row({
          ref: item.ref,
          title: item.title,
          repo: item.repo,
          status: item.status ?? "in-progress",
          updatedAt: item.updatedAt,
          source: "project-pool",
          id: item.id,
        }),
      ),
  ]);

  // #1162 parity: /queue exposes the same reconciled in-progress set as the
  // front-door overview — the union of Project Status=In Progress, active
  // session lined-up work, and open PRs referencing the item — with the
  // per-item signal list and any Project-vs-derived disagreement attached.
  const reconciled = reconciledInProgress({
    pool: input.pool,
    board: input.board,
    items: input.items,
    orchestration: input.orchestration,
  });
  const reconciledByRef = new Map(reconciled.map((item) => [item.ref, item]));
  const inProgressWithSignals = inProgress.map((existing) => {
    const rec = existing.ref ? reconciledByRef.get(existing.ref) : undefined;
    return rec ? { ...existing, signals: rec.signals, disagreement: rec.disagreement } : existing;
  });
  for (const rec of reconciled) {
    if (inProgressWithSignals.some((existing) => existing.ref === rec.ref)) continue;
    inProgressWithSignals.push(
      row({
        ref: rec.ref,
        title: rec.title,
        repo: rec.repo,
        status: poolByRef.get(rec.ref)?.status ?? "in-progress",
        source: "reconciled",
        url: rec.url,
        signals: rec.signals,
        disagreement: rec.disagreement,
      }),
    );
  }

  const blocked = unique([
    ...input.board
      .filter((item) => item.orchStatus === "blocked" || item.blockedBy?.length || item.status === "blocked")
      .map((item) =>
        row({
          ref: item.links[0]?.ref,
          title: item.title,
          repo: item.repo,
          status: item.orchStatus ?? item.status,
          updatedAt: item.updatedAt ?? input.orchestration?.generatedAt,
          blockedBy: item.blockedBy,
          source: item.source,
          id: item.id,
          url: item.kaneoUrl,
        }),
      ),
    ...input.pool
      .filter((item) => item.blockedBy?.length || statusMatches(item.status, ["blocked"]))
      .map((item) =>
        row({
          ref: item.ref,
          title: item.title,
          repo: item.repo,
          status: item.status ?? "blocked",
          updatedAt: item.updatedAt,
          blockedBy: blockersForRefs(item.blockedBy),
          source: "project-pool",
          id: item.id,
        }),
      ),
    ...(input.orchestration
      ? input.orchestration.goals.flatMap((goal) =>
          goal.linedUp
            .filter((task) => task.status === "blocked")
            .map((task) =>
              row({
                ref: task.ref,
                title: task.title,
                repo: repoForRef(task.ref),
                status: "blocked",
                updatedAt: input.orchestration?.generatedAt,
                blockedBy: task.blockedBy?.map((dependency) => `${dependency.task} (${dependency.status})`),
                worker:
                  task.blockedBy
                    ?.map((dependency) => dependency.workerLabel)
                    .filter(Boolean)
                    .join(", ") || undefined,
                source: "orchestration",
              }),
            ),
        )
      : []),
  ]);

  const doneRecently = unique([
    ...input.board
      .filter((item) => item.status === "done" && !isRetiredDoneTableTitle(item.title) && recent(item.updatedAt, nowMs))
      .map((item) =>
        row({
          ref: item.links[0]?.ref,
          title: item.title,
          repo: item.repo,
          status: "done",
          updatedAt: item.updatedAt,
          source: item.source,
          id: item.id,
          url: item.kaneoUrl,
        }),
      ),
    ...input.pool
      .filter(
        (item) =>
          item.contentKind === "pr" &&
          item.state === "merged" &&
          !isRetiredDoneTableTitle(item.title) &&
          recent(item.updatedAt, nowMs),
      )
      .map((item) =>
        row({
          ref: item.ref,
          title: item.title,
          repo: item.repo,
          status: "merged",
          updatedAt: item.updatedAt,
          source: "project-pool",
          id: item.id,
        }),
      ),
    ...[...itemByRef.values()]
      .filter(
        (item) =>
          item.kind === "pr" &&
          item.state === "merged" &&
          !isRetiredDoneTableTitle(item.title) &&
          recent(item.updatedAt, nowMs),
      )
      .map((item) =>
        row({ ref: item.ref, title: item.title, status: "merged", updatedAt: item.updatedAt, source: "github" }),
      ),
  ]);

  const nextCandidates: { row: QueueRow; horizonRank: number; sequence: number }[] = [];
  let sequence = 0;
  const addNext = (args: Parameters<typeof row>[0], horizon?: string) => {
    const normalizedHorizon = norm(horizon);
    nextCandidates.push({
      row: row({ ...args, horizon: horizon ?? args.horizon }),
      horizonRank: HORIZON_ORDER[normalizedHorizon] ?? 5,
      sequence: sequence++,
    });
  };
  for (const goal of input.orchestration?.goals ?? []) {
    for (const task of goal.linedUp) {
      if (task.status !== "queued") continue;
      addNext(
        {
          ref: task.ref,
          title: task.title,
          repo: repoForRef(task.ref),
          status: "queued",
          updatedAt: input.orchestration?.generatedAt,
          blockedBy: task.blockedBy?.map((dependency) => `${dependency.task} (${dependency.status})`),
          worker:
            task.blockedBy
              ?.map((dependency) => dependency.workerLabel)
              .filter(Boolean)
              .join(", ") || undefined,
          why: task.horizonWhy,
          source: "orchestration",
        },
        task.horizon,
      );
    }
  }
  for (const item of input.pool) {
    if (!statusMatches(item.status, ["queued", "next", "to-do", "todo"])) continue;
    addNext(
      {
        ref: item.ref,
        title: item.title,
        repo: item.repo,
        status: item.status ?? "queued",
        updatedAt: item.updatedAt,
        blockedBy: blockersForRefs(item.blockedBy),
        why: item.horizon ? `Project horizon: ${item.horizon}` : "Queued in the Project pool",
        source: "project-pool",
        id: item.id,
      },
      item.horizon ?? undefined,
    );
  }
  for (const item of input.board) {
    if (item.status !== "in-flight" || item.orchStatus !== "queued") continue;
    addNext({
      ref: item.links[0]?.ref,
      title: item.title,
      repo: item.repo,
      status: "queued",
      updatedAt: item.updatedAt,
      blockedBy: item.blockedBy,
      why: item.goal ? `Lined up under ${item.goal}` : "Queued on the board",
      source: item.source,
      id: item.id,
      url: item.kaneoUrl,
    });
  }
  const nextUp = unique(
    nextCandidates
      .sort((a, b) => a.horizonRank - b.horizonRank || a.sequence - b.sequence)
      .map((candidate) => candidate.row),
  );

  return {
    generatedAt: now.toISOString(),
    inProgress: inProgressWithSignals,
    blocked,
    doneRecently: doneRecently.sort((a, b) => (b.updatedAt ?? "").localeCompare(a.updatedAt ?? "")),
    nextUp,
  };
}
