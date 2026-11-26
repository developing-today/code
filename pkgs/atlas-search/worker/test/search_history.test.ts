/**
 * The search-history family of retained objects (#9140 slice 3): one
 * version per answered search in a named session, with the query text
 * kept only under a class that allows it.
 */
import { env } from "cloudflare:test";
import { fetchMock } from "./fetch-mock";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import worker, { type Env } from "../src/index";
import {
  recordSearch,
  resultRefs,
  searchHistoryDraft,
  searchSessionObjectId,
  validSession,
  type AnsweredSearch,
} from "../src/search_history";
import { DEFAULT_CLASSES, RetainedObjectStore } from "../src/retained_object";
import { EMBED_DIMS, EMBED_MODEL } from "../src/space";
import baseSql from "../migrations/0003_retained_objects.sql?raw";
import retirementSql from "../migrations/0004_retained_objects_retirement.sql?raw";
import modesSql from "../migrations/0005_retention_modes_and_reports.sql?raw";

const schemaSql = [baseSql, retirementSql, modesSql].join(";\n");

const db = (env as unknown as { HOT_CACHE: D1Database }).HOT_CACHE;
const store = new RetainedObjectStore(db);
const GATE = { Authorization: "Bearer test-gate" };

beforeAll(async () => {
  const statements = schemaSql
    .split("\n")
    .filter((line) => !line.trim().startsWith("--"))
    .join("\n")
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  for (const stmt of statements) await db.prepare(stmt).run();

  fetchMock.activate();
  fetchMock.disableNetConnect();
  const origin = fetchMock.get("https://service.test");
  origin
    .intercept({ path: "/v1/embed", method: "POST" })
    .reply(() => {
      const vector = new Array(EMBED_DIMS).fill(0);
      vector[0] = 1.0;
      return {
        statusCode: 200,
        data: JSON.stringify({ model: EMBED_MODEL, dims: EMBED_DIMS, vector }),
        responseOptions: { headers: { "Content-Type": "application/json" } },
      };
    })
    .persist();
  origin
    .intercept({ path: "/v1/search", method: "POST" })
    .reply(() => ({
      statusCode: 200,
      data: JSON.stringify({
        status: "complete",
        effective_ceiling: "personal",
        total_results: 2,
        job_id: "job-history",
        next_cursor: null,
        envelope: {},
        results: [
          { chunk_id: 42, text: "the claim protocol", score: 0.5 },
          { chunk_id: 7, text: "the lane brief", score: 0.25 },
        ],
      }),
      responseOptions: { headers: { "Content-Type": "application/json" } },
    }))
    .persist();
});

beforeEach(async () => {
  await db.prepare("DELETE FROM atlas_retained_objects").run();
  await db.prepare("DELETE FROM atlas_retention_classes").run();
  await store.seedDefaults();
});

const workerEnv = { ...(env as unknown as Env), RETAINED: db } as Env;

async function search(body: unknown, target: Env = workerEnv): Promise<Response> {
  return worker.fetch(
    new Request("https://worker.test/v1/search", {
      method: "POST",
      headers: { "Content-Type": "application/json", ...GATE },
      body: JSON.stringify(body),
    }),
    target,
  );
}

async function enable(allowsQueryText = false): Promise<void> {
  await store.setClass({ ...DEFAULT_CLASSES["search-history"], enabled: true, allowsQueryText });
}

const answered: AnsweredSearch = {
  session: "s-1",
  query: "who owns atlas",
  k: 5,
  key: "abc123",
  cache: "miss",
  result: { results: [{ chunk_id: 42 }, { chunk_id: "x9" }, { text: "no id" }, null] },
};

describe("search history", () => {
  it("validates sessions and extracts result refs", () => {
    for (const ok of ["s", "lane-8:2026.11.26_a", "x".repeat(128)]) expect(validSession(ok), ok).toBe(true);
    for (const bad of ["", "has space", "x".repeat(129), "a/b", 7, null]) expect(validSession(bad)).toBe(false);
    expect(resultRefs(answered.result)).toEqual(["chunk:42", "chunk:x9"]);
    expect(resultRefs(null)).toEqual([]);
    expect(resultRefs({ results: "no" })).toEqual([]);
  });

  it("builds a draft under the session's object", () => {
    const draft = searchHistoryDraft(answered, new Date("2026-11-26T03:30:00Z"));
    expect(draft).toEqual({
      objectId: searchSessionObjectId("s-1"),
      family: "search_history",
      class: "search-history",
      capturedAt: "2026-11-26T03:30:00.000Z",
      inputsRef: "query-key:abc123",
      payload: {
        session: "s-1",
        query: "who owns atlas",
        filters: { k: 5 },
        cache: "miss",
        result_refs: ["chunk:42", "chunk:x9"],
      },
    });
  });

  it("records nothing without a binding or while the class is disabled", async () => {
    expect(await recordSearch(undefined, answered)).toEqual({ recorded: false });
    expect(await recordSearch(db, answered)).toEqual({ recorded: false });
    expect(await store.history(searchSessionObjectId("s-1"))).toEqual([]);
  });

  it("keeps each answered search in a session, without its query text by default", async () => {
    await enable();
    const first = await search({ query: "history first query", k: 3, session: "s-1" });
    expect(first.status).toBe(200);
    const second = await search({ query: "history first query", k: 3, session: "s-1" });
    expect(second.status).toBe(200);
    expect(((await second.json()) as { cache: string }).cache).toBe("hit");
    const history = await store.history(searchSessionObjectId("s-1"));
    expect(history.map((v) => v.version)).toEqual([1, 2]);
    expect(history.map((v) => (v.payload as { cache: string }).cache)).toEqual(["miss", "hit"]);
    for (const version of history) {
      expect(version.payload).not.toHaveProperty("query");
      expect(version.payload).toMatchObject({
        session: "s-1",
        filters: { k: 3 },
        result_refs: ["chunk:42", "chunk:7"],
      });
      expect(version.inputsRef).toMatch(/^query-key:/);
    }
  });

  it("keeps the query text only when the class allows it", async () => {
    await enable(true);
    expect((await search({ query: "history open query", session: "s-2" })).status).toBe(200);
    const current = await store.current(searchSessionObjectId("s-2"));
    expect(current?.payload).toMatchObject({ query: "history open query" });
  });

  it("records nothing for a search without a session, and refuses a bad session", async () => {
    await enable();
    expect((await search({ query: "history no session" })).status).toBe(200);
    const { results } = await db.prepare("SELECT COUNT(*) AS n FROM atlas_retained_objects").all<{ n: number }>();
    expect(results[0]!.n).toBe(0);
    const refused = await search({ query: "history bad session", session: "has space" });
    expect(refused.status).toBe(400);
    expect(((await refused.json()) as { error: { code: string } }).error.code).toBe("bad_request");
  });

  it("answers the search even when the history store is unusable", async () => {
    const broken = {
      prepare() {
        throw new Error("d1 unavailable");
      },
    } as unknown as D1Database;
    const resp = await search({ query: "history broken store", session: "s-3" }, { ...workerEnv, RETAINED: broken });
    expect(resp.status).toBe(200);
  });
});
