// GitHub Project pool (#910 stage 5): the org Project "Developing
// Today" (developing-today, #4, PVT_kwDOAZNvF84Bouay) is the live
// resolution source for GitHub items. The collector reads it via
// GraphQL (Projects v2) with a Projects-scope token and embeds the
// pool in the fleet feed; the aggregator resolves refs from the pool
// first and renders it on /projects. Read-only everywhere: nothing
// in this codebase writes to the Project.
//
// This module is pure parsing/mapping (no I/O except fetchProjectSnapshot,
// whose fetch is injectable for tests), shared by the collector and
// the live source so the wire shape is parsed exactly once.
import type { BoardSnapshot, BoardEntry } from "./design-boards.ts";
import type { BoardDef } from "./board-registry.ts";
import { PROJECT_BOARDS as REGISTERED_PROJECT_BOARDS, validateBoardRegistry } from "./board-registry.ts";
import type {
  GhItem,
  MilestoneRef,
  OrchestrationSection,
  ProjectAudit,
  ProjectPoolEdgeDiagnostic,
  ProjectPoolItem,
  RefKey,
} from "./model.ts";
import { isValidRef, refKey } from "./model.ts";
import { canonicalStatusFor, type CanonicalStatus } from "./status-taxonomy.ts";
import { isIsoDate, isRecord, isTimestampNotFuture } from "./validate.ts";
import { workItemRefSet } from "./work-items.ts";

export const PROJECT_ID = "PVT_kwDOAZNvF84Bouay";
export const PROJECT_URL = "https://github.com/orgs/developing-today/projects/4";
export const PROJECT_ORG = "developing-today";
export const PROJECT_NUMBER = 4;

/** Registry-backed board metadata; kept here as the shared collector API. */
export { PROJECT_BOARDS } from "./board-registry.ts";

/** GraphQL for one page of project items plus the board's audit schema. */
export const PROJECT_QUERY = `query($id: ID!, $cursor: String) {
  node(id: $id) {
    ... on ProjectV2 {
      title
      number
      url
      fields(first: 100) {
        nodes {
          ... on ProjectV2FieldCommon { id name dataType }
          ... on ProjectV2SingleSelectField { options { id name } }
        }
      }
      views(first: 100) { nodes { id name layout filter } }
      items(first: 100, after: $cursor) {
        totalCount
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          databaseId
          type
          content {
            ... on Issue { number title body state url repository { nameWithOwner } updatedAt closedAt milestone { title dueOn closedAt } subIssuesSummary { total completed } author { login } labels(first: 20) { nodes { name } } parent { number url repository { nameWithOwner } } blockedBy(first: 50) { nodes { number url repository { nameWithOwner } } } blocking(first: 50) { nodes { number url repository { nameWithOwner } } } }
            ... on PullRequest { number title state url repository { nameWithOwner } updatedAt mergedAt closedAt milestone { title dueOn closedAt } author { login } isDraft }
            ... on DraftIssue { title }
          }
          fieldValues(first: 100) {
            nodes {
              ... on ProjectV2ItemFieldSingleSelectValue { name field { ... on ProjectV2FieldCommon { name } } }
              ... on ProjectV2ItemFieldTextValue { text field { ... on ProjectV2FieldCommon { name } } }
              ... on ProjectV2ItemFieldDateValue { date field { ... on ProjectV2FieldCommon { name } } }
            }
          }
        }
      }
    }
  }
}`;

interface GqlFieldValue {
  date?: string;
  name?: string;
  text?: string;
  field?: { name?: string } | null;
}
interface GqlEdgeNode {
  number?: number;
  url?: string;
  repository?: { nameWithOwner?: string } | null;
}
interface GqlContent {
  number?: number;
  title?: string;
  body?: string;
  state?: string;
  url?: string;
  repository?: { nameWithOwner?: string } | null;
  updatedAt?: string;
  closedAt?: string;
  mergedAt?: string;
  author?: { login?: string } | null;
  isDraft?: boolean;
  /** Milestone ingestion (#9067): { title, dueOn, closedAt } or null. */
  milestone?: { title?: string; dueOn?: string | null; closedAt?: string | null } | null;
  subIssuesSummary?: { total?: number; completed?: number } | null;
  labels?: { nodes?: { name?: string }[] } | null;
  /** Sub-issue parent (issues only; null when none). */
  parent?: GqlEdgeNode | null;
  /** "blocked by" / "blocks" dependency edges (issues only). */
  blockedBy?: { nodes?: (GqlEdgeNode | null)[] | null } | null;
  blocking?: { nodes?: (GqlEdgeNode | null)[] | null } | null;
}
interface GqlItem {
  id?: string;
  // ProjectV2Item.databaseId is a nullable Int in the GraphQL schema:
  // null is a valid upstream shape, normalized away in parseProjectItems.
  databaseId?: number | null;
  type?: string;
  content?: GqlContent | null;
  fieldValues?: { nodes?: (GqlFieldValue | null)[] } | null;
}
interface GqlProjectField {
  id?: string;
  name?: string;
  dataType?: string;
  options?: { id?: string; name?: string }[] | null;
}
interface GqlProjectView {
  id?: string;
  name?: string;
  layout?: string;
  filter?: string | null;
}
export interface GqlProjectPage {
  data?: {
    node?: {
      title?: string;
      number?: number;
      url?: string;
      fields?: { nodes?: (GqlProjectField | null)[] } | null;
      views?: { nodes?: (GqlProjectView | null)[] } | null;
      items?: {
        totalCount?: number;
        pageInfo?: { hasNextPage?: boolean; endCursor?: string | null };
        nodes?: (GqlItem | null)[];
      } | null;
    } | null;
  };
  errors?: unknown;
}

