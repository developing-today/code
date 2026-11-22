#!/usr/bin/env bun
// Dry-run by default. Apply only with --apply.
// Example: bun run scripts/epic-reparent.ts --child owner/repo#12 --closing-epic owner/repo#34 --maintenance owner/repo#56 --candidate owner/repo#78 --area owner/repo#12=ID --area owner/repo#78=ID

import {
  applyEpicReparent,
  planEpicReparent,
  setHistoricalParentMetadata,
  selectReparentDestination,
  type ReparentEpic,
  type ReparentGateway,
  type ReparentIssue,
} from "../src/epic-reparent.ts";
import type { RefKey } from "../src/model.ts";

interface Args {
  child: RefKey;
  closingEpic: RefKey;
  maintenance: RefKey;
  candidates: RefKey[];
  areas: Map<RefKey, string>;
  apply: boolean;
}

function parseArgs(argv: string[]): Args {
  const values = new Map<string, string[]>();
  const flags = new Set<string>();
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i]!;
    if (!["--child", "--closing-epic", "--maintenance", "--candidate", "--area"].includes(flag)) {
      if (flag === "--apply" && !flags.has(flag)) {
        flags.add(flag);
        continue;
      }
      throw new Error(`unknown or duplicate argument: ${flag}`);
    }
    const value = argv[++i];
    if (!value) throw new Error(`${flag} requires a value`);
    const entries = values.get(flag) ?? [];
    if (flag !== "--candidate" && flag !== "--area" && entries.length > 0)
      throw new Error(`${flag} may be supplied only once`);
    entries.push(value);
    values.set(flag, entries);
  }
  const one = (flag: string): RefKey => {
    const value = values.get(flag)?.[0];
    if (!value) throw new Error(`required argument: ${flag}`);
    if (!/^[^/#]+\/[^/#]+#[1-9][0-9]*$/.test(value)) throw new Error(`invalid ref for ${flag}: ${value}`);
    return value as RefKey;
  };
  const candidates = (values.get("--candidate") ?? []).map((value) => {
    if (!/^[^/#]+\/[^/#]+#[1-9][0-9]*$/.test(value)) throw new Error(`invalid candidate ref: ${value}`);
    return value as RefKey;
  });
  const maintenance = one("--maintenance");
  if (!candidates.includes(maintenance)) candidates.push(maintenance);
  const areas = new Map<RefKey, string>();
  for (const pair of values.get("--area") ?? []) {
    const split = pair.indexOf("=");
    const ref = pair.slice(0, split);
    const area = pair.slice(split + 1).trim();
    if (split < 1 || !/^[^/#]+\/[^/#]+#[1-9][0-9]*$/.test(ref) || !area)
      throw new Error(`invalid --area value: ${pair}`);
    if (areas.has(ref as RefKey)) throw new Error(`duplicate --area ref: ${ref}`);
    areas.set(ref as RefKey, area);
  }
  return { child: one("--child"), closingEpic: one("--closing-epic"), maintenance, candidates, areas, apply: flags.has("--apply") };
}

function tokenFromEnvironmentOrGh(): string {
  const direct = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  if (direct) return direct;
  const result = Bun.spawnSync(["gh", "auth", "token"], { stdout: "pipe", stderr: "pipe" });
  if (result.exitCode !== 0) throw new Error("no GitHub token; set GH_TOKEN or authenticate gh");
  const token = result.stdout.toString().trim();
  if (!token) throw new Error("gh auth token returned an empty token");
  return token;
}

const ISSUE_QUERY = `query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){issue(number:$number){id number state url body labels(first:100){totalCount nodes{name}} parent{number repository{nameWithOwner}}}}}`;
const ADD_PARENT = `mutation($parent:ID!,$child:ID!){addSubIssue(input:{issueId:$parent,subIssueId:$child,replaceParent:true}){issue{id}}}`;
const REMOVE_PARENT = `mutation($parent:ID!,$child:ID!){removeSubIssue(input:{issueId:$parent,subIssueId:$child}){issue{id}}}`;

interface IssueRecord extends ReparentIssue { id: string; url: string; body: string; }

function splitRef(ref: RefKey): { owner: string; name: string; number: number } {
  const match = /^([^/]+)\/([^#]+)#([1-9][0-9]*)$/.exec(ref);
  if (!match) throw new Error(`invalid issue ref: ${ref}`);
  return { owner: match[1]!, name: match[2]!, number: Number(match[3]) };
}

async function main(): Promise<void> {
  // Parse and validate all arguments before reading credentials or making requests.
  const args = parseArgs(process.argv.slice(2));
  if (new Set(args.candidates).size !== args.candidates.length) throw new Error("duplicate candidate refs");
  const token = tokenFromEnvironmentOrGh();
  const cache = new Map<RefKey, IssueRecord>();
  const request = async (url: string, init: RequestInit): Promise<unknown> => {
    const response = await fetch(url, init);
    const body = await response.json().catch(() => null) as { errors?: unknown; message?: string; data?: unknown } | null;
    if (!response.ok || body?.errors) throw new Error(`GitHub request failed (${response.status})${body?.message ? `: ${body.message}` : ""}`);
    return body;
  };
  const fetchIssue = async (ref: RefKey, fresh = false): Promise<IssueRecord> => {
    const cached = cache.get(ref);
    if (!fresh && cached) return cached;
    const location = splitRef(ref);
    const responseBody = await request("https://api.github.com/graphql", {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json", "User-Agent": "pm-fleet-epic-reparent" },
      body: JSON.stringify({ query: ISSUE_QUERY, variables: location }),
      signal: AbortSignal.timeout(20_000),
    });
    const body = responseBody as {
      data?: {
        repository?: {
          issue?: {
            id?: string;
            number?: number;
            state?: string;
            url?: string;
            body?: string;
            labels?: { totalCount?: number; nodes?: { name?: string }[] };
            parent?: { number?: number; repository?: { nameWithOwner?: string } } | null;
          } | null;
        } | null;
      };
    };
    const issue = body.data?.repository?.issue;
    const labels = issue?.labels?.nodes;
    if (!issue?.id || !issue.url || typeof issue.body !== "string" || !Array.isArray(labels) || labels.length !== issue.labels?.totalCount || labels.some((label) => typeof label.name !== "string"))
      throw new Error(`incomplete issue metadata for ${ref}`);
    if (issue.state !== "OPEN" && issue.state !== "CLOSED") throw new Error(`unknown state for ${ref}`);
    const parentRef = issue.parent?.repository?.nameWithOwner && issue.parent.number
      ? `${issue.parent.repository.nameWithOwner}#${issue.parent.number}` as RefKey
      : null;
    let currentParent = parentRef;
    const chain = new Set<RefKey>();
    while (currentParent) {
      if (chain.has(currentParent)) throw new Error(`cycle in parent chain for ${ref}`);
      chain.add(currentParent);
      const parentIssue = await fetchIssue(currentParent);
      currentParent = parentIssue.parentRef;
    }
    const record: IssueRecord = {
      id: issue.id,
      url: issue.url,
      body: issue.body,
      ref,
      parentRef,
      state: issue.state,
      labels: labels.map((label) => label.name!),
      primaryParentChain: [...chain],
      area: args.areas.get(ref) ?? null,
    };
    cache.set(ref, record);
    return record;
  };

  const child = await fetchIssue(args.child);
  if (child.parentRef !== args.closingEpic)
    throw new Error(`child is not a direct native child of the closing epic: ${args.child}`);
  const candidateIssues = await Promise.all(args.candidates.map(fetchIssue));
  const candidateEpics: ReparentEpic[] = candidateIssues.map((candidate) => ({
    ref: candidate.ref,
    state: candidate.state,
    labels: candidate.labels,
    primaryParentChain: candidate.primaryParentChain,
    area: candidate.area,
    maintenance: candidate.ref === args.maintenance,
  }));
  const selection = selectReparentDestination(child, candidateEpics);
  if (selection.kind !== "selected") throw new Error(`cannot choose destination: ${JSON.stringify(selection)}`);
  const plan = planEpicReparent(child, selection.epic);
  const preview = { mode: args.apply ? "apply" : "dry-run", selection: selection.reason, plan, commentsOn: [args.child, args.closingEpic] };
  console.log(JSON.stringify(preview, null, 2));
  if (!args.apply) return;

  const gateway: ReparentGateway = {
    readChildLive: async (ref) => fetchIssue(ref, true),
    isOpenEpicLive: async (ref) => {
      const candidate = await fetchIssue(ref, true);
      return candidate.state === "OPEN" && candidate.labels.includes("epic");
    },
    replaceNativeParent: async (_child, _oldParent, newParent) => {
      const target = await fetchIssue(newParent, true);
      const subissue = await fetchIssue(args.child, true);
      await request("https://api.github.com/graphql", {
        method: "POST",
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json", "User-Agent": "pm-fleet-epic-reparent" },
        body: JSON.stringify({ query: ADD_PARENT, variables: { parent: target.id, child: subissue.id } }),
        signal: AbortSignal.timeout(20_000),
      });
    },
    removeNativeParent: async (child, parent) => {
      const target = await fetchIssue(parent, true);
      const subissue = await fetchIssue(child, true);
      await request("https://api.github.com/graphql", {
        method: "POST",
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json", "User-Agent": "pm-fleet-epic-reparent" },
        body: JSON.stringify({ query: REMOVE_PARENT, variables: { parent: target.id, child: subissue.id } }),
        signal: AbortSignal.timeout(20_000),
      });
    },
    replaceLabels: async (ref, labels) => {
      const location = splitRef(ref);
      await request(`https://api.github.com/repos/${location.owner}/${location.name}/issues/${location.number}`, {
        method: "PATCH",
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json", "User-Agent": "pm-fleet-epic-reparent", "X-GitHub-Api-Version": "2022-11-28" },
        body: JSON.stringify({ labels }),
        signal: AbortSignal.timeout(20_000),
      });
    },
    recordHistoricalParent: async (ref, parent) => {
      return writeHistoricalParent(ref, parent, true);
    },
    removeHistoricalParent: async (ref, parent) => {
      await writeHistoricalParent(ref, parent, false);
    },
    addComment: async (ref, body) => {
      const location = splitRef(ref);
      await request(`https://api.github.com/repos/${location.owner}/${location.name}/issues/${location.number}/comments`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json", "User-Agent": "pm-fleet-epic-reparent", "X-GitHub-Api-Version": "2022-11-28" },
        body: JSON.stringify({ body }),
        signal: AbortSignal.timeout(20_000),
      });
    },
  };
  async function writeHistoricalParent(ref: RefKey, parent: RefKey, present: boolean): Promise<boolean> {
    const issue = await fetchIssue(ref, true);
    const body = setHistoricalParentMetadata(issue.body, parent, present);
    if (body === issue.body) return false;
    const location = splitRef(ref);
    await request(`https://api.github.com/repos/${location.owner}/${location.name}/issues/${location.number}`, {
      method: "PATCH",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json", "User-Agent": "pm-fleet-epic-reparent", "X-GitHub-Api-Version": "2022-11-28" },
      body: JSON.stringify({ body }),
      signal: AbortSignal.timeout(20_000),
    });
    return present;
  }
  await applyEpicReparent(plan, args.closingEpic, gateway);
}

main().catch((error: unknown) => {
  console.error(`epic-reparent: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
