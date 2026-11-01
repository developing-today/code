// The cross-project queue view is a projection of sources the aggregator
// already reads: the board, the Project pool, and the orchestration section.
// It owns no persistence or new feed contract.
import { DONE_BOARD_WINDOW_MS, isRetiredDoneTableTitle } from "./board-policy.ts";
import type { BoardItem, GhItem, OrchestrationSection, ProjectPoolItem, RefKey } from "./model.ts";
import { refUrl } from "./model.ts";

export interface QueueRow {
  id: string;
  title: string;
  ref?: RefKey;
  url?: string;
  repo: string;
  area?: string;
  epic?: { ref: RefKey; title: string; url: string };
  status: string;
  updatedAt?: string;
  ageDays?: number;
  blockedBy?: string[];
  worker?: string;
  horizon?: string;
  why?: string;
  source: string;
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
const PRIORITY_ORDER: Record<string, number> = { urgent: 0, critical: 0, high: 1, normal: 2, medium: 2, low: 3 };

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
    return { ref, title: parent?.title ?? ref, url: parent?.issueUrl ?? parent?.url ?? refUrl(ref) };
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
  }): QueueRow => {
    const projectItem = args.ref ? poolByRef.get(args.ref) : undefined;
    const ghItem = args.ref ? itemByRef.get(args.ref) : undefined;
    const repo = projectItem?.repo ?? (args.ref ? repoForRef(args.ref, args.repo) : args.repo ?? "unknown");
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
        (args.ref ? refUrl(args.ref, ghItem?.kind) : undefined),
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
      });
    }
    return [...byKey.values()];
  };

  const inProgress = unique([
    ...input.board
      .filter((item) => item.status === "in-progress" || item.status === "in-review" ||
        (item.status === "in-flight" && item.orchStatus === "in-progress"))
      .map((item) => row({
        ref: item.links[0]?.ref,
        title: item.title,
        repo: item.repo,
        status: item.orchStatus ?? item.status,
        updatedAt: item.updatedAt,
        blockedBy: item.blockedBy,
        source: item.source,
        id: item.id,
        url: item.kaneoUrl,
      })),
    ...input.pool
      .filter((item) => {
        const labels = [...(item.labels ?? []), ...(itemByRef.get(item.ref)?.labels ?? [])].map(norm);
        return statusMatches(item.status, ["in-progress", "in-review", "in-flight"]) ||
          labels.includes("status/in-progress") || labels.includes("status/in-review");
      })
      .map((item) => row({ ref: item.ref, title: item.title, repo: item.repo, status: item.status ?? "in-progress", updatedAt: item.updatedAt, source: "project-pool", id: item.id })),
  ]);

  const blocked = unique([
    ...input.board
      .filter((item) => item.orchStatus === "blocked" || item.blockedBy?.length || item.status === "blocked")
      .map((item) => row({
        ref: item.links[0]?.ref,
        title: item.title,
        repo: item.repo,
        status: item.orchStatus ?? item.status,
        updatedAt: item.updatedAt ?? input.orchestration?.generatedAt,
        blockedBy: item.blockedBy,
        source: item.source,
        id: item.id,
        url: item.kaneoUrl,
      })),
    ...input.pool
      .filter((item) => item.blockedBy?.length || statusMatches(item.status, ["blocked"]))
      .map((item) => row({
        ref: item.ref,
        title: item.title,
        repo: item.repo,
        status: item.status ?? "blocked",
        updatedAt: item.updatedAt,
        blockedBy: blockersForRefs(item.blockedBy),
        source: "project-pool",
        id: item.id,
      })),
    ...(input.orchestration ? input.orchestration.goals.flatMap((goal) => goal.linedUp
      .filter((task) => task.status === "blocked")
      .map((task) => row({
        ref: task.ref,
        title: task.title,
        repo: repoForRef(task.ref),
        status: "blocked",
        updatedAt: input.orchestration?.generatedAt,
        blockedBy: task.blockedBy?.map((dependency) => `${dependency.task} (${dependency.status})`),
        worker: task.blockedBy?.map((dependency) => dependency.workerLabel).filter(Boolean).join(", ") || undefined,
        source: "orchestration",
      }))) : []),
  ]);

  const doneRecently = unique([
    ...input.board
      .filter((item) => item.status === "done" && !isRetiredDoneTableTitle(item.title) && recent(item.updatedAt, nowMs))
      .map((item) => row({ ref: item.links[0]?.ref, title: item.title, repo: item.repo, status: "done", updatedAt: item.updatedAt, source: item.source, id: item.id, url: item.kaneoUrl })),
    ...input.pool
      .filter((item) => item.contentKind === "pr" && item.state === "merged" && !isRetiredDoneTableTitle(item.title) && recent(item.updatedAt, nowMs))
      .map((item) => row({ ref: item.ref, title: item.title, repo: item.repo, status: "merged", updatedAt: item.updatedAt, source: "project-pool", id: item.id })),
    ...[...itemByRef.values()]
      .filter((item) => item.kind === "pr" && item.state === "merged" && !isRetiredDoneTableTitle(item.title) && recent(item.updatedAt, nowMs))
      .map((item) => row({ ref: item.ref, title: item.title, status: "merged", updatedAt: item.updatedAt, source: "github" })),
  ]);

  const nextCandidates: { row: QueueRow; horizonRank: number; priorityRank: number; sequence: number }[] = [];
  let sequence = 0;
  const addNext = (args: Parameters<typeof row>[0], horizon?: string, priority?: string) => {
    const normalizedHorizon = norm(horizon);
    nextCandidates.push({
      row: row({ ...args, horizon: horizon ?? args.horizon }),
      horizonRank: HORIZON_ORDER[normalizedHorizon] ?? 5,
      priorityRank: PRIORITY_ORDER[norm(priority)] ?? 2,
      sequence: sequence++,
    });
  };
  for (const goal of input.orchestration?.goals ?? []) {
    for (const task of goal.linedUp) {
      if (task.status !== "queued") continue;
      addNext({
        ref: task.ref,
        title: task.title,
        repo: repoForRef(task.ref),
        status: "queued",
        updatedAt: input.orchestration?.generatedAt,
        blockedBy: task.blockedBy?.map((dependency) => `${dependency.task} (${dependency.status})`),
        worker: task.blockedBy?.map((dependency) => dependency.workerLabel).filter(Boolean).join(", ") || undefined,
        why: task.horizonWhy,
        source: "orchestration",
      }, task.horizon);
    }
  }
  for (const item of input.pool) {
    if (!statusMatches(item.status, ["queued", "next", "to-do", "todo"])) continue;
    addNext({
      ref: item.ref,
      title: item.title,
      repo: item.repo,
      status: item.status ?? "queued",
      updatedAt: item.updatedAt,
      blockedBy: blockersForRefs(item.blockedBy),
      why: item.horizon ? `Project horizon: ${item.horizon}` : "Queued in the Project pool",
      source: "project-pool",
      id: item.id,
    }, item.horizon ?? undefined);
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
  const nextUp = unique(nextCandidates
    .sort((a, b) => a.horizonRank - b.horizonRank || a.priorityRank - b.priorityRank || a.sequence - b.sequence)
    .map((candidate) => candidate.row));

  return {
    generatedAt: now.toISOString(),
    inProgress,
    blocked,
    doneRecently: doneRecently.sort((a, b) => (b.updatedAt ?? "").localeCompare(a.updatedAt ?? "")),
    nextUp,
  };
}