/** The canonical Project-item URL: the project board with the item pane open. */
export function projectItemUrl(databaseId: number | undefined, projectUrl = PROJECT_URL): string {
  return databaseId != null ? `${projectUrl}?pane=issue&itemId=${databaseId}` : projectUrl;
}

function singleSelect(item: GqlItem, fieldName: string): string | null {
  for (const v of item.fieldValues?.nodes ?? []) {
    if (v && v.field?.name === fieldName && typeof v.name === "string") return v.name;
  }
  return null;
}

/** One edge endpoint (GraphQL parent/blockedBy/blocking node) as a
 * canonical ref; incomplete or malformed endpoints are diagnosed and dropped. */
function edgeRef(
  n: GqlEdgeNode | null | undefined,
  relation: ProjectPoolEdgeDiagnostic["relation"],
  diagnostics: ProjectPoolEdgeDiagnostic[],
): RefKey | null {
  if (typeof n?.number !== "number" || !n.repository?.nameWithOwner) {
    diagnostics.push({ relation, state: "unavailable", target: null });
    return null;
  }
  const ref = refKey({ repo: n.repository.nameWithOwner, number: n.number });
  if (!isValidRef(ref)) {
    diagnostics.push({ relation, state: "malformed", target: ref });
    return null;
  }
  return ref;
}

function edgeNodes(
  connection: { nodes?: (GqlEdgeNode | null)[] | null } | null | undefined,
  relation: "blockedBy" | "blocks",
  diagnostics: ProjectPoolEdgeDiagnostic[],
): (GqlEdgeNode | null)[] {
  if (connection === undefined || connection === null || connection.nodes == null) {
    diagnostics.push({ relation, state: "unavailable", target: null });
    return [];
  }
  return connection.nodes;
}

/** Parse one GraphQL page (or a full response) into pool items.
 * Draft items / items without a linked issue/PR are skipped: they
 * have no ref the dashboard can resolve or link to. Edge fields
 * (parent/blockedBy/blocks) map only for issues — PRs and drafts
 * never carry them. Explicit empty connections mean no edge; omitted
 * queried edge fields are retained as unavailable diagnostics.
 * Epic membership (epicRef) is NOT resolved here:
 * it needs the whole pool (parents may sit on other pages), so
 * resolveEpicRefs runs over the aggregate after all pages parse. */
