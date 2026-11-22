import { describe, expect, test } from "bun:test";
import { fetchNativeSubissueTree } from "../src/epic-subtree.ts";
import type { ProjectFetch } from "../src/projects.ts";

function page(
  nodes: unknown[],
  hasNextPage = false,
  endCursor: string | null = null,
  totalCount = nodes.length,
): Response {
  return Response.json({
    data: {
      repository: {
        issue: {
          subIssues: {
            totalCount,
            nodes,
            pageInfo: { hasNextPage, endCursor },
          },
        },
      },
    },
  });
}

const child = (number: number, labels: string[] = []) => ({
  number,
  title: `Issue ${number}`,
  state: "OPEN",
  url: `https://github.com/developing-today/code/issues/${number}`,
  repository: { nameWithOwner: "developing-today/code" },
  labels: {
    totalCount: labels.length,
    nodes: labels.map((name) => ({ name })),
  },
});

describe("fetchNativeSubissueTree", () => {
  // Oracle: native parent edges, not board membership; every page/label count must be exhausted.
  test("follows cross-repo children with the same issue number as the root", async () => {
    const calls: string[] = [];
    const fetchFn: ProjectFetch = async (_input, init) => {
      const { owner, name, number } = JSON.parse(String(init?.body)).variables;
      calls.push(`${owner}/${name}#${number}`);
      return calls.length === 1 ? page([{ ...child(1), repository: { nameWithOwner: "another/repo" } }]) : page([]);
    };
    const tree = await fetchNativeSubissueTree("developing-today/code#1", "token", fetchFn);
    expect(tree[0]?.ref).toBe("another/repo#1");
    expect(tree[0]?.parentRef).toBe("developing-today/code#1");
    expect(calls).toEqual(["developing-today/code#1", "another/repo#1"]);
  });

  test("rejects missing, truncated and inconsistent connection facts", async () => {
    for (const response of [
      page([], false, null, 1),
      page([{ ...child(2), labels: { totalCount: 2, nodes: [] } }]),
      page([{ ...child(2), labels: undefined }]),
      Response.json({ data: { repository: { issue: null } } }),
    ]) {
      await expect(fetchNativeSubissueTree("developing-today/code#1", "token", async () => response)).rejects.toThrow();
    }
  });

  test("recurses and paginates, deduplicating cycles and repeated child links", async () => {
    const requests: { number: number; after: string | null }[] = [];
    const fetchFn: ProjectFetch = async (_input, init) => {
      const body = JSON.parse(String(init?.body)) as {
        variables: { number: number; after: string | null };
      };
      const { number, after } = body.variables;
      requests.push({ number, after });
      if (number === 1 && after === null) return page([child(2)], true, "cursor-1", 2);
      if (number === 1 && after === "cursor-1") return page([child(2), child(3, ["kind/bug"])]);
      if (number === 2) return page([child(1), child(4)]);
      return page([]);
    };

    const tree = await fetchNativeSubissueTree("developing-today/code#1", "test-token", fetchFn);

    expect(tree.map((item) => item.ref)).toEqual([
      "developing-today/code#2",
      "developing-today/code#4",
      "developing-today/code#3",
    ]);
    expect(tree[0]?.parentRef).toBe("developing-today/code#1");
    expect(tree[1]?.parentRef).toBe("developing-today/code#2");
    expect(tree[2]?.labels).toEqual(["kind/bug"]);
    expect(requests).toEqual([
      { number: 1, after: null },
      { number: 2, after: null },
      { number: 4, after: null },
      { number: 1, after: "cursor-1" },
      { number: 3, after: null },
    ]);
  });

  test("fails closed on missing pagination cursor or API errors", async () => {
    const stuck: ProjectFetch = async () => page([child(2)], true, null);
    await expect(fetchNativeSubissueTree("developing-today/code#1", "token", stuck)).rejects.toThrow(
      "pagination did not advance",
    );

    const failed: ProjectFetch = async () => Response.json({ errors: [{ message: "denied" }] });
    await expect(fetchNativeSubissueTree("developing-today/code#1", "token", failed)).rejects.toThrow(
      "GraphQL returned errors",
    );
  });
});
