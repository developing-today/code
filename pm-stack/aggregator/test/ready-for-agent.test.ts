import { describe, expect, test } from "bun:test";
import {
  dispatchStateFor,
  fetchRfaState,
  parseRfaState,
  resetRfaCache,
  rfaHealthFacet,
  RFA_NOT_PICKED_UP,
  type RfaFetch,
} from "../src/sources/ready-for-agent.ts";

// #1203: the aggregator's ready-for-agent source — dispatch-state
// parsing, the explicit "not picked up" fallback, and the /health
// facet's up/down rule. Payloads mirror the live harness schema
// (verified 2026-11-01 against :6056/graphql introspection).

const repositoriesPayload = {
  data: {
    repositories: [
      {
        id: "repo-1",
        projectPath: "developing-today/code",
        localPath: "/home/user/pm-stack/ready-for-agent/repos/code",
        paused: false,
        mergePolicy: "OFF",
        defaultModel: "opencode-go/kimi-k3.1-pro",
      },
      {
        id: "repo-2",
        projectPath: "dezren39/nix",
        localPath: null,
        paused: true,
        mergePolicy: null,
        defaultModel: null,
      },
    ],
  },
};

const backendsPayload = {
  data: { agentBackendStatuses: [{ kind: "READY", reason: null, backend: { id: "opencode" } }] },
};

const workItemsPayload = {
  data: {
    workItems: [{ issueNumber: 762, state: "COMPLETE", statusLabel: "Complete", pullRequestNumber: 763 }],
  },
};

function state() {
  return parseRfaState(repositoriesPayload, backendsPayload, [{ repositoryId: "repo-1", payload: workItemsPayload }]);
}

describe("ready-for-agent source parsing", () => {
  test("parses repositories, backends, and work items", () => {
    const s = state();
    expect(s.repositories).toHaveLength(2);
    expect(s.repositories[0].mergePolicy).toBe("OFF");
    expect(s.repositories[1].paused).toBe(true);
    expect(s.backends).toEqual([{ id: "opencode", kind: "READY", reason: null }]);
    expect(s.workItems[0]).toMatchObject({ repositoryId: "repo-1", issueNumber: 762, pullRequestNumber: 763 });
  });

  test("GraphQL errors throw instead of parsing as empty", () => {
    expect(() => parseRfaState({ errors: [{ message: "boom" }] }, backendsPayload, [])).toThrow("boom");
  });
});

describe("dispatchStateFor", () => {
  test("returns the work item's real state for a picked-up issue", () => {
    expect(dispatchStateFor(state(), "developing-today/code", 762)).toBe("Complete");
  });

  test("returns explicit 'not picked up' for a labelled issue with no work item", () => {
    expect(dispatchStateFor(state(), "developing-today/code", 1203)).toBe(RFA_NOT_PICKED_UP);
  });

  test("returns 'not picked up' for an unregistered repository", () => {
    expect(dispatchStateFor(state(), "developing-today/opencode", 1)).toBe(RFA_NOT_PICKED_UP);
  });
});

describe("rfaHealthFacet", () => {
  test("up when every backend is READY, with per-repo work item counts", () => {
    const facet = rfaHealthFacet(state());
    expect(facet.state).toBe("up");
    expect(facet.repositories[0]).toMatchObject({ projectPath: "developing-today/code", workItems: 1 });
    expect(facet.repositories[1].workItems).toBe(0);
  });

  test("down when a backend is not READY, naming the reason", () => {
    const s = parseRfaState(repositoriesPayload, {
      data: { agentBackendStatuses: [{ kind: "UNAVAILABLE", reason: "opencode not found", backend: { id: "opencode" } }] },
    }, []);
    const facet = rfaHealthFacet(s);
    expect(facet.state).toBe("down");
    expect(facet.detail).toContain("opencode not found");
  });
});

describe("fetchRfaState", () => {
  test("queries repositories, backends, and per-repo work items over GraphQL", async () => {
    resetRfaCache();
    const queries: string[] = [];
    const fetcher: RfaFetch = async (_url, init) => {
      const { query } = JSON.parse(init.body) as { query: string };
      queries.push(query);
      const payload = query.includes("agentBackendStatuses")
        ? backendsPayload
        : query.includes("workItems")
          ? workItemsPayload
          : repositoriesPayload;
      return { json: async () => payload };
    };
    const s = await fetchRfaState("http://127.0.0.1:6056", fetcher, 1_000);
    expect(queries.some((q) => q.includes("repositories"))).toBe(true);
    expect(queries.some((q) => q.includes("agentBackendStatuses"))).toBe(true);
    // One workItems query per registered repository.
    expect(queries.filter((q) => q.includes("workItems"))).toHaveLength(2);
    expect(s.workItems).toHaveLength(2);
    // Cached: a second call inside the window sends no new queries.
    const again = await fetchRfaState("http://127.0.0.1:6056", fetcher, 2_000);
    expect(again).toBe(s);
    expect(queries).toHaveLength(4);
    resetRfaCache();
  });
});