export function parseProjectItems(json: unknown, projectUrl = PROJECT_URL, boardId?: string): ProjectPoolItem[] {
  const nodes = (json as GqlProjectPage)?.data?.node?.items?.nodes ?? [];
  const out: ProjectPoolItem[] = [];
  for (const item of nodes) {
    if (!item?.id || !item.content) continue;
    const c = item.content;
    if (typeof c.number !== "number" || !c.repository?.nameWithOwner || !c.url) continue;
    const repo = c.repository.nameWithOwner;
    const isPr = item.type === "PULL_REQUEST" || /\/pull\//.test(c.url);
    const rawState = (c.state ?? "OPEN").toUpperCase();
    const edgeDiagnostics: ProjectPoolEdgeDiagnostic[] = [];
    let parentRef: RefKey | null = null;
    if (!isPr) {
      if (c.parent === undefined) edgeDiagnostics.push({ relation: "parent", state: "unavailable", target: null });
      else if (c.parent) parentRef = edgeRef(c.parent, "parent", edgeDiagnostics);
    }
    const blockedBy = !isPr
      ? edgeNodes(c.blockedBy, "blockedBy", edgeDiagnostics)
          .map((n) => edgeRef(n, "blockedBy", edgeDiagnostics))
          .filter((x): x is RefKey => x !== null)
      : [];
    const historicalParentRefs = !isPr && typeof c.body === "string" ? historicalParentsFromMetadata(c.body) : [];
    const blocks = !isPr
      ? edgeNodes(c.blocking, "blocks", edgeDiagnostics)
          .map((n) => edgeRef(n, "blocks", edgeDiagnostics))
          .filter((x): x is RefKey => x !== null)
      : [];
    out.push({
      id: item.id,
      ...(boardId ? { boardId } : {}),
      // Normalize the nullable GraphQL databaseId to the model's optional
      // number: null becomes absent, so a valid item is never dropped by
      // isProjectPoolItem for an upstream null (Codex P2 on PR #4181).
      databaseId: item.databaseId ?? undefined,
      url: projectItemUrl(item.databaseId ?? undefined, projectUrl),
      projectUrl,
      ref: refKey({ repo, number: c.number }),
      repo,
      number: c.number,
      issueUrl: c.url,
      title: c.title ?? "",
      state: rawState === "MERGED" ? "merged" : rawState === "OPEN" ? "open" : "closed",
      contentKind: isPr ? "pr" : "issue",
      draft: isPr ? Boolean(c.isDraft) : undefined,
      labels: (c.labels?.nodes ?? []).map((l) => l?.name).filter((x): x is string => typeof x === "string"),
      author: c.author?.login ?? "",
      updatedAt: c.updatedAt ?? "",
      ...(c.closedAt ? { closedAt: c.closedAt } : {}),
      ...(c.mergedAt ? { mergedAt: c.mergedAt } : {}),
      ...(c.milestone && typeof c.milestone.title === "string"
        ? {
            milestone: {
              title: c.milestone.title,
              dueOn: c.milestone.dueOn ?? null,
              closedAt: c.milestone.closedAt ?? null,
            } satisfies MilestoneRef,
          }
        : {}),
      ...(c.subIssuesSummary &&
      Number.isSafeInteger(c.subIssuesSummary.total) &&
      Number.isSafeInteger(c.subIssuesSummary.completed)
        ? {
            subIssuesSummary: {
              total: c.subIssuesSummary.total!,
              completed: c.subIssuesSummary.completed!,
            },
          }
        : {}),
      status: singleSelect(item, "Status"),
      area: singleSelect(item, "Area"),
      horizon: singleSelect(item, "Horizon"),
      kind: singleSelect(item, "Kind"),
      ...(parentRef ? { parentRef } : {}),
      ...(blockedBy.length ? { blockedBy } : {}),
      ...(blocks.length ? { blocks } : {}),
      ...(historicalParentRefs.length ? { historicalParentRefs } : {}),
      ...(edgeDiagnostics.length ? { edgeDiagnostics } : {}),
    });
  }
  return out;
}

/** Read only explicitly historical parent links from the canonical PM metadata block. */
export function historicalParentsFromMetadata(body: string): RefKey[] {
  const block = /<!--\s*pm-metadata:start\s*-->([\s\S]*?)<!--\s*pm-metadata:end\s*-->/.exec(body)?.[1];
  if (!block) return [];
  const json = /```json\s*([\s\S]*?)```/i.exec(block)?.[1];
  if (!json) return [];
  try {
    const metadata: unknown = JSON.parse(json);
    if (!isRecord(metadata) || !Array.isArray(metadata.links)) return [];
    return [
      ...new Set(
        metadata.links.flatMap((link) => {
          if (!isRecord(link) || link.relation !== "parent" || link.status !== "historical" || !isRecord(link.target))
            return [];
          const target = link.target;
          if (typeof target.repo !== "string" || !Number.isSafeInteger(target.number) || (target.number as number) <= 0)
            return [];
          const ref = refKey({ repo: target.repo, number: target.number as number });
          return isValidRef(ref) ? [ref] : [];
        }),
      ),
    ];
  } catch {
    return [];
  }
}

/** Resolve epic membership over a fully parsed pool (#2399): every
 * non-epic item whose sub-issue parent chain (within this pool) reaches
 * a Kind="Epic" item gains epicRef naming that epic. Epics themselves,
 * items with no epic ancestor, and chains that leave the pool keep the
 * field absent. Cycle-safe: a parent loop ends the walk with no epic. */
export function resolveEpicRefs(items: ProjectPoolItem[]): ProjectPoolItem[] {
  // An item appearing on several boards is one epic if any board classifies it as Epic.
  const epicRefs = new Set(items.filter((p) => p.kind === "Epic").map((p) => p.ref));
  const byRef = new Map<string, ProjectPoolItem>();
  // Conflicting copies can have different parents; choose deterministically.
  for (const p of [...items].sort(
    (a, b) => a.ref.localeCompare(b.ref) || (a.parentRef ?? "").localeCompare(b.parentRef ?? ""),
  )) {
    if (!byRef.has(p.ref)) byRef.set(p.ref, p);
  }
  return items.map((p) => {
    const { epicRef: _staleEpicRef, ...withoutEpicRef } = p;
    if (epicRefs.has(p.ref) || !p.parentRef) return withoutEpicRef;
    const seen = new Set<string>([p.ref]);
    let cursor: ProjectPoolItem | undefined = byRef.get(p.parentRef);
    while (cursor && !seen.has(cursor.ref)) {
      if (epicRefs.has(cursor.ref)) return { ...withoutEpicRef, epicRef: cursor.ref };
      seen.add(cursor.ref);
      cursor = cursor.parentRef ? byRef.get(cursor.parentRef) : undefined;
    }
    return withoutEpicRef;
  });
}

