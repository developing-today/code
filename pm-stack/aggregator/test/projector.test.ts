import { describe, expect, test } from "bun:test";
import {
  mapProjectorComment,
  mapProjectorItem,
  mergeLinks,
  parseProjectorLabels,
  projectorComments,
  projectorItems,
  type ProjectorCommentRow,
  type ProjectorItemRow,
} from "../src/sources/live.ts";
import type { Link } from "../src/model.ts";

// Rows shaped exactly as Session J's projector returns them
// (pm-stack/projector/schema.sql + src/store.ts): snake_case, state is
// open|closed only, merged/draft/deleted are 0/1, labels is a JSON string.
const mergedPr: ProjectorItemRow = {
  ref: "developing-today/code#752",
  repo: "developing-today/code",
  number: 752,
  kind: "pr",
  url: "https://github.com/developing-today/code/pull/752",
  title: "PM stack Phase 1",
  state: "closed",
  state_reason: "completed",
  draft: 0,
  merged: 1,
  author: "dezren39",
  labels: '["status:done","pm-stack"]',
  assignees: "[]",
  created_at: "2026-10-24T11:00:00Z",
  updated_at: "2026-10-24T13:40:00Z",
  closed_at: "2026-10-24T13:40:00Z",
  merged_at: "2026-10-24T13:40:00Z",
};

const openIssue: ProjectorItemRow = {
  ...mergedPr,
  ref: "dezren39/nix#501",
  repo: "dezren39/nix",
  number: 501,
  kind: "issue",
  url: "https://github.com/dezren39/nix/issues/501",
  title: "Basement cluster",
  state: "open",
  merged: 0,
  labels: '["cluster"]',
};

describe("projector item mapping", () => {
  test("merged flag wins over closed state", () => {
    const item = mapProjectorItem(mergedPr);
    expect(item.state).toBe("merged");
    expect(item.kind).toBe("pr");
    expect(item.draft).toBe(false);
    expect(item.updatedAt).toBe("2026-10-24T13:40:00Z");
  });

  test("open issue stays open; labels JSON string is parsed", () => {
    const item = mapProjectorItem(openIssue);
    expect(item.state).toBe("open");
    expect(item.labels).toEqual(["cluster"]);
  });

  test("labels tolerate arrays and junk", () => {
    expect(parseProjectorLabels(["a", "b"])).toEqual(["a", "b"]);
    expect(parseProjectorLabels("not json")).toEqual([]);
    expect(parseProjectorLabels(undefined)).toEqual([]);
    expect(parseProjectorLabels('["a", 1, "b"]')).toEqual(["a", "b"]);
  });
});

describe("projector comment mapping", () => {
  const reviewComment: ProjectorCommentRow = {
    id: "987",
    ref: "developing-today/code#763",
    kind: "review_comment",
    author: "dezren39",
    body: "Keep the link relative.",
    created_at: "2026-10-24T15:40:00Z",
    url: "https://github.com/developing-today/code/pull/763#discussion_r1",
    path: "doc/README.md",
    line: 42,
    review_state: null,
    deleted: 0,
  };

  test("snake_case review comment maps to the read model", () => {
    const c = mapProjectorComment(reviewComment);
    expect(c.createdAt).toBe("2026-10-24T15:40:00Z");
    expect(c.path).toBe("doc/README.md");
    expect(c.line).toBe(42);
    expect(c.deleted).toBe(false);
  });

  test("review_state maps; deleted 1 is truthy", () => {
    const c = mapProjectorComment({ ...reviewComment, kind: "review", review_state: "approved", deleted: 1 });
    expect(c.reviewState).toBe("approved");
    expect(c.deleted).toBe(true);
  });
});

describe("projector HTTP reads (stub server, actual wire shape)", () => {
  test("refs survive the query string (# is percent-encoded) and rows map", async () => {
    let seenRefs = "";
    const server = Bun.serve({
      port: 0,
      fetch(req) {
        const url = new URL(req.url);
        if (url.pathname === "/items") {
          seenRefs = url.searchParams.get("refs") ?? "";
          return Response.json({ items: [mergedPr] });
        }
        if (url.pathname === "/comments") {
          return Response.json({
            comments: [
              {
                id: "1",
                ref: "developing-today/code#752",
                kind: "review",
                author: "dezren39",
                body: "ok",
                created_at: "2026-10-24T13:35:00Z",
                url: "https://github.com/developing-today/code/pull/752#pullrequestreview-1",
                review_state: "approved",
                deleted: 0,
              },
              {
                id: "2",
                ref: "developing-today/code#752",
                kind: "issue_comment",
                author: "x",
                body: "gone",
                created_at: "2026-10-24T13:36:00Z",
                deleted: 1,
              },
            ],
          });
        }
        return new Response("not found", { status: 404 });
      },
    });
    try {
      const base = `http://127.0.0.1:${server.port}`;
      const items = await projectorItems(base, ["developing-today/code#752"]);
      expect(seenRefs).toBe("developing-today/code#752");
      expect(items?.get("developing-today/code#752")?.state).toBe("merged");
      const comments = await projectorComments(base, ["developing-today/code#752"]);
      // Deleted tombstones are dropped; review_state survives.
      expect(comments?.map((c) => c.id)).toEqual(["1"]);
      expect(comments?.[0]?.reviewState).toBe("approved");
    } finally {
      server.stop(true);
    }
  });
});

describe("projector HTTP reads (chunking past the 431 ceiling)", () => {
  test("600 refs are chunked under the request-line limit and merged", async () => {
    let requests = 0;
    const server = Bun.serve({
      port: 0,
      fetch(req) {
        // Bun.serve rejects request lines past ~16KB with 431 — the real
        // projector does exactly this, which is what broke the board
        // prime once it passed ~500 refs (2026-10-27, #1199).
        if (req.url.length > 16000) return new Response("headers too large", { status: 431 });
        requests++;
        const url = new URL(req.url);
        const refs = (url.searchParams.get("refs") ?? "").split(",").filter(Boolean);
        if (url.pathname === "/items") {
          return Response.json({ items: refs.map((ref) => ({ ...mergedPr, ref })) });
        }
        if (url.pathname === "/comments") {
          return Response.json({
            comments: refs.map((ref) => ({
              id: `c-${ref}`,
              ref,
              kind: "issue_comment",
              author: "dezren39",
              body: "ok",
              created_at: "2026-10-27T00:00:00Z",
              deleted: 0,
            })),
          });
        }
        return new Response("not found", { status: 404 });
      },
    });
    try {
      const base = `http://127.0.0.1:${server.port}`;
      const refs = Array.from({ length: 600 }, (_, i) => `developing-today/code#${1000 + i}`);
      const items = await projectorItems(base, refs);
      expect(items?.size).toBe(600);
      const comments = await projectorComments(base, refs);
      expect(comments?.length).toBe(600);
      // 600 refs at 200 per request = 3 requests per endpoint.
      expect(requests).toBe(6);
    } finally {
      server.stop(true);
    }
  });
});

describe("registry link merge", () => {
  test("unions cross-repo links without duplicating ref+relation", () => {
    const existing: Link[] = [{ ref: "developing-today/code#902", relation: "implements" }];
    const extra: Link[] = [
      { ref: "developing-today/code#902", relation: "implements" },
      { ref: "developing-today/openchamber#210", relation: "relates" },
    ];
    expect(mergeLinks(existing, extra)).toEqual([
      { ref: "developing-today/code#902", relation: "implements" },
      { ref: "developing-today/openchamber#210", relation: "relates" },
    ]);
  });
});
