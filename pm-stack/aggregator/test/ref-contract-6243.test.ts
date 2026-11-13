// Ref contract Phase 4 (#6243, design #6242): the two producer/consumer
// sites the closing inventory found still minting or dereferencing refs
// without the shared isValidRef rule, pinned:
//  - Kaneo board links are minted from a GitHub URL + externalId; an
//    externalId parseRef rejects (unsafe integer, non-numeric) used to
//    become a board link that renderers throw on in refUrl. The board
//    reader now drops it at minting (rule 3), traced, card survives.
//  - mapProjectorComment fell back to refUrl(row.ref) for a stored
//    projector comment with no stored url; an invalid stored ref threw
//    out of the map. The fallback now fires only for a valid ref
//    (rule 4); a stored url still wins over any ref.
//
// Run: bun test test/ref-contract-6243.test.ts
// Salvage (task 12925): the sessionRecordLine disposition-5 tests below
// come from PR #7642 (task 13017), whose render.ts fix is salvaged into
// this PR; see the salvage note on PR #7639.
import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource, mapProjectorComment, type ProjectorCommentRow } from "../src/sources/live.ts";
import { isValidRef, parseRef, refUrl, type RefKey, type SessionRecord } from "../src/model.ts";
import { sessionRecordLine } from "../src/render.ts";

describe("mapProjectorComment refUrl fallback (#6243)", () => {
  const row = (over: Partial<ProjectorCommentRow>): ProjectorCommentRow => ({
    id: "1",
    ref: "developing-today/code#6243",
    kind: "issue_comment",
    created_at: "2026-11-08T00:00:00Z",
    ...over,
  });

  test("valid ref without a stored url derives the comment url", () => {
    expect(mapProjectorComment(row({})).url).toBe("https://github.com/developing-today/code/issues/6243");
  });

  test("unsafe-integer ref without a stored url yields no url, never a throw", () => {
    const bad = row({ ref: "developing-today/code#99999999999999999999" });
    expect(() => mapProjectorComment(bad)).not.toThrow();
    expect(mapProjectorComment(bad).url).toBe("");
  });

  test("non-ref stored value without a stored url yields no url, never a throw", () => {
    const bad = row({ ref: "not-a-ref" });
    expect(() => mapProjectorComment(bad)).not.toThrow();
    expect(mapProjectorComment(bad).url).toBe("");
  });

  test("a stored url wins even when the ref is invalid", () => {
    const stored = row({ ref: "not-a-ref", url: "https://github.com/example/x" });
    expect(mapProjectorComment(stored).url).toBe("https://github.com/example/x");
  });
});

const FEED = {
  version: 1,
  generatedAt: new Date().toISOString(),
  quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
  attention: [],
};

async function driveBoard(externalLinks: unknown[]) {
  const dir = await mkdtemp(join(tmpdir(), "pm-ref-contract-"));
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(feedFile, JSON.stringify(FEED));
  const projectsFile = join(dir, "kaneo-projects.json");
  await writeFile(
    projectsFile,
    JSON.stringify({
      projects: [{ slug: "code", projectId: "p1", repository: "example-org/example-repo", installationId: 1 }],
    }),
  );
  const realFetch = globalThis.fetch;
  globalThis.fetch = (async (input: unknown) => {
    const url = String(input);
    const json = (body: unknown, headers: Record<string, string> = {}) =>
      new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json", ...headers } });
    if (url.includes("/api/auth/sign-in/email")) return json({}, { "set-cookie": "session=test; Path=/" });
    if (url.includes("/api/workspace")) return json([{ id: "ws1" }]);
    if (url.includes("/api/project?")) return json([{ id: "p1", slug: "code", name: "Code" }]);
    if (url.includes("/api/task/tasks/")) {
      return json({
        data: {
          columns: [
            {
              id: "in-progress",
              name: "In Progress",
              tasks: [{ id: "t-p1", number: 7, title: "Work in p1", status: "in-progress", externalLinks }],
            },
          ],
        },
        pagination: { totalPages: 1 },
      });
    }
    return new Response("not found", { status: 404 });
  }) as typeof fetch;
  try {
    const cfg = loadConfig({
      ...Bun.env,
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_PROJECTOR_URL: "off",
      PM_KANEO_PROJECTS_FILE: projectsFile,
      KANEO_URL: "http://kaneo.test",
      KANEO_ADMIN_EMAIL: "test@example.com",
      KANEO_ADMIN_PASSWORD: "test-password",
    });
    return (await liveSource(cfg).board()).filter((b) => b.source === "kaneo");
  } finally {
    globalThis.fetch = realFetch;
  }
}