/** Add a spawn rollup to design issues from direct native children. The
 * GitHub summary is authoritative for total/done counts (including children
 * outside this Project); loaded Project children supply Status for the
 * in-progress count. Blocking is the intersection of the parent's
 * blockedBy edges and its direct spawned children. Missing summaries stay
 * absent rather than inventing a zero. */
export function resolveSpawnRollups(items: ProjectPoolItem[]): ProjectPoolItem[] {
  const childrenByParent = new Map<string, ProjectPoolItem[]>();
  for (const item of items) {
    if (!item.parentRef) continue;
    const children = childrenByParent.get(item.parentRef) ?? [];
    children.push(item);
    childrenByParent.set(item.parentRef, children);
  }
  return items.map((item) => {
    const isDesign = item.labels?.some((label) => label === "design" || label.startsWith("design/"));
    const summary = item.subIssuesSummary;
    if (!isDesign || !summary) {
      const { spawnRollup: _stale, ...unchanged } = item;
      return unchanged;
    }
    const children = childrenByParent.get(item.ref) ?? [];
    const directChildRefs = new Set(children.map((child) => child.ref));
    return {
      ...item,
      spawnRollup: {
        total: summary.total,
        done: summary.completed,
        inProgress: children.filter((child) => child.status?.toLowerCase() === "in progress").length,
        blocking: (item.blockedBy ?? []).filter((ref) => directChildRefs.has(ref)).sort(),
      },
    };
  });
}

export function projectPageInfo(json: unknown): { hasNextPage: boolean; endCursor: string | null; totalCount: number } {
  const items = (json as GqlProjectPage)?.data?.node?.items;
  return {
    hasNextPage: Boolean(items?.pageInfo?.hasNextPage),
    endCursor: items?.pageInfo?.endCursor ?? null,
    totalCount: items?.totalCount ?? 0,
  };
}

function fieldValue(item: GqlItem, name: string): string {
  const value = (item.fieldValues?.nodes ?? []).find((v) => v?.field?.name === name);
  const label = typeof value?.name === "string" ? value.name : value?.text;
  return typeof label === "string" && label.length > 0 ? label : "(unset)";
}

