// Live ready-for-agent harness source (#1203): the aggregator's first
// reader for the dispatch layer. Until now config.readyForAgent.baseUrl
// fed only a static topology row, derive.ts treated the
// `ready-for-agent` label itself as "implementing", and a labelled
// issue could claim dispatch that never happened. This module queries
// the harness GraphQL API (loopback :6056/graphql — schema verified
// live 2026-11-01: repositories, agentBackendStatuses,
// workItems(repositoryId:)) and answers, per issue, the honest
// dispatch state: the work item's real state, or the explicit
// "not picked up" when the harness has no work item for it.
//
// Pure parsing is split from fetching so the unit tests never need a
// running harness; fetchRfaState caches for 60 s like the health
// probes, so a refreshing dashboard never fans out per hit.

export interface RfaRepository {
  id: string;
  projectPath: string;
  localPath: string | null;
  paused: boolean;
  mergePolicy: string | null;
  defaultModel: string | null;
}

export interface RfaBackend {
  id: string;
  /** Harness readiness, e.g. "READY". */
  kind: string;
  reason: string | null;
}

export interface RfaWorkItem {
  repositoryId: string;
  issueNumber: number;
  state: string;
  statusLabel: string | null;
  pullRequestNumber: number | null;
}

export interface RfaState {
  repositories: RfaRepository[];
  backends: RfaBackend[];
  workItems: RfaWorkItem[];
}

/** The explicit fallback when the harness has no work item for an issue. */
export const RFA_NOT_PICKED_UP = "not picked up";

interface GraphqlResponse<T> {
  data?: T;
  errors?: { message: string }[];
}

function checked<T>(payload: GraphqlResponse<T>, what: string): T {
  if (payload.errors?.length) {
    throw new Error(`ready-for-agent GraphQL ${what}: ${payload.errors[0].message}`);
  }
  if (!payload.data) throw new Error(`ready-for-agent GraphQL ${what}: empty response`);
  return payload.data;
}

/** Parse the repositories query payload into the read model. */
export function parseRfaRepositories(payload: GraphqlResponse<{ repositories: Record<string, unknown>[] }>): RfaRepository[] {
  const data = checked(payload, "repositories");
  return data.repositories.map((r) => ({
    id: String(r.id),
    projectPath: String(r.projectPath),
    localPath: typeof r.localPath === "string" ? r.localPath : null,
    paused: r.paused === true,
    mergePolicy: typeof r.mergePolicy === "string" ? r.mergePolicy : null,
    defaultModel: typeof r.defaultModel === "string" ? r.defaultModel : null,
  }));
}

/** Parse the agentBackendStatuses query payload. */
export function parseRfaBackends(payload: GraphqlResponse<{ agentBackendStatuses: Record<string, unknown>[] }>): RfaBackend[] {
  const data = checked(payload, "agentBackendStatuses");
  return data.agentBackendStatuses.map((b) => {
    const backend = (b.backend ?? {}) as Record<string, unknown>;
    return {
      id: typeof backend.id === "string" ? backend.id : "unknown",
      kind: typeof b.kind === "string" ? b.kind : "UNKNOWN",
      reason: typeof b.reason === "string" ? b.reason : null,
    };
  });
}

/** Parse one repository's workItems query payload. */
export function parseRfaWorkItems(
  payload: GraphqlResponse<{ workItems: Record<string, unknown>[] }>,
  repositoryId: string,
): RfaWorkItem[] {
  const data = checked(payload, "workItems");
  return data.workItems.map((w) => ({
    repositoryId,
    issueNumber: Number(w.issueNumber),
    state: typeof w.state === "string" ? w.state : "UNKNOWN",
    statusLabel: typeof w.statusLabel === "string" ? w.statusLabel : null,
    pullRequestNumber: typeof w.pullRequestNumber === "number" ? w.pullRequestNumber : null,
  }));
}

/** Assemble the full read model from the three query payloads. */
export function parseRfaState(
  repositoriesPayload: GraphqlResponse<{ repositories: Record<string, unknown>[] }>,
  backendsPayload: GraphqlResponse<{ agentBackendStatuses: Record<string, unknown>[] }>,
  workItemPayloads: { repositoryId: string; payload: GraphqlResponse<{ workItems: Record<string, unknown>[] }> }[],
): RfaState {
  return {
    repositories: parseRfaRepositories(repositoriesPayload),
    backends: parseRfaBackends(backendsPayload),
    workItems: workItemPayloads.flatMap(({ repositoryId, payload }) => parseRfaWorkItems(payload, repositoryId)),
  };
}

