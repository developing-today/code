import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import type { Comment, GhItem, Provenance, RefKey } from "../src/model.ts";
import { parseProjectItems } from "../src/projects.ts";
import { renderDiscuss, renderItem } from "../src/render.ts";
import { liveSource } from "../src/sources/live.ts";

// #1500: item detail pages — empty sections must say why. The Linked
// header carries tally + cause when resolution is partial; the comment
// sections name their fixture sourcing in place instead of a bare
// "no comments". Wording is keyed off the provenance strings only.

const PARTIAL_ITEMS = "live (project pool · fleet feed · 4m old) + fixtures for 2 ref(s) outside the pool";
const LIVE_ITEMS = "live (project pool · fleet feed · 4m old)";

const prov = (items: string, comments: string): Provenance =>
  ({
    board: "x",
    roadmap: "x",
    items,
    comments,
    quota: "x",
    usage: "x",
    attention: "x",
    orchestration: "x",
    dns: "x",
    registry: "x",
    aa: "x",
  }) as Provenance;

const gh = (ref: RefKey): GhItem => ({
  ref,
  kind: "issue",
  url: `https://github.com/${ref.replace("#", "/issues/")}`,
  title: `Item ${ref}`,
  state: "open",
  labels: [],
  author: "dezren39",
  updatedAt: "2026-10-29T00:00:00Z",
});

const LINKS = [
  { ref: "developing-today/code#101", relation: "tracks" as const },
  { ref: "developing-today/code#102", relation: "tracks" as const },
];

describe("renderItem linked section honesty (#1500)", () => {
  test("(a) partial linked resolution with fixture provenance -> header tally+cause + 'none resolved' row", () => {
    const html = renderItem("Subject", "subj", LINKS, new Map(), [], prov(PARTIAL_ITEMS, "live (projector)"));
    expect(html).toContain(
      "<h2>Linked issues / PRs — 2 linked · 0 resolved — resolution fell back to fixtures (projector off, #1199)</h2>",
    );
    expect(html).toContain("none resolved");
  });

  test("partial with one resolved -> tally counts the resolved ref", () => {
    const items = new Map<RefKey, GhItem>([["developing-today/code#101", gh("developing-today/code#101")]]);
    const html = renderItem("Subject", "subj", LINKS, items, [], prov(PARTIAL_ITEMS, "live (projector)"));
    expect(html).toContain(
      "Linked issues / PRs — 2 linked · 1 resolved — resolution fell back to fixtures (projector off, #1199)",
    );
  });

  test("(b) fully resolved -> plain header, no tally, no cause", () => {
    const items = new Map<RefKey, GhItem>([
      ["developing-today/code#101", gh("developing-today/code#101")],
      ["developing-today/code#102", gh("developing-today/code#102")],
    ]);
    const html = renderItem("Subject", "subj", LINKS, items, [], prov(LIVE_ITEMS, "live (projector)"));
    expect(html).toContain("<h2>Linked issues / PRs</h2>");
    expect(html).not.toContain("linked ·");
    expect(html).not.toContain("resolution fell back to fixtures");
  });
});

describe("renderItem comment section honesty (#1500)", () => {
  test("(c) fixture comment facet -> banner + honest empty wording, never a bare 'no comments'", () => {
    const html = renderItem("Subject", "subj", LINKS, new Map(), [], prov(LIVE_ITEMS, "fixtures"));
    expect(html).toContain("The comment stream is fixture-sourced while the projector is off (#1199)");
    expect(html).toContain(
      "no comments loaded — the comment stream is fixture-sourced while the projector is off (#1199)",
    );
    expect(html).not.toContain(">no comments</td>");
  });

  test("(c2) fixture comment facet with comments present -> banner still shows, rows render", () => {
    const comments: Comment[] = [
      {
        id: "c1",
        ref: "developing-today/code#101",
        kind: "issue_comment",
        author: "dezren39",
        body: "a real fixture comment",
        createdAt: "2026-10-29T00:00:00Z",
        url: "https://github.com/developing-today/code/issues/101#issuecomment-1",
      },
    ];
    const html = renderItem("Subject", "subj", LINKS, new Map(), comments, prov(LIVE_ITEMS, "fixtures"));
    expect(html).toContain("The comment stream is fixture-sourced while the projector is off (#1199)");
    expect(html).toContain("a real fixture comment");
    expect(html).not.toContain("no comments loaded");
  });

  test("(d) live comment facet -> 'no comments', no banner", () => {
    const html = renderItem("Subject", "subj", LINKS, new Map(), [], prov(LIVE_ITEMS, "live (projector)"));
    expect(html).toContain(">no comments</td>");
    expect(html).not.toContain("fixture-sourced while the projector is off");
    expect(html).not.toContain("no comments loaded");
  });
});

describe("renderDiscuss fixture honesty (#1500)", () => {
  test("fixture comment facet -> banner + honest empty wording", () => {
    const html = renderDiscuss(
      "gh:developing-today/code#101",
      "Discuss",
      [],
      { identities: [] },
      prov(LIVE_ITEMS, "fixtures"),
    );
    expect(html).toContain("The comment stream is fixture-sourced while the projector is off (#1199)");
    expect(html).toContain(
      "no comments loaded — the comment stream is fixture-sourced while the projector is off (#1199)",
    );
  });

  test("live comment facet -> no banner, no fixture empty wording", () => {
    const html = renderDiscuss(
      "gh:developing-today/code#101",
      "Discuss",
      [],
      { identities: [] },
      prov(LIVE_ITEMS, "live (projector)"),
    );
    expect(html).not.toContain("fixture-sourced while the projector is off");
    expect(html).not.toContain("no comments loaded");
  });
});

describe("items facet state classification (#1500)", () => {
  const GQL = {
    data: {
      node: {
        title: "Developing Today",
        number: 4,
        url: "https://github.com/orgs/developing-today/projects/4",
        items: {
          totalCount: 1,
          pageInfo: { hasNextPage: false, endCursor: null },
          nodes: [
            {
              id: "PVTI_1",
              databaseId: 111,
              type: "ISSUE",
              content: {
                number: 508,
                title: "EPIC: Basement cluster",
                state: "OPEN",
                url: "https://github.com/dezren39/nix/issues/508",
                repository: { nameWithOwner: "dezren39/nix" },
                updatedAt: "2026-10-20T12:05:58Z",
                author: { login: "dezren39" },
              },
              fieldValues: { nodes: [{ name: "In Progress", field: { name: "Status" } }] },
            },
          ],
        },
      },
    },
  };

  async function liveCfg() {
    const dir = await mkdtemp(join(tmpdir(), "pm-1500-"));
    const feedFile = join(dir, "fleet-feed.json");
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        projects: parseProjectItems(GQL),
      }),
    );
    return loadConfig({ PM_AGG_SOURCE: "live", PM_AGG_FEED_FILE: feedFile, PM_PROJECTOR_URL: "off" });
  }

  test("mixed pool + fixture fallback pass classifies items facet as partial-fixtures", async () => {
    const src = liveSource(await liveCfg());
    await src.items(["dezren39/nix#508", "developing-today/code#752"]);
    const detail = src.provenanceDetail?.();
    expect(detail?.items.text).toContain("+ fixtures for 1 ref(s) outside the pool");
    expect(detail?.items.state).toBe("partial-fixtures");
  });

  test("pool-only pass classifies items facet as live", async () => {
    const src = liveSource(await liveCfg());
    await src.items(["dezren39/nix#508"]);
    expect(src.provenanceDetail?.().items.state).toBe("live");
  });
});
