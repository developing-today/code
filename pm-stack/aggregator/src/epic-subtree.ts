import type { ProjectFetch } from "./projects.ts";
import type { RefKey } from "./model.ts";

const SUBISSUES_QUERY = `query($owner: String!, $name: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      subIssues(first: 100, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes { number title state url labels(first: 100) { nodes { name } } }
      }
    }
  }
}`;

export interface NativeSubissue {
  ref: RefKey;
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
          pageInfo?: { hasNextPage?: boolean; endCursor?: string | null };
          nodes?: Array<{
            number?: number;
            title?: string;
            state?: string;
            url?: string;
            labels?: { nodes?: Array<{ name?: string }> };
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
  const [owner, name] = repo.split("/");
  const seen = new Set<RefKey>([root]);
  const descendants: NativeSubissue[] = [];

  const visit = async (number: number): Promise<void> => {
    let after: string | null = null;
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
      if (!response.ok)
        throw new Error(
          `github GraphQL HTTP ${response.status} reading ${repo}#${number}`,
        );
      const json = (await response.json()) as SubissuePage;
      if (json.errors)
        throw new Error(
          `github GraphQL returned errors reading ${repo}#${number}`,
        );
      const connection = json.data?.repository?.issue?.subIssues;
      if (!connection?.pageInfo || !Array.isArray(connection.nodes))
        throw new Error(
          `github GraphQL omitted sub-issue connection for ${repo}#${number}`,
        );

      for (const child of connection.nodes) {
        if (
          !child ||
          !Number.isSafeInteger(child.number) ||
          (child.number ?? 0) <= 0 ||
          !child.url
        )
          throw new Error(
            `github GraphQL returned a malformed sub-issue for ${repo}#${number}`,
          );
        const ref = `${repo}#${child.number}` as RefKey;
        if (seen.has(ref)) continue;
        if (child.state !== "OPEN" && child.state !== "CLOSED")
          throw new Error(
            `github GraphQL returned an unknown state for ${ref}`,
          );
        seen.add(ref);
        descendants.push({
          ref,
          title: child.title ?? "",
          state: child.state,
          url: child.url,
          labels: (child.labels?.nodes ?? []).flatMap((label) =>
            typeof label?.name === "string" ? [label.name] : [],
          ),
        });
        await visit(child.number!);
      }

      if (!connection.pageInfo.hasNextPage) return;
      const cursor = connection.pageInfo.endCursor;
      if (!cursor || cursor === after)
        throw new Error(
          `github GraphQL sub-issue pagination did not advance for ${repo}#${number}`,
        );
      after = cursor;
    }
  };

  await visit(Number(rootNumber));
  return descendants;
}
