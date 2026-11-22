import type { ProjectFetch } from "./projects.ts";
import type { RefKey } from "./model.ts";

const SUBISSUES_QUERY = `query($owner: String!, $name: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      subIssues(first: 100, after: $after) {
        totalCount
        pageInfo { hasNextPage endCursor }
        nodes { number title state url repository { nameWithOwner } labels(first: 100) { totalCount nodes { name } } }
      }
    }
  }
}`;

export interface NativeSubissue {
  ref: RefKey;
  parentRef: RefKey;
  title: string;
  state: "OPEN" | "CLOSED";
  url: string;
  labels: string[];
}

interface SubissuePage {
  data?: {
    repository?: {
      issue?: {
        subIssues?: {
          totalCount?: number;
          pageInfo?: { hasNextPage?: boolean; endCursor?: string | null };
          nodes?: Array<{
            number?: number;
            title?: string;
            state?: string;
            url?: string;
            repository?: { nameWithOwner?: string };
            labels?: { totalCount?: number; nodes?: Array<{ name?: string }> };
          } | null>;
        } | null;
      } | null;
    } | null;
  };
  errors?: unknown;
}

/** Fetch every recursive native sub-issue, failing rather than presenting a partial tree as complete. */
export async function fetchNativeSubissueTree(
  root: RefKey,
  token: string,
  fetchFn: ProjectFetch = fetch,
): Promise<NativeSubissue[]> {
  const match = /^([^/]+\/[^#]+)#([1-9][0-9]*)$/.exec(root);
  if (!match) throw new Error(`invalid root issue ref: ${root}`);
  const [, repo, rootNumber] = match;
  const seen = new Set<RefKey>([root]);
  const descendants: NativeSubissue[] = [];

  const visit = async (repo: string, number: number): Promise<void> => {
    const [owner, name] = repo.split("/");
    let after: string | null = null;
    const cursors = new Set<string>();
    const directRefs = new Set<RefKey>();
    let expectedTotal: number | undefined;
    for (;;) {
      const response = await fetchFn("https://api.github.com/graphql", {
        method: "POST",
        headers: {
          Authorization: `Bearer ${token}`,
          "Content-Type": "application/json",
          "User-Agent": "pm-fleet-closure-gate",
        },
        body: JSON.stringify({
          query: SUBISSUES_QUERY,
          variables: { owner, name, number, after },
        }),
        signal: AbortSignal.timeout(20_000),
      });
      if (!response.ok) throw new Error(`github GraphQL HTTP ${response.status} reading ${repo}#${number}`);
      const json = (await response.json()) as SubissuePage;
      if (json.errors) throw new Error(`github GraphQL returned errors reading ${repo}#${number}`);
      const connection = json.data?.repository?.issue?.subIssues;
      if (
        !connection?.pageInfo ||
        typeof connection.pageInfo.hasNextPage !== "boolean" ||
        !Number.isSafeInteger(connection.totalCount) ||
        connection.totalCount! < 0 ||
        !Array.isArray(connection.nodes)
      )
        throw new Error(`github GraphQL omitted sub-issue connection for ${repo}#${number}`);
      if (expectedTotal !== undefined && expectedTotal !== connection.totalCount)
        throw new Error(`github GraphQL subtree changed during pagination for ${repo}#${number}`);
      expectedTotal = connection.totalCount;

      for (const child of connection.nodes) {
        if (
          !child ||
          !Number.isSafeInteger(child.number) ||
          (child.number ?? 0) <= 0 ||
          !child.url ||
          !/^[^/#]+\/[^/#]+$/.test(child.repository?.nameWithOwner ?? "")
        )
          throw new Error(`github GraphQL returned a malformed sub-issue for ${repo}#${number}`);
        const childRepo = child.repository!.nameWithOwner!;
        const ref = `${childRepo}#${child.number}` as RefKey;
        directRefs.add(ref);
        if (seen.has(ref)) continue;
        if (child.state !== "OPEN" && child.state !== "CLOSED")
          throw new Error(`github GraphQL returned an unknown state for ${ref}`);
        const labelNodes = child.labels?.nodes;
        if (
          !Array.isArray(labelNodes) ||
          child.labels?.totalCount !== labelNodes.length ||
          labelNodes.some((label) => typeof label?.name !== "string")
        )
          throw new Error(`github GraphQL returned incomplete labels for ${ref}`);
        seen.add(ref);
        descendants.push({
          ref,
          parentRef: `${repo}#${number}` as RefKey,
          title: child.title ?? "",
          state: child.state,
          url: child.url,
          labels: labelNodes.map((label) => label.name!),
        });
        await visit(childRepo, child.number!);
      }

      if (!connection.pageInfo.hasNextPage) {
        if (directRefs.size !== expectedTotal)
          throw new Error(`github GraphQL incomplete sub-issue count for ${repo}#${number}`);
        return;
      }
      const cursor = connection.pageInfo.endCursor;
      if (!cursor || cursors.has(cursor))
        throw new Error(`github GraphQL sub-issue pagination did not advance for ${repo}#${number}`);
      cursors.add(cursor);
      after = cursor;
    }
  };

  await visit(repo, Number(rootNumber));
  return descendants;
}
