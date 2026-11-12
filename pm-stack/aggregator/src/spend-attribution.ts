import type { ProjectPoolItem, RefKey } from "./model.ts";

/** Increment when the rules that produce spend facts change. */
export const ATTRIBUTION_VERSION = "v1";

export interface SpendTokens {
  input: number;
  output: number;
  reasoning: number;
  cacheRead: number;
  cacheWrite: number;
}

export interface SpendSession {
  id: string;
  parentId: string | null;
  directory: string | null;
  providerID: string | null;
  modelID: string | null;
  tokens: SpendTokens;
}

export interface SpendLaneBinding {
  sessionId: string;
  taskId: string;
  issueRef: RefKey;
}

export type SpendUnattributedReason = "no-binding" | "no-worktree" | "parent-left-pool" | "source-gap";
export type SpendAttributionMethod = "queue-binding" | "fleet-worktree" | "unattributed";

export interface SpendAttributionFact {
  attributionVersion: string;
  rootSessionId: string | null;
  sessionIds: string[];
  providerID: string | null;
  modelID: string | null;
  tokens: SpendTokens | null;
  method: SpendAttributionMethod;
  issueRef: RefKey | null;
  epicRef: RefKey | null;
  projectId: string | null;
  reason: SpendUnattributedReason | null;
}

export interface SpendAttributionInput {
  /** A failed or missing export is not equivalent to a successful empty export. */
  sourceStatus: "ok" | "missing" | "error";
  sessions: readonly SpendSession[];
  bindings: readonly SpendLaneBinding[];
  pool: readonly ProjectPoolItem[];
  /** Exact project membership supplied by the caller; Area is never used as a key. */
  projectByRef?: ReadonlyMap<RefKey, string>;
  /** Optional authoritative directory-to-issue mapping, keyed by directory or basename. */
  worktreeIssueRefs?: ReadonlyMap<string, RefKey>;
}

export interface SpendAttributionProjection {
  attributionVersion: string;
  facts: SpendAttributionFact[];
  sourceTotals: SpendTokens | null;
  byIssue: Map<RefKey, SpendTokens>;
  byEpic: Map<RefKey, SpendTokens>;
  byProject: Map<string, SpendTokens>;
  unattributedByReason: Map<SpendUnattributedReason, SpendTokens>;
  /** Null means the source failed or was absent, so reconciliation is unknown. */
  reconciled: boolean | null;
}

const ZERO: SpendTokens = { input: 0, output: 0, reasoning: 0, cacheRead: 0, cacheWrite: 0 };
const TOKEN_KEYS = ["input", "output", "reasoning", "cacheRead", "cacheWrite"] as const;

function addTokens(target: SpendTokens, value: SpendTokens): void {
  for (const key of TOKEN_KEYS) target[key] += value[key];
}

function sumTokens(values: readonly SpendTokens[]): SpendTokens {
  const total = { ...ZERO };
  for (const value of values) addTokens(total, value);
  return total;
}

function addToGroup<K>(groups: Map<K, SpendTokens>, key: K, value: SpendTokens): void {
  const total = groups.get(key) ?? { ...ZERO };
  addTokens(total, value);
  groups.set(key, total);
}

function directoryBase(directory: string): string {
  return (
    directory
      .replace(/[\\/]+$/, "")
      .split(/[\\/]/)
      .at(-1) ?? directory
  );
}

function fleetIssueNumber(directory: string | null): number | null {
  if (!directory || !/(?:^|[\\/])fleet[\\/]/.test(directory)) return null;
  const match = /^(\d+)-(\d+)-[^/\\]+$/.exec(directoryBase(directory));
  return match ? Number(match[2]) : null;
}

function resolveWorktreeIssue(directory: string | null, input: SpendAttributionInput): RefKey | null {
  if (!directory) return null;
  const base = directoryBase(directory);
  const explicit = input.worktreeIssueRefs?.get(directory) ?? input.worktreeIssueRefs?.get(base);
  if (explicit) return explicit;

  const issueNumber = fleetIssueNumber(directory);
  if (issueNumber === null) return null;
  const matches = input.pool.filter((item) => item.number === issueNumber && item.contentKind === "issue");
  // The directory contains no repository name. Only infer when the issue number
  // has exactly one candidate in the fetched pool; never guess across repos.
  return matches.length === 1 ? matches[0]!.ref : null;
}

interface RootGroup {
  rootSessionId: string;
  sessions: SpendSession[];
  /** A missing parent means the actual root was outside this source window. */
  parentLeftPool: boolean;
}

function collectRootGroups(sessions: readonly SpendSession[]): RootGroup[] {
  const byId = new Map(sessions.map((session) => [session.id, session] as const));
  const groups = new Map<string, RootGroup>();

  for (const session of sessions) {
    let cursor = session;
    const visited = new Set<string>();
    let missingParent: string | null = null;
    while (cursor.parentId) {
      if (visited.has(cursor.id)) break;
      visited.add(cursor.id);
      const parent = byId.get(cursor.parentId);
      if (!parent) {
        missingParent = cursor.parentId;
        break;
      }
      cursor = parent;
    }
    const rootSessionId = missingParent ?? cursor.id;
    let group = groups.get(rootSessionId);
    if (!group) {
      group = { rootSessionId, sessions: [], parentLeftPool: missingParent !== null };
      groups.set(rootSessionId, group);
    }
    group.sessions.push(session);
    group.parentLeftPool ||= missingParent !== null;
  }

  return [...groups.values()].sort((a, b) => a.rootSessionId.localeCompare(b.rootSessionId));
}

