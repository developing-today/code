/**
 * The pool wire shape and its parser — the hosted collector's half of
 * the same-world contract (component A design §3, §7 invariant 1).
 *
 * The GraphQL below is the SAME pool query the laptop collector runs
 * (pm-stack/aggregator/src/projects.ts PROJECT_QUERY: fields, views,
 * parent, blockedBy/blocking, labels, milestones, subIssuesSummary),
 * and the parse/map functions are a port of that module's, producing
 * the same ProjectPoolItem / ProjectAudit shapes (aggregator
 * model.ts) the tenant store's schema was pinned against in slice 1.
 * The port is deliberate: pm-service deploys as one Worker bundle and
 * cannot import the aggregator app; the dual-edition conformance
 * suite (design §8 slice 8) is what pins the two implementations to
 * identical output on identical input.
 *
 * Nothing in this module performs I/O, and nothing here ever sees a
 * token: parsing is a pure function of the GraphQL response.
 */

/** "owner/repo#123" — the canonical item key (aggregator RefKey). */
export type RefKey = string;

export function refKey(input: { repo: string; number: number }): RefKey {
  return `${input.repo}#${input.number}`;
}

export function isValidRef(value: unknown): value is RefKey {
  return typeof value === "string" && /^[^/\s#]+\/[^/\s#]+#\d+$/.test(value);
}

/** GraphQL for one page of project items plus the board's audit schema.
 * Byte-identical to the laptop collector's PROJECT_QUERY. */
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

export interface MilestoneRef {
  title: string;
  dueOn: string | null;
  closedAt: string | null;
}

export interface SubIssuesSummary {
  total: number;
  completed: number;
}

export interface SpawnRollup {
  total: number;
  done: number;
  inProgress: number;
  blocking: RefKey[];
}

export interface ProjectPoolEdgeDiagnostic {
  relation: "parent" | "blockedBy" | "blocks";
  state: "unavailable" | "malformed";
  target: string | null;
}

/** One item of the merged pool — the aggregator's ProjectPoolItem
 * shape (model.ts), field for field; `item_json` in pm_items is this
 * object serialized byte-faithful. */
export interface ProjectPoolItem {
  boardId?: string;
  id: string;
  databaseId?: number;
  url: string;
  projectUrl: string;
  ref: RefKey;
  repo: string;
  number: number;
  issueUrl: string;
  title: string;
  state: "open" | "closed" | "merged";
  contentKind: "issue" | "pr";
  draft?: boolean;
  labels?: string[];
  author?: string;
  updatedAt?: string;
  closedAt?: string;
  mergedAt?: string;
  milestone?: MilestoneRef;
  subIssuesSummary?: SubIssuesSummary;
  spawnRollup?: SpawnRollup;
  status: string | null;
  area: string | null;
  horizon: string | null;
  kind: string | null;
  parentRef?: RefKey;
  blockedBy?: RefKey[];
  blocks?: RefKey[];
  edgeDiagnostics?: ProjectPoolEdgeDiagnostic[];
  epicRef?: RefKey;
  historicalParentRefs?: RefKey[];
}

export interface ProjectAuditField {
  id: string;
  name: string;
  dataType: string;
  options?: { id: string; name: string }[];
}

export interface ProjectAuditView {
  id: string;
  name: string;
  layout: string;
  filter: string | null;
}

/** The per-board audit snapshot stored as pm_snapshots.audit_json. */
export interface ProjectAudit {
  title: string;
  number: number;
  url: string;
  generatedAt: string;
  totalItems: number;
  poolTotalItems?: number;
  fields: ProjectAuditField[];
  views: ProjectAuditView[];
  counts: {
    status: Record<string, number>;
    area: Record<string, number>;
    repo: Record<string, number>;
  };
}

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
  milestone?: { title?: string; dueOn?: string | null; closedAt?: string | null } | null;
  subIssuesSummary?: { total?: number; completed?: number } | null;
  labels?: { nodes?: { name?: string }[] } | null;
  parent?: GqlEdgeNode | null;
  blockedBy?: { nodes?: (GqlEdgeNode | null)[] | null } | null;
  blocking?: { nodes?: (GqlEdgeNode | null)[] | null } | null;
}
export interface GqlItem {
  id?: string;
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The canonical Project-item URL: the project board with the item pane open. */
export function projectItemUrl(databaseId: number | undefined, projectUrl: string): string {
  return databaseId != null ? `${projectUrl}?pane=issue&itemId=${databaseId}` : projectUrl;
}

function singleSelect(item: GqlItem, fieldName: string): string | null {
  for (const v of item.fieldValues?.nodes ?? []) {
    if (v && v.field?.name === fieldName && typeof v.name === "string") return v.name;
  }
  return null;
}

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
 * have no ref the pool can resolve. Edge fields map only for issues.
 * Epic membership is NOT resolved here — it needs the whole merged
 * pool, so resolveEpicRefs runs over the aggregate after all boards
 * and pages parse (that is what lets cross-board ancestry resolve). */
export function parseProjectItems(json: unknown, projectUrl: string, boardId?: string): ProjectPoolItem[] {
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
      // Normalize the nullable GraphQL databaseId to the model's
      // optional number: null becomes absent (laptop parity).
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
              total: c.subIssuesSummary.total as number,
              completed: c.subIssuesSummary.completed as number,
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

/** The canonical PM Metadata block embedded in an issue body, parsed
 * but not yet validated — the store validates with slice 1's
 * validateMetadataBlock before writing a pm_item_metadata copy. */
export function metadataBlockFromBody(body: string): unknown {
  const block = /<!--\s*pm-metadata:start\s*-->([\s\S]*?)<!--\s*pm-metadata:end\s*-->/.exec(body)?.[1];
  if (!block) return null;
  const json = /```json\s*([\s\S]*?)```/i.exec(block)?.[1];
  if (!json) return null;
  try {
    return JSON.parse(json);
  } catch {
    return null;
  }
}

/** Resolve epic membership over a fully parsed MERGED pool: every
 * non-epic item whose sub-issue parent chain (within this pool,
 * across boards) reaches a Kind="Epic" item gains epicRef naming
 * that epic. Cycle-safe. Port of the laptop collector's resolver. */
export function resolveEpicRefs(items: ProjectPoolItem[]): ProjectPoolItem[] {
  const epicRefs = new Set(items.filter((p) => p.kind === "Epic").map((p) => p.ref));
  const byRef = new Map<string, ProjectPoolItem>();
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

/** Add a spawn rollup to design issues from direct native children
 * (port of the laptop collector's resolver; the GitHub summary is
 * authoritative for total/done counts). */
export function resolveSpawnRollups(items: ProjectPoolItem[]): ProjectPoolItem[] {
  const childrenByParent = new Map<string, ProjectPoolItem[]>();
  for (const item of items) {
    if (!item.parentRef) continue;
    const children = childrenByParent.get(item.parentRef) ?? [];
    children.push(item);
    childrenByParent.set(item.parentRef, children);
  }
  return items.map((item) => {
    const isDesign =
      item.labels?.some((label) => label === "design" || label.startsWith("design/") || label === "kind/design") ||
      item.kind?.toLowerCase() === "design";
    const summary = item.subIssuesSummary;
    if (!isDesign || !summary) {
      const { spawnRollup: _stale, ...unchanged } = item;
      return unchanged;
    }
    const children = childrenByParent.get(item.ref) ?? [];
    const directChildRefs = new Set(children.filter((child) => child.state === "open").map((child) => child.ref));
    const inProgressRefs = new Set(
      children.filter((child) => child.status?.toLowerCase() === "in progress").map((child) => child.ref),
    );
    return {
      ...item,
      spawnRollup: {
        total: summary.total,
        done: summary.completed,
        inProgress: inProgressRefs.size,
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

/** Build the board's audit snapshot from the first page's board schema and every item node. */
export function parseProjectAudit(json: unknown, items: GqlItem[], generatedAt: string): ProjectAudit {
  const node = (json as GqlProjectPage)?.data?.node;
  const itemConnection = node?.items;
  const totalItems = itemConnection?.totalCount;
  if (!node || !itemConnection || !node.fields || !node.views || totalItems === undefined || !Number.isInteger(totalItems) || totalItems < 0) {
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
    title: node.title ?? "",
    number: node.number ?? 0,
    url: node.url ?? "",
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