function distribution(items: GqlItem[], value: (item: GqlItem) => string): Record<string, number> {
  const counts = new Map<string, number>();
  for (const item of items) {
    const key = value(item);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return Object.fromEntries([...counts.entries()].sort(([a], [b]) => a.localeCompare(b)));
}

/** Build the public audit snapshot from the first page's board schema and every item node. */
export function parseProjectAudit(json: unknown, items: GqlItem[], generatedAt: string): ProjectAudit {
  const node = (json as GqlProjectPage)?.data?.node;
  const itemConnection = node?.items;
  const totalItems = itemConnection?.totalCount;
  if (!node || !itemConnection || !node.fields || !node.views || !Number.isInteger(totalItems) || totalItems! < 0) {
    throw new Error("github GraphQL response omitted Project metadata, fields, views, or item totalCount");
  }
  const fields = (node.fields?.nodes ?? []).flatMap((field) => {
    if (!field?.id || !field.name || !field.dataType) return [];
    const options = (field.options ?? []).flatMap((option) =>
      option?.id && option.name ? [{ id: option.id, name: option.name }] : [],
    );
    return [{ id: field.id, name: field.name, dataType: field.dataType, ...(options.length ? { options } : {}) }];
  });
  const views = (node.views?.nodes ?? []).flatMap((view) =>
    view?.id && view.name && view.layout
      ? [{ id: view.id, name: view.name, layout: view.layout, filter: view.filter ?? null }]
      : [],
  );
  return {
    title: node.title ?? "Developing Today",
    number: node.number ?? PROJECT_NUMBER,
    url: node.url ?? PROJECT_URL,
    generatedAt,
    totalItems: totalItems as number,
    fields,
    views,
    counts: {
      status: distribution(items, (item) => fieldValue(item, "Status")),
      area: distribution(items, (item) => fieldValue(item, "Area")),
      repo: distribution(items, (item) => item.content?.repository?.nameWithOwner ?? "(unlinked)"),
    },
  };
}

/** Retain draft cards and dates for the board family; never carry tokens/raw responses. */
function parseBoardEntries(items: GqlItem[], projectUrl: string): BoardEntry[] {
  const date = (item: GqlItem, names: string[]): string | null => {
    const value = item.fieldValues?.nodes?.find((v) => v && names.includes(v.field?.name ?? ""))?.date;
    return typeof value === "string" && /^\d{4}-\d{2}-\d{2}$/.test(value) && isIsoDate(value) ? value : null;
  };
  return items.flatMap((item) => {
    if (!item.id || !item.content || typeof item.content.title !== "string") return [];
    return [
      {
        id: item.id,
        title: item.content.title,
        url: item.content.url ?? projectItemUrl(item.databaseId ?? undefined, projectUrl),
        status: singleSelect(item, "Status"),
        area: singleSelect(item, "Area"),
        theme: singleSelect(item, "Theme"),
        repo: item.content.repository?.nameWithOwner ?? null,
        milestone: item.content.milestone?.title ?? null,
        labels: (item.content.labels?.nodes ?? []).flatMap((l) => (typeof l?.name === "string" ? [l.name] : [])),
        start: date(item, ["Start date", "Start", "Start Date"]),
        target: date(item, ["Target date", "Target", "Target Date"]),
      },
    ];
  });
}

/** Runtime gate for audit snapshots read from the collector-owned feed. */
export function isProjectAudit(value: unknown): value is ProjectAudit {
  if (!isRecord(value) || !isRecord(value.counts)) return false;
  const validDistribution = (v: unknown) =>
    isRecord(v) && Object.values(v).every((n) => Number.isInteger(n) && (n as number) >= 0);
  const distributionTotal = (v: unknown): number | null => {
    if (!isRecord(v) || !Object.values(v).every((n) => Number.isInteger(n) && (n as number) >= 0)) return null;
    return Object.values(v).reduce<number>((sum, n) => sum + (n as number), 0);
  };
  return (
    typeof value.title === "string" &&
    Number.isInteger(value.number) &&
    typeof value.url === "string" &&
    isTimestampNotFuture(value.generatedAt) &&
    Number.isInteger(value.totalItems) &&
    (value.totalItems as number) >= 0 &&
    (value.poolTotalItems === undefined ||
      (Number.isInteger(value.poolTotalItems) && (value.poolTotalItems as number) >= (value.totalItems as number))) &&
    Array.isArray(value.fields) &&
    value.fields.every(
      (field) =>
        isRecord(field) &&
        typeof field.id === "string" &&
        typeof field.name === "string" &&
        typeof field.dataType === "string" &&
        (field.options === undefined ||
          (Array.isArray(field.options) &&
            field.options.every(
              (option) => isRecord(option) && typeof option.id === "string" && typeof option.name === "string",
            ))),
    ) &&
    Array.isArray(value.views) &&
    value.views.every(
      (view) =>
        isRecord(view) &&
        typeof view.id === "string" &&
        typeof view.name === "string" &&
        typeof view.layout === "string" &&
        (typeof view.filter === "string" || view.filter === null),
    ) &&
    validDistribution(value.counts.status) &&
    validDistribution(value.counts.area) &&
    validDistribution(value.counts.repo) &&
    distributionTotal(value.counts.status) === value.totalItems &&
    distributionTotal(value.counts.area) === value.totalItems &&
    distributionTotal(value.counts.repo) === value.totalItems
  );
}

export interface ProjectSnapshot {
  /** All board cards, including draft ideas, safe for cached rendering. */
  board: BoardSnapshot;
  items: ProjectPoolItem[];
  audit: ProjectAudit;
  /** Section counts for feed health (#5180): the provider total made
   * comparable to the emitted count. totalCount counts every board
   * item, while `items` excludes drafts and unlinked items, so the
   * exclusions (raw nodes minus emitted items) come off the total —
   * comparing the raw total against the emitted count reported
   * positive lag on a complete collection whenever the board held a
   * draft (Codex P2 on PR #5180). */
  counts: { reportedTotal: number; emittedEntries: number };
}

/** Narrow fetch shape the pool readers need; fleet gh's routed adapter satisfies it without Bun's preconnect. */
export type ProjectFetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>;

/** Fetch the whole pool, board schema, views, and distributions (paginated). */
export async function fetchProjectSnapshot(
  token: string,
  fetchFn: ProjectFetch = fetch,
  generatedAt = new Date().toISOString(),
  projectId = PROJECT_ID,
  boardId?: string,
): Promise<ProjectSnapshot> {
  const parsedItems: ProjectPoolItem[] = [];
  const auditItems: GqlItem[] = [];
  const seenItemIds = new Set<string>();
  let firstPage: unknown;
  let cursor: string | null = null;
  let lastPageInfo: ReturnType<typeof projectPageInfo> | null = null;
  // Paginate to exhaustion (Drew, 2026-11-12: scan them all, or paginate —
  // no page-count cap; the old 10-page fetch cap discarded the whole pool
  // once Project #4 passed 1,000 items). The did-not-advance guard below
  // is the loop-safety bound.
  for (let page = 0; ; page++) {
    const res = await fetchFn("https://api.github.com/graphql", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
        "User-Agent": "pm-fleet-feed-collector",
      },
      body: JSON.stringify({ query: PROJECT_QUERY, variables: { id: projectId, cursor } }),
      signal: AbortSignal.timeout(20_000),
    });
    if (!res.ok) throw new Error(`github GraphQL HTTP ${res.status}`);
    const json: unknown = await res.json();
    if ((json as GqlProjectPage).errors) throw new Error("github GraphQL returned errors");
    if (!firstPage) firstPage = json;
    const nodes = (json as GqlProjectPage).data?.node?.items?.nodes ?? [];
    // #8599: overlapping pages (the board mutating mid-export) can
    // deliver the same node twice. Deduplicate by item id at
    // collection, first delivery wins: the audit distributions and
    // the completeness check below are judged on the deduplicated
    // export, so an over-delivered export still sums to totalItems
    // and survives the feed-side isProjectAudit gate instead of
    // being discarded as an absent snapshot. Fetch-side tolerance
    // and feed-side strictness agree on the distinct-item count.
    const freshNodes = nodes.filter((node): node is GqlItem => {
      if (node === null) return false;
      if (!node.id) return true;
      if (seenItemIds.has(node.id)) return false;
      seenItemIds.add(node.id);
      return true;
    });
    auditItems.push(...freshNodes);
    const freshIds = new Set(freshNodes.map((node) => node.id));
    parsedItems.push(
      ...parseProjectItems(json, (json as GqlProjectPage).data?.node?.url ?? PROJECT_URL, boardId).filter((item) =>
        freshIds.has(item.id),
      ),
    );
    const info = projectPageInfo(json);
    if (!info.hasNextPage) {
      if (auditItems.length < info.totalCount) {
        // Shortfall only, judged on the deduplicated count: fewer
        // distinct items than the board total means the export lost
        // items. Over-delivery is absorbed by the dedupe above.
        throw new Error(`github GraphQL project export incomplete (${auditItems.length}/${info.totalCount} items)`);
      }
      const audit = parseProjectAudit(firstPage, auditItems, generatedAt);
      return {
        board: {
          number: audit.number,
          url: audit.url,
          generatedAt,
          totalItems: audit.totalItems,
          views: audit.views,
          items: parseBoardEntries(auditItems, audit.url),
        },
        items: resolveSpawnRollups(resolveEpicRefs(parsedItems)),
        audit,
        counts: {
          reportedTotal: info.totalCount - (auditItems.length - parsedItems.length),
          emittedEntries: parsedItems.length,
        },
      };
    }
    if (!info.endCursor || info.endCursor === cursor)
      throw new Error("github GraphQL project pagination did not advance");
    cursor = info.endCursor;
  }
  throw new Error("github GraphQL project pagination ended unexpectedly");
}