function addFactTotals(
  fact: SpendAttributionFact,
  byIssue: Map<RefKey, SpendTokens>,
  byEpic: Map<RefKey, SpendTokens>,
  byProject: Map<string, SpendTokens>,
  unattributedByReason: Map<SpendUnattributedReason, SpendTokens>,
): void {
  if (!fact.tokens) return;
  if (fact.issueRef) addToGroup(byIssue, fact.issueRef, fact.tokens);
  if (fact.epicRef) addToGroup(byEpic, fact.epicRef, fact.tokens);
  if (fact.projectId) addToGroup(byProject, fact.projectId, fact.tokens);
  if (fact.reason && !fact.issueRef) addToGroup(unattributedByReason, fact.reason, fact.tokens);
}

/**
 * Project immutable per-session usage into root-session issue/epic/project facts.
 * All token dimensions are conserved independently. This function never uses
 * OpenCode's list-price `cost` as paid-dollar spend.
 */
export function projectSpendAttribution(input: SpendAttributionInput): SpendAttributionProjection {
  const byIssue = new Map<RefKey, SpendTokens>();
  const byEpic = new Map<RefKey, SpendTokens>();
  const byProject = new Map<string, SpendTokens>();
  const unattributedByReason = new Map<SpendUnattributedReason, SpendTokens>();

  if (input.sourceStatus !== "ok") {
    const fact: SpendAttributionFact = {
      attributionVersion: ATTRIBUTION_VERSION,
      rootSessionId: null,
      sessionIds: [],
      providerID: null,
      modelID: null,
      tokens: null,
      method: "unattributed",
      issueRef: null,
      epicRef: null,
      projectId: null,
      reason: "source-gap",
    };
    return {
      attributionVersion: ATTRIBUTION_VERSION,
      facts: [fact],
      sourceTotals: null,
      byIssue,
      byEpic,
      byProject,
      unattributedByReason,
      reconciled: null,
    };
  }

  const poolByRef = new Map(input.pool.map((item) => [item.ref, item] as const));
  const bindingsBySession = new Map(input.bindings.map((binding) => [binding.sessionId, binding] as const));
  const facts: SpendAttributionFact[] = [];

  for (const group of collectRootGroups(input.sessions)) {
    const root = input.sessions.find((session) => session.id === group.rootSessionId);
    // Consult the exact binding for the group root even when the root's own
    // session row is outside the source window: the binding is keyed by the
    // root session ID this group already carries, and discarding it would
    // strand the whole group's usage as unattributed despite authoritative
    // issue information being available (PR #5164 Codex finding).
    const binding = bindingsBySession.get(group.rootSessionId);
    const issueRef = binding?.issueRef ?? resolveWorktreeIssue(root?.directory ?? null, input);
    const method: SpendAttributionMethod = binding ? "queue-binding" : issueRef ? "fleet-worktree" : "unattributed";
    const poolItem = issueRef ? poolByRef.get(issueRef) : undefined;

    let reason: SpendUnattributedReason | null = null;
    if (group.parentLeftPool) reason = "parent-left-pool";
    else if (!issueRef)
      reason = root?.directory && fleetIssueNumber(root.directory) !== null ? "no-worktree" : "no-binding";
    else if (!poolItem || (poolItem.parentRef && !poolByRef.has(poolItem.parentRef))) reason = "parent-left-pool";

    // Usage facts are split by provider/model within the root group: a child
    // session can run on a different provider or model from its root, and
    // summing across that boundary would label one pool's consumption with
    // another pool's identity, losing the per-pool accounting the design
    // requires (doc/spend-rollups-design-1259.md, Units; PR #5164 Codex
    // finding). Issue attribution stays with the root group for every
    // provider/model subgroup.
    const byProviderModel = new Map<string, SpendSession[]>();
    for (const session of group.sessions) {
      const key = `${session.providerID ?? ""}\u0000${session.modelID ?? ""}`;
      const subgroup = byProviderModel.get(key) ?? [];
      subgroup.push(session);
      byProviderModel.set(key, subgroup);
    }
    const subgroups = [...byProviderModel.values()].sort((a, b) =>
      `${a[0]!.providerID ?? ""}\u0000${a[0]!.modelID ?? ""}`.localeCompare(
        `${b[0]!.providerID ?? ""}\u0000${b[0]!.modelID ?? ""}`,
      ),
    );
    for (const subgroup of subgroups) {
      const fact: SpendAttributionFact = {
        attributionVersion: ATTRIBUTION_VERSION,
        rootSessionId: group.rootSessionId,
        sessionIds: subgroup.map((session) => session.id).sort(),
        providerID: subgroup[0]!.providerID,
        modelID: subgroup[0]!.modelID,
        tokens: sumTokens(subgroup.map((session) => session.tokens)),
        method,
        issueRef,
        epicRef: poolItem?.epicRef ?? (poolItem?.kind === "Epic" ? poolItem.ref : null),
        projectId: issueRef ? (input.projectByRef?.get(issueRef) ?? null) : null,
        reason,
      };
      facts.push(fact);
      addFactTotals(fact, byIssue, byEpic, byProject, unattributedByReason);
    }
  }

  const sourceTotals = sumTokens(input.sessions.map((session) => session.tokens));
  // Issue-attributed facts are a disjoint grouping. Reasons on a known issue
  // describe missing hierarchy context and do not double-count as unattributed.
  const noIssueTotals = sumTokens(facts.filter((fact) => !fact.issueRef && fact.tokens).map((fact) => fact.tokens!));
  const issueTotals = sumTokens([...byIssue.values()]);
  const reconciled = TOKEN_KEYS.every((key) => issueTotals[key] + noIssueTotals[key] === sourceTotals[key]);

  return {
    attributionVersion: ATTRIBUTION_VERSION,
    facts,
    sourceTotals,
    byIssue,
    byEpic,
    byProject,
    unattributedByReason,
    reconciled,
  };
}
