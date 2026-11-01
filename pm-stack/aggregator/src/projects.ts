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
import type { GhItem, OrchestrationSection, ProjectAudit, ProjectPoolItem, RefKey } from "./model.ts";
import { refKey } from "./model.ts";
import { isRecord, isTimestampNotFuture } from "./validate.ts";

export const PROJECT_ID = "PVT_kwDOAZNvF84Bouay";
export const PROJECT_URL = "https://github.com/orgs/developing-today/projects/4";
export const PROJECT_ORG = "developing-today";
export const PROJECT_NUMBER = 4;

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
            ... on Issue { number title state url repository { nameWithOwner } updatedAt author { login } labels(first: 20) { nodes { name } } parent { number url repository { nameWithOwner } } blockedBy(first: 50) { nodes { number url repository { nameWithOwner } } } blocking(first: 50) { nodes { number url repository { nameWithOwner } } } }
            ... on PullRequest { number title state url repository { nameWithOwner } updatedAt author { login } isDraft }
            ... on DraftIssue { title }
          }
          fieldValues(first: 100) {
            nodes {
              ... on ProjectV2ItemFieldSingleSelectValue { name field { ... on ProjectV2FieldCommon { name } } }
              ... on ProjectV2ItemFieldTextValue { text field { ... on ProjectV2FieldCommon { name } } }
            }
          }
        }
      }
    }
  }
}`;

interface GqlFieldValue {
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
  state?: string;
  url?: string;
  repository?: { nameWithOwner?: string } | null;
  updatedAt?: string;
  author?: { login?: string } | null;
  isDraft?: boolean;
  labels?: { nodes?: { name?: string }[] } | null;
  /** Sub-issue parent (issues only; null when none). */
  parent?: GqlEdgeNode | null;
  /** "blocked by" / "blocks" dependency edges (issues only). */
  blockedBy?: { nodes?: (GqlEdgeNode | null)[] } | null;
  blocking?: { nodes?: (GqlEdgeNode | null)[] } | null;
}
interface GqlItem {
  id?: string;
  databaseId?: number;
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
 * canonical ref, or null when the node is incomplete. */
function edgeRef(n: GqlEdgeNode | null | undefined): RefKey | null {
  if (typeof n?.number !== "number" || !n.repository?.nameWithOwner) return null;
  return refKey({ repo: n.repository.nameWithOwner, number: n.number });
}

/** Parse one GraphQL page (or a full response) into pool items.
 * Draft items / items without a linked issue/PR are skipped: they
 * have no ref the dashboard can resolve or link to. Edge fields
 * (parent/blockedBy/blocks) map only for issues — PRs and drafts
 * never carry them, and items whose issue has no edges parse with
 * the fields absent. Epic membership (epicRef) is NOT resolved here:
 * it needs the whole pool (parents may sit on other pages), so
 * resolveEpicRefs runs over the aggregate after all pages parse. */
export function parseProjectItems(json: unknown, projectUrl = PROJECT_URL): ProjectPoolItem[] {
  const nodes = (json as GqlProjectPage)?.data?.node?.items?.nodes ?? [];
  const out: ProjectPoolItem[] = [];
  for (const item of nodes) {
    if (!item?.id || !item.content) continue;
    const c = item.content;
    if (typeof c.number !== "number" || !c.repository?.nameWithOwner || !c.url) continue;
    const repo = c.repository.nameWithOwner;
    const isPr = item.type === "PULL_REQUEST" || /\/pull\//.test(c.url);
    const rawState = (c.state ?? "OPEN").toUpperCase();
    const parentRef = !isPr ? edgeRef(c.parent) : null;
    const blockedBy = !isPr ? (c.blockedBy?.nodes ?? []).map(edgeRef).filter((x): x is RefKey => x !== null) : [];
    const blocks = !isPr ? (c.blocking?.nodes ?? []).map(edgeRef).filter((x): x is RefKey => x !== null) : [];
    out.push({
      id: item.id,
      databaseId: item.databaseId,
      url: projectItemUrl(item.databaseId, projectUrl),
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
      status: singleSelect(item, "Status"),
      area: singleSelect(item, "Area"),
      horizon: singleSelect(item, "Horizon"),
      kind: singleSelect(item, "Kind"),
      ...(parentRef ? { parentRef } : {}),
      ...(blockedBy.length ? { blockedBy } : {}),
      ...(blocks.length ? { blocks } : {}),
    });
  }
  return out;
}

/** Resolve epic membership over a fully parsed pool (#2399): every
 * non-epic item whose sub-issue parent chain (within this pool) reaches
 * a Kind="Epic" item gains epicRef naming that epic. Epics themselves,
 * items with no epic ancestor, and chains that leave the pool keep the
 * field absent. Cycle-safe: a parent loop ends the walk with no epic. */
export function resolveEpicRefs(items: ProjectPoolItem[]): ProjectPoolItem[] {
  const byRef = new Map(items.map((p) => [p.ref, p] as const));
  return items.map((p) => {
    if (p.kind === "Epic" || !p.parentRef) return p;
    const seen = new Set<string>([p.ref]);
    let cursor: ProjectPoolItem | undefined = byRef.get(p.parentRef);
    while (cursor && !seen.has(cursor.ref)) {
      if (cursor.kind === "Epic") return { ...p, epicRef: cursor.ref };
      seen.add(cursor.ref);
      cursor = cursor.parentRef ? byRef.get(cursor.parentRef) : undefined;
    }
    return p;
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
    Array.isArray(value.fields) &&
    value.fields.every((field) =>
      isRecord(field) &&
      typeof field.id === "string" &&
      typeof field.name === "string" &&
      typeof field.dataType === "string" &&
      (field.options === undefined ||
        (Array.isArray(field.options) &&
          field.options.every((option) => isRecord(option) && typeof option.id === "string" && typeof option.name === "string"))),
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
  items: ProjectPoolItem[];
  audit: ProjectAudit;
}

/** Fetch the whole pool, board schema, views, and distributions (paginated). */
export async function fetchProjectSnapshot(
  token: string,
  fetchFn: typeof fetch = fetch,
  generatedAt = new Date().toISOString(),
): Promise<ProjectSnapshot> {
  const parsedItems: ProjectPoolItem[] = [];
  const auditItems: GqlItem[] = [];
  let firstPage: unknown;
  let cursor: string | null = null;
  for (let page = 0; page < 10; page++) {
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
    if (!firstPage) firstPage = json;
    const nodes = (json as GqlProjectPage).data?.node?.items?.nodes ?? [];
    auditItems.push(...nodes.filter((node): node is GqlItem => node !== null));
    parsedItems.push(...parseProjectItems(json));
    const info = projectPageInfo(json);
    if (!info.hasNextPage) {
      if (auditItems.length !== info.totalCount) {
        throw new Error(`github GraphQL project export incomplete (${auditItems.length}/${info.totalCount} items)`);
      }
      return {
        items: resolveEpicRefs(parsedItems),
        audit: parseProjectAudit(firstPage, auditItems, generatedAt),
      };
    }
    if (!info.endCursor || info.endCursor === cursor) throw new Error("github GraphQL project pagination did not advance");
    if (page === 9) throw new Error("github GraphQL project exceeds the 1,000-item export limit");
    cursor = info.endCursor;
  }
  throw new Error("github GraphQL project pagination ended unexpectedly");
}

/** Fetch the whole pool (paginated). Throws on HTTP/GraphQL errors so
 * the collector can fail-soft and omit the section. Never logs the token. */
export async function fetchProjectPool(
  token: string,
  fetchFn: typeof fetch = fetch,
): Promise<ProjectPoolItem[]> {
  return (await fetchProjectSnapshot(token, fetchFn)).items;
}

/** Structural check for a pool entry in a feed (collector + live share
 * it, and the feed.ts section gate delegates to it — the two sides can
 * never drift). Edge fields (#2399) are absence-friendly but
 * shape-strict when present: a malformed edge fails the item, never
 * the section. */
export function isProjectPoolItem(v: unknown): v is ProjectPoolItem {
  if (typeof v !== "object" || v === null) return false;
  const p = v as Partial<ProjectPoolItem>;
  return (
    typeof p.id === "string" &&
    typeof p.ref === "string" &&
    typeof p.repo === "string" &&
    typeof p.number === "number" &&
    typeof p.issueUrl === "string" &&
    typeof p.title === "string" &&
    (p.state === "open" || p.state === "closed" || p.state === "merged") &&
    (p.parentRef === undefined || typeof p.parentRef === "string") &&
    (p.epicRef === undefined || typeof p.epicRef === "string") &&
    (p.blockedBy === undefined || (Array.isArray(p.blockedBy) && p.blockedBy.every((x) => typeof x === "string"))) &&
    (p.blocks === undefined || (Array.isArray(p.blocks) && p.blocks.every((x) => typeof x === "string")))
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

export type DerivedProjectState = "Todo" | "In Progress" | "Done";

/**
 * Derived state — what the rest of the dashboard derives for an issue,
 * kept deliberately simple (documented per #910 stage 5):
 *   - issue/PR closed or merged            -> Done
 *   - open, and its ref appears anywhere in the orchestration
 *     section (a tier item, a goal's lined-up task, or a dependency)
 *                                            -> In Progress
 *   - otherwise                              -> Todo
 * There is no derived "In Review": a Project Status of "In Review"
 * therefore disagrees with an "In Progress" derivation, and the
 * /projects page shows that disagreement rather than hiding it.
 */
export function derivedProjectState(p: ProjectPoolItem, section?: OrchestrationSection | null): DerivedProjectState {
  if (p.state === "closed" || p.state === "merged") return "Done";
  if (section && refInOrchestration(p.ref, section)) return "In Progress";
  return "Todo";
}

function refInOrchestration(ref: RefKey, section: OrchestrationSection): boolean {
  for (const t of [...section.tiers.needsDrew, ...section.tiers.needsOrchestrator]) {
    if (t.ref === ref) return true;
  }
  for (const g of section.goals) {
    for (const t of g.linedUp) if (t.ref === ref) return true;
  }
  for (const d of section.dependencies) {
    if (d.ref === ref || d.taskRef === ref) return true;
  }
  return false;
}

/** The disagreement to display, or null when both sides agree (or the
 * Project carries no Status to disagree with). */
export function projectDisagreement(
  p: ProjectPoolItem,
  section?: OrchestrationSection | null,
): { project: string; derived: DerivedProjectState } | null {
  if (!p.status) return null;
  const derived = derivedProjectState(p, section);
  return p.status === derived ? null : { project: p.status, derived };
}