export interface ProjectBoardsSnapshot {
  items: ProjectPoolItem[];
  audits: Record<string, ProjectAudit>;
  boards: BoardSnapshot[];
  failedBoardIds: string[];
  counts: { reportedTotal: number; emittedEntries: number };
}

/** Collect the configured board family independently; a broken satellite never discards a healthy board. */
export async function fetchProjectBoardsSnapshot(
  token: string,
  fetchFn: ProjectFetch,
  generatedAt = new Date().toISOString(),
  boards: readonly BoardDef[] = REGISTERED_PROJECT_BOARDS,
): Promise<ProjectBoardsSnapshot> {
  const problems = validateBoardRegistry(boards);
  if (problems.length) throw new Error(`invalid project board registry: ${problems.join("; ")}`);
  const items: ProjectPoolItem[] = [];
  const audits: Record<string, ProjectAudit> = {};
  const snapshots: BoardSnapshot[] = [];
  const failedBoardIds: string[] = [];
  let reportedTotal = 0;
  let emittedEntries = 0;
  for (const board of boards) {
    try {
      const snapshot = await fetchProjectSnapshot(token, fetchFn, generatedAt, board.nodeId, board.id);
      if (!snapshot.items.length && board.role === "overall") throw new Error("board returned zero resolvable items");
      items.push(...snapshot.items);
      audits[board.id] = snapshot.audit;
      snapshots.push(snapshot.board);
      reportedTotal += snapshot.counts.reportedTotal;
      emittedEntries += snapshot.counts.emittedEntries;
    } catch {
      failedBoardIds.push(board.id);
    }
  }
  return {
    items: resolveSpawnRollups(resolveEpicRefs(items)),
    audits,
    boards: snapshots,
    failedBoardIds,
    counts: { reportedTotal, emittedEntries },
  };
}

/** Fetch the whole pool (paginated). Throws on HTTP/GraphQL errors so
 * the collector can fail-soft and omit the section. Never logs the token.
 * Pool-only path: it must NOT route through fetchProjectSnapshot — the
 * audit parser requires board fields/views metadata that pool-only
 * responses (e.g. the contract corpus) do not carry, and a missing audit
 * must never cost the pool. The collector uses fetchProjectSnapshot
 * for the audited section instead. */