describe("Kaneo board link minting (#6243)", () => {
  test("only valid refs are minted: unsafe-integer and non-numeric externalIds drop, the card survives", async () => {
    const board = await driveBoard([
      { resourceType: "issue", externalId: "42", url: "https://github.com/example-org/example-repo/issues/42" },
      {
        resourceType: "issue",
        externalId: "99999999999999999999",
        url: "https://github.com/example-org/example-repo/issues/99999999999999999999",
      },
      { resourceType: "issue", externalId: "abc", url: "https://github.com/example-org/example-repo/issues/abc" },
    ]);
    const card = board.find((b) => b.title === "Work in p1");
    expect(card).toBeDefined();
    expect(card?.links.map((l) => l.ref)).toEqual(["example-org/example-repo#42"]);
  });
});

// ---- Salvaged from PR #7642: sessionRecordLine alignment (disposition 5) ----

function rec(taskRefs: string[]): SessionRecord {
  return {
    id: "ses_phase4",
    source: "claude-code",
    label: "Phase 4 probe",
    agentId: "claude-code",
    agentLabel: "Muse",
    parentId: null,
    status: "succeeded",
    startedAt: "2026-11-08T00:00:00Z",
    lastSeenAt: "2026-11-08T01:00:00Z",
    taskRefs: taskRefs as RefKey[],
  } as SessionRecord;
}

const GOOD = "developing-today/code#6243";
const BAD_UNSAFE = "developing-today/code#99999999999999999999";
const BAD_SHAPE = "goal:keep-the-four-repos-healthy";

describe("sessionRecordLine renders invalid stored refs as plain text (#6243)", () => {
  test("unsafe-integer ref: no throw, ref visible as plain text, no anchor", () => {
    const html = sessionRecordLine(rec([BAD_UNSAFE]));
    expect(html).toContain(BAD_UNSAFE);
    expect(html).not.toContain(`href="https://github.com/developing-today/code/issues/99999999999999999999"`);
  });
  test("non-GitHub ref: no throw, ref visible as plain text", () => {
    const html = sessionRecordLine(rec([BAD_SHAPE]));
    expect(html).toContain(BAD_SHAPE);
    expect(html).not.toContain('<a href="https://github.com/goal');
  });
  test("valid ref control: anchor unchanged", () => {
    const html = sessionRecordLine(rec([GOOD]));
    expect(html).toContain(`<a href="https://github.com/developing-today/code/issues/6243">${GOOD}</a>`);
  });
  test("mixed refs: valid anchors, invalid is plain text, session survives", () => {
    const html = sessionRecordLine(rec([GOOD, BAD_UNSAFE]));
    expect(html).toContain(`<a href="https://github.com/developing-today/code/issues/6243">${GOOD}</a>`);
    expect(html).toContain(BAD_UNSAFE);
  });
});

describe("the internal ref assertion stays total-checked (#6242 rule 1)", () => {
  test("isValidRef is the shared rule", () => {
    expect(isValidRef(GOOD)).toBe(true);
    expect(isValidRef(BAD_UNSAFE)).toBe(false);
    expect(isValidRef(BAD_SHAPE)).toBe(false);
  });
  test("parseRef and refUrl still throw on invalid refs", () => {
    expect(() => parseRef(BAD_UNSAFE as RefKey)).toThrow();
    expect(() => refUrl(BAD_SHAPE as RefKey)).toThrow();
  });
});