/**
 * The dispatch state of one issue: the harness work item's label (or
 * raw state) when the harness has picked the issue up, and the
 * explicit "not picked up" otherwise — never an inferred "implementing"
 * from the label alone.
 */
export function dispatchStateFor(state: RfaState, projectPath: string, issueNumber: number): string {
  const repo = state.repositories.find((r) => r.projectPath === projectPath);
  if (!repo) return RFA_NOT_PICKED_UP;
  const item = state.workItems.find((w) => w.repositoryId === repo.id && w.issueNumber === issueNumber);
  if (!item) return RFA_NOT_PICKED_UP;
  return item.statusLabel ?? item.state;
}

/** The /health facet shape for the harness. */
export interface RfaHealthFacet {
  state: "up" | "down" | "fixtures";
  detail: string;
  repositories: { projectPath: string; paused: boolean; mergePolicy: string | null; workItems: number }[];
  backends: RfaBackend[];
}

export function rfaHealthFacet(state: RfaState): RfaHealthFacet {
  const down = state.backends.find((b) => b.kind !== "READY");
  return {
    state: down ? "down" : "up",
    detail: down
      ? `backend ${down.id} is ${down.kind}${down.reason ? `: ${down.reason}` : ""}`
      : `${state.repositories.length} repositories, ${state.workItems.length} work items`,
    repositories: state.repositories.map((r) => ({
      projectPath: r.projectPath,
      paused: r.paused,
      mergePolicy: r.mergePolicy,
      workItems: state.workItems.filter((w) => w.repositoryId === r.id).length,
    })),
    backends: state.backends,
  };
}

export function rfaHealthDown(detail: string): RfaHealthFacet {
  return { state: "down", detail, repositories: [], backends: [] };
}

/** The minimal fetch shape this source needs — injectable for tests. */
export type RfaFetch = (
  url: string,
  init: { method: string; headers: Record<string, string>; body: string; signal: AbortSignal },
) => Promise<{ json(): Promise<unknown> }>;

const defaultFetch: RfaFetch = async (url, init) => fetch(url, init);

const REPOSITORIES_QUERY =
  "{ repositories { id projectPath localPath paused mergePolicy defaultModel } }";
const BACKENDS_QUERY = "{ agentBackendStatuses { kind reason backend { id } } }";
const workItemsQuery = (repositoryId: string) =>
  `{ workItems(repositoryId: ${JSON.stringify(repositoryId)}) { issueNumber state statusLabel pullRequestNumber } }`;

async function gql<T>(fetcher: RfaFetch, graphqlUrl: string, query: string): Promise<GraphqlResponse<T>> {
  const res = await fetcher(graphqlUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ query }),
    signal: AbortSignal.timeout(2_500),
  });
  return (await res.json()) as GraphqlResponse<T>;
}

export const RFA_CACHE_MS = 60_000;
let cached: { at: number; state: RfaState } | null = null;

/** Fetch the harness state live, cached 60 s across callers. */
export async function fetchRfaState(
  baseUrl: string,
  fetcher: RfaFetch = defaultFetch,
  now: number = Date.now(),
): Promise<RfaState> {
  if (cached && now - cached.at < RFA_CACHE_MS) return cached.state;
  const graphqlUrl = `${baseUrl.replace(/\/$/, "")}/graphql`;
  const repositoriesPayload = await gql<{ repositories: Record<string, unknown>[] }>(fetcher, graphqlUrl, REPOSITORIES_QUERY);
  const repositories = parseRfaRepositories(repositoriesPayload);
  const backendsPayload = await gql<{ agentBackendStatuses: Record<string, unknown>[] }>(fetcher, graphqlUrl, BACKENDS_QUERY);
  const workItemPayloads = [];
  for (const repo of repositories) {
    workItemPayloads.push({
      repositoryId: repo.id,
      payload: await gql<{ workItems: Record<string, unknown>[] }>(fetcher, graphqlUrl, workItemsQuery(repo.id)),
    });
  }
  const state = parseRfaState(repositoriesPayload, backendsPayload, workItemPayloads);
  cached = { at: now, state };
  return state;
}

/** Test seam: drop the shared cache. */
export function resetRfaCache(): void {
  cached = null;
}