export async function fetchProjectPool(token: string, fetchFn: ProjectFetch = fetch): Promise<ProjectPoolItem[]> {
  const out: ProjectPoolItem[] = [];
  let cursor: string | null = null;
  let lastPageInfo: ReturnType<typeof projectPageInfo> | null = null;
  // Paginate to exhaustion, same as fetchProjectSnapshot (no page cap).
  for (let page = 0; ; page++) {
    const res = await fetchFn("https://api.github.com/graphql", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
        "User-Agent": "pm-fleet-feed-collector",
      },
      body: JSON.stringify({ query: PROJECT_QUERY, variables: { id: PROJECT_ID, cursor } }),
      signal: AbortSignal.timeout(20_000),
    });
    if (!res.ok) throw new Error(`github GraphQL HTTP ${res.status}`);
    const json: unknown = await res.json();
    if ((json as GqlProjectPage).errors) throw new Error("github GraphQL returned errors");
    out.push(...parseProjectItems(json));
    lastPageInfo = projectPageInfo(json);
    if (!lastPageInfo.hasNextPage) break;
    if (!lastPageInfo.endCursor || lastPageInfo.endCursor === cursor)
      throw new Error("github GraphQL project pagination did not advance");
    cursor = lastPageInfo.endCursor;
  }
  // Epic membership needs the whole pool (parents may sit on other
  // pages), so it resolves once over the aggregate, not per page.
  return resolveSpawnRollups(resolveEpicRefs(out));
}

/** Structural check for a pool entry in a feed (collector + live share
 * it, and the feed.ts section gate delegates to it — the two sides can
 * never drift). Edge fields (#2399) are absence-friendly, but every
 * present ref must satisfy the shared isValidRef rule: a malformed edge
 * fails the item, never the section. */
export function isProjectPoolItem(v: unknown): v is ProjectPoolItem {
  if (typeof v !== "object" || v === null) return false;
  const p = v as Partial<ProjectPoolItem>;
  return (
    typeof p.id === "string" &&
    typeof p.ref === "string" &&
    typeof p.repo === "string" &&
    typeof p.number === "number" &&
    Number.isInteger(p.number) &&
    p.number > 0 &&
    typeof p.issueUrl === "string" &&
    typeof p.title === "string" &&
    // url/projectUrl/contentKind and the Project single-select fields are
    // required by the model and dereferenced by board/item renders; the
    // gate must own their shapes (#2418).
    typeof p.url === "string" &&
    typeof p.projectUrl === "string" &&
    (p.contentKind === "issue" || p.contentKind === "pr") &&
    (p.status === null || typeof p.status === "string") &&
    (p.area === null || typeof p.area === "string") &&
    (p.horizon === null || typeof p.horizon === "string") &&
    (p.kind === null || typeof p.kind === "string") &&
    // Optional fields stay absence-friendly, but a present value must
    // match its declared type: labels feeds .map/.includes in renders,
    // so a string or non-string array crashes instead of dropping (#2418).
    (p.databaseId === undefined || typeof p.databaseId === "number") &&
    (p.draft === undefined || typeof p.draft === "boolean") &&
    (p.labels === undefined || (Array.isArray(p.labels) && p.labels.every((x) => typeof x === "string"))) &&
    (p.author === undefined || typeof p.author === "string") &&
    // updatedAt feeds derive.ts momentum/recency math, which Date.parse's
    // it: a bare string check lets impossible dates roll into fabricated
    // activity (#5202). A present non-empty stamp must be a real ISO date;
    // '' is the collector's missing-stamp form and counts as absent.
    (p.updatedAt === undefined || p.updatedAt === "" || isIsoDate(p.updatedAt)) &&
    (p.closedAt === undefined || typeof p.closedAt === "string") &&
    (p.mergedAt === undefined || typeof p.mergedAt === "string") &&
    (p.milestone === undefined ||
      (typeof p.milestone === "object" &&
        p.milestone !== null &&
        typeof p.milestone.title === "string" &&
        (p.milestone.dueOn === null || typeof p.milestone.dueOn === "string") &&
        (p.milestone.closedAt === null || typeof p.milestone.closedAt === "string"))) &&
    (p.subIssuesSummary === undefined ||
      (typeof p.subIssuesSummary === "object" &&
        p.subIssuesSummary !== null &&
        Number.isSafeInteger(p.subIssuesSummary.total) &&
        (p.subIssuesSummary.total as number) >= 0 &&
        Number.isSafeInteger(p.subIssuesSummary.completed) &&
        (p.subIssuesSummary.completed as number) >= 0 &&
        (p.subIssuesSummary.completed as number) <= (p.subIssuesSummary.total as number))) &&
    (p.spawnRollup === undefined ||
      (typeof p.spawnRollup === "object" &&
        p.spawnRollup !== null &&
        Number.isSafeInteger(p.spawnRollup.total) &&
        Number.isSafeInteger(p.spawnRollup.done) &&
        Number.isSafeInteger(p.spawnRollup.inProgress) &&
        Array.isArray(p.spawnRollup.blocking) &&
        p.spawnRollup.blocking.every(isValidRef))) &&
    (p.state === "open" || p.state === "closed" || p.state === "merged") &&
    (p.parentRef === undefined || isValidRef(p.parentRef)) &&
    (p.epicRef === undefined || isValidRef(p.epicRef)) &&
    (p.historicalParentRefs === undefined ||
      (Array.isArray(p.historicalParentRefs) && p.historicalParentRefs.every(isValidRef))) &&
    (p.blockedBy === undefined || (Array.isArray(p.blockedBy) && p.blockedBy.every(isValidRef))) &&
    (p.blocks === undefined || (Array.isArray(p.blocks) && p.blocks.every(isValidRef))) &&
    (p.edgeDiagnostics === undefined ||
      (Array.isArray(p.edgeDiagnostics) &&
        p.edgeDiagnostics.every(
          (d) =>
            d !== null &&
            typeof d === "object" &&
            ["parent", "blockedBy", "blocks"].includes(d.relation) &&
            ["unavailable", "malformed"].includes(d.state) &&
            (d.target === null || typeof d.target === "string"),
        )))
  );
}

