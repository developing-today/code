import { describe, expect, test } from "bun:test";
import { fetchNativeSubissueTree } from "../src/epic-subtree.ts";
import type { ProjectFetch } from "../src/projects.ts";

function page(
  nodes: unknown[],
  hasNextPage = false,
  endCursor: string | null = null,
): Response {
  return Response.json({
    data: {
      repository: {
        issue: { subIssues: { nodes, pageInfo: { hasNextPage, endCursor } } },
      },
    },
  });
}

const child = (number: number, labels: string[] = []) => ({
  number,
  title: `Issue ${number}`,
  state: "OPEN",
  url: `https://github.com/developing-today/code/issues/${number}`,
  labels: { nodes: labels.map((name) => ({ name })) },
});

describe("fetchNativeSubissueTree", () => {
  test("recurses and paginates, deduplicating cycles and repeated child links", async () => {
    const requests: { number: number; after: string | null }[] = [];
    const fetchFn: ProjectFetch = async (_input, init) => {
      const body = JSON.parse(String(init?.body)) as {
        variables: { number: number; after: string | null };
      };
      const { number, after } = body.variables;
      requests.push({ number, after });
      if (number === 1 && after === null)
        return page([child(2)], true, "cursor-1");
      if (number === 1 && after === "cursor-1")
        return page([child(2), child(3, ["kind/bug"])]);
      if (number === 2) return page([child(1), child(4)]);
      return page([]);
    };

    const tree = await fetchNativeSubissueTree(
      "developing-today/code#1",
      "test-token",
      fetchFn,
    );

    expect(tree.map((item) => item.ref)).toEqual([
      "developing-today/code#2",
      "developing-today/code#4",
      "developing-today/code#3",
    ]);
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
    await expect(
      fetchNativeSubissueTree("developing-today/code#1", "token", stuck),
    ).rejects.toThrow("pagination did not advance");

    const failed: ProjectFetch = async () =>
      Response.json({ errors: [{ message: "denied" }] });
    await expect(
      fetchNativeSubissueTree("developing-today/code#1", "token", failed),
    ).rejects.toThrow("GraphQL returned errors");
  });
});
