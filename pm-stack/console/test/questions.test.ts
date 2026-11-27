// Questions discovery at the edge (#13491, slice 2): the edge reads
// the decision sets the decision-events Worker holds — GET /sets,
// then each set's GET /sets/<setId>/state — and serves them as the
// questionSets section, with no laptop report in the path.
//
// Oracle: the displayed discovery's rows are exactly the set IDs the
// Worker enumerated, in its order, each carrying the state the
// Worker returned for it; the section says plainly that question
// definitions and answers still need the laptop.
import { describe, expect, test } from "bun:test";
import { Edge, type KV } from "../edge/src/edge.ts";
import { readQuestionSets } from "../edge/src/questions.ts";

const SETS = [
  {
    setId: "set-a",
    events: 3,
    firstTs: "2026-11-26T10:00:00Z",
    lastTs: "2026-11-26T12:00:00Z",
    lastEventId: "e3",
    lastType: "answer",
  },
  {
    setId: "set-b",
    events: 1,
    firstTs: "2026-11-26T11:00:00Z",
    lastTs: "2026-11-26T11:00:00Z",
    lastEventId: "e9",
    lastType: "created",
  },
];
const STATES: Record<string, unknown> = {
  "set-a": {
    setId: "set-a",
    d1Events: 3,
    object: {
      setId: "set-a",
      fannedOut: 3,
      lastEventId: "e3",
      lastType: "answer",
      lastTs: "2026-11-26T12:00:00Z",
      updatedAt: "2026-11-26T12:00:01Z",
      subscribers: 1,
    },
  },
  "set-b": {
    setId: "set-b",
    d1Events: 1,
    object: {
      setId: "set-b",
      fannedOut: 1,
      lastEventId: "e9",
      lastType: "created",
      lastTs: "2026-11-26T11:00:00Z",
      updatedAt: "2026-11-26T11:00:01Z",
      subscribers: 0,
    },
  },
};

/** The decision-events Worker, fixture-backed: it records how it was
 * asked (the secret must travel as a Bearer token) and answers the
 * enumeration and per-set state above. `failState` names a set whose
 * state read fails; `failList` fails the enumeration itself. */
function workerFetch(options: { failState?: string; failList?: boolean } = {}) {
  const asked: { url: string; authorization?: string }[] = [];
  const fetchImpl = (async (url: string, init?: { headers?: Record<string, string> }) => {
    asked.push({ url, authorization: init?.headers?.Authorization });
    if (url === "https://decision-events.test/sets") {
      if (options.failList) return { ok: false, status: 401, json: async () => ({ error: "unauthorized" }) };
      return { ok: true, status: 200, json: async () => ({ count: SETS.length, sets: SETS }) };
    }
    const match = /^https:\/\/decision-events\.test\/sets\/([^/]+)\/state$/.exec(url);
    if (match) {
      const setId = decodeURIComponent(match[1] as string);
      if (setId === options.failState) return { ok: false, status: 500, json: async () => ({}) };
      return { ok: true, status: 200, json: async () => STATES[setId] };
    }
    throw new Error(`unexpected fetch ${url}`);
  }) as never;
  return { asked, fetchImpl };
}

const SOURCE = { url: "https://decision-events.test", secret: "de-test-secret" };

describe("readQuestionSets", () => {
  test("the discovery matches the Worker's sets and their state, in its order", async () => {
    const { asked, fetchImpl } = workerFetch();
    const found = await readQuestionSets(SOURCE, fetchImpl);
    expect(found.source).toBe("decision-events");
    expect(found.laptopRequired).toBe(false);
    // Definitions and the answer path are not this section's to give.
    expect(found.definitions).toBe("unavailable");
    expect(found.note).toContain("still need the laptop");
    expect(found.sets.map((s) => s.setId)).toEqual(["set-a", "set-b"]);
    expect(found.sets[0]).toMatchObject({ events: 3, lastType: "answer", lastEventId: "e3" });
    expect(found.sets[0]?.state).toEqual({
      fannedOut: 3,
      lastEventId: "e3",
      lastType: "answer",
      lastTs: "2026-11-26T12:00:00Z",
      updatedAt: "2026-11-26T12:00:01Z",
      subscribers: 1,
      d1Events: 3,
    });
    expect(found.sets[1]?.state?.fannedOut).toBe(1);
    // Every call carried the secret as a Bearer token.
    expect(asked.map((a) => a.authorization)).toEqual([
      "Bearer de-test-secret",
      "Bearer de-test-secret",
      "Bearer de-test-secret",
    ]);
  });

  test("one set's failed state read keeps the set listed, with the reason", async () => {
    const { fetchImpl } = workerFetch({ failState: "set-b" });
    const found = await readQuestionSets(SOURCE, fetchImpl);
    expect(found.sets.map((s) => s.setId)).toEqual(["set-a", "set-b"]);
    expect(found.sets[1]?.state).toBeNull();
    expect(found.sets[1]?.stateError).toContain("500");
  });

  test("a failed enumeration fails the read: there is no discovery without the list", async () => {
    const { fetchImpl } = workerFetch({ failList: true });
    await expect(readQuestionSets(SOURCE, fetchImpl)).rejects.toThrow(/answered 401/);
  });
});

describe("the edge, with the laptop away", () => {
  test("the questionSets section is read from the Worker, not from a machine report", async () => {
    const { asked, fetchImpl } = workerFetch();
    const stored = new Map<string, unknown>();
    // A machine said who may sign in once (its config), and nothing
    // more: no section of any kind was ever reported by a machine.
    stored.set("cfg", {
      owners: ["owner@example.com"],
      certNames: [],
      issuers: [],
      repo: "developing-today/code",
      queue: "repo-health-code-pilot",
      androidPackage: "today.developing.pmconsole",
      androidFingerprints: [],
      androidApp: false,
      queueDbUrl: "https://queue.test",
      queueTokenName: "queue-token",
      promptLabel: "user/prompt",
      needsRepos: [],
    });
    const kv: KV = {
      get: async (key) => structuredClone(stored.get(key)),
      put: async (key, value) => {
        stored.set(key, structuredClone(value));
      },
      delete: async (key) => {
        stored.delete(key);
      },
    };
    const edge = new Edge({
      kv,
      now: () => Date.parse("2026-11-26T13:00:00Z"),
      fetchImpl: fetchImpl as never,
      direct: {
        enabled: true,
        githubToken: "gh-test",
        queueToken: "queue-test",
        decisionEvents: SOURCE,
        fetchImpl: fetchImpl as never,
      },
    });
    await edge.docs.refresh("questionSets");
    const found = edge.docs.good("questionSets") as { sets: { setId: string }[] } | null;
    // The oracle: the section the app draws is the Worker's sets,
    // IDs and state as it returned them, with the laptop never asked.
    expect(found?.sets.map((s) => s.setId)).toEqual(["set-a", "set-b"]);
    expect(asked.map((a) => a.url)).toEqual([
      "https://decision-events.test/sets",
      "https://decision-events.test/sets/set-a/state",
      "https://decision-events.test/sets/set-b/state",
    ]);
    const health = edge.docs.snapshot().find((s) => s.name === "questionSets");
    expect(health?.error).toBeNull();
  });
});