/** Build the dashboard's GhItem for one pool entry. */
export function poolItemToGhItem(p: ProjectPoolItem): GhItem {
  return {
    ref: p.ref,
    kind: p.contentKind,
    url: p.issueUrl,
    title: p.title,
    state: p.state,
    draft: p.draft,
    labels: p.labels ?? [],
    author: p.author ?? "",
    updatedAt: p.updatedAt ?? "",
    projectStatus: p.status,
    projectArea: p.area,
    projectHorizon: p.horizon,
    projectKind: p.kind,
  };
}

/** Keep the first board membership for each canonical issue ref. Use only
 * where an issue-level calculation must not count cross-board copies; the
 * pool itself retains every membership for board attribution and rendering. */
/** The derived state IS a canonical status (#1198): the derived side
 * and every source side speak the one taxonomy in status-taxonomy.ts. */
export type DerivedProjectState = CanonicalStatus;

/**
 * Derived state — what the rest of the dashboard derives for an item,
 * in the canonical taxonomy (#1198; documented per #910 stage 5):
 *   - issue/PR closed or merged            -> Done
 *   - open, ready (non-draft) PR            -> In Review
 *     (GitHub's draft/ready distinction is the review boundary: a
 *     ready PR is under review by construction, so a Project Status
 *     of "In Review" AGREES for ready PRs instead of disagreeing by
 *     construction, as it did before #1198)
 *   - open, and its ref appears anywhere in the orchestration
 *     section (a tier item, a goal's lined-up task, or a dependency),
 *     or the item is an open draft PR        -> In Progress
 *   - otherwise                              -> Todo
 */
export function derivedProjectState(
  p: ProjectPoolItem,
  section?: OrchestrationSection | null,
  additionalInProgressRefs?: ReadonlySet<RefKey>,
): DerivedProjectState {
  if (p.state === "closed" || p.state === "merged") return "Done";
  if (p.contentKind === "pr" && p.state === "open" && !p.draft) return "In Review";
  if ((section && refInOrchestration(p.ref, section)) || additionalInProgressRefs?.has(p.ref)) return "In Progress";
  if (p.contentKind === "pr" && p.state === "open") return "In Progress";
  return "Todo";
}

function refInOrchestration(ref: RefKey, section: OrchestrationSection): boolean {
  // #1206: the canonical projection's ref set — one assembly answers
  // "does this ref appear in the section" for every page, instead of
  // this module rescanning tiers, goals, and dependencies itself.
  return workItemRefSet(section).has(ref);
}

/** The disagreement to display, or null when both sides agree (or the
 * Project carries no Status to disagree with). The comparison is
 * canonical (#1198): the Project Status is mapped through the
 * taxonomy's project table first, so a spelling variant of the same
 * status never fakes a disagreement — and an UNMAPPED Project Status
 * (canonical null) always disagrees, because an unknown vocabulary
 * silently treated as agreement is how the pages drifted apart. */
export function projectDisagreement(
  p: ProjectPoolItem,
  section?: OrchestrationSection | null,
  additionalInProgressRefs?: ReadonlySet<RefKey>,
): { project: string; derived: DerivedProjectState } | null {
  if (!p.status) return null;
  const derived = derivedProjectState(p, section, additionalInProgressRefs);
  return canonicalStatusFor("project", p.status) === derived ? null : { project: p.status, derived };
}
