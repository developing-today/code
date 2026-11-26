// The Stores section's live asking (#14254): each store in the list
// of record is asked over its HTTP API for whether it answers, its
// size, the rows in the tables the app reads, and its schema version.
//
// Oracles: the stand-in stores below are written out by hand — a
// pipeline answer in the Turso wire shape, with known numbers — and
// the read is checked against those numbers, not against anything
// the code under test computes. A store that does not answer (http
// error, a token that cannot be fetched, a token not held by name)
// must come back as not answering with the error said plainly and
// every live field null: never a stale success.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { storeSaid } from "../public/app.js";
import { readConfig } from "../src/config.ts";
import { Ops, type Fetch, type Run } from "../src/ops.ts";
import { storeAnswered, storeLiveStatements, storeNotAnswering } from "../src/store-live.ts";
import { storesList, type StoreRow } from "../src/stores.ts";

const file = readFileSync(join(import.meta.dir, "..", "..", "platform", "stores.json"), "utf8");
const listed = storesList(JSON.parse(file)).stores;
const queue = listed.find((s) => s.name === "fleet-queue") as StoreRow;
const platform = listed.find((s) => s.name === "fleet-platform") as StoreRow;

/** A pipeline answer in the wire shape: one result per statement. */
const answer = (values: (number | null)[][], refused: number[] = []) => ({
  results: values.map((row, i) =>
    refused.includes(i)
      ? { type: "error" }
      : {
          type: "ok",
          response: {
            result: {
              cols: [{ name: "n" }],
              rows: [row.map((v) => ({ type: v === null ? "null" : "integer", value: v === null ? null : String(v) }))],
            },
          },
        },
  ),
});

/** An Ops whose GitHub serves the list, whose secrets are the
 * stand-ins given, and whose fetch is answered per store address. */
function opsWith(secrets: Record<string, string>, fetchImpl: Fetch, fetched?: string[]) {
  const run: Run = async (argv) => {
    if (argv[1] === "gh" && argv[3] === "repos/developing-today/code/contents/pm-stack/platform/stores.json") {
      return { code: 0, stdout: JSON.stringify({ content: btoa(file) }), stderr: "" };
    }
    if (argv[1] === "secrets" && argv[2] === "get") {
      fetched?.push(argv[3]);
      const value = secrets[argv[3]];
      if (value === undefined) return { code: 1, stdout: "", stderr: "no such secret" };
      return { code: 0, stdout: `${value}\n`, stderr: "" };
    }
    return { code: 1, stdout: "", stderr: "not here" };
  };
  return new Ops(readConfig({ HOME: "/h" }), run, fetchImpl);
}

test("the asking: size, one count per table the app reads, and the schema version — reads only", () => {
  const statements = storeLiveStatements(platform);
  expect(statements).toHaveLength(2 + platform.tables.length);
  for (const st of statements) expect(st.sql).toMatch(/^\s*SELECT\b/i);
  for (const table of platform.tables) {
    expect(statements.some((st) => st.sql.includes(`"${table}"`))).toBe(true);
  }
  // A store whose list names no tables is still asked its size and version.
  expect(storeLiveStatements(queue)).toHaveLength(2);
});

test("an answer is read against the numbers the stand-in sent", () => {
  const got = storeAnswered(platform, answer([[4096], [3], [0], [12], [1], [2], [0], [5], [7]]).results.map((r) => r.response?.result));
  expect(got.answers).toBe(true);
  expect(got.error).toBeNull();
  expect(got.sizeBytes).toBe(4096);
  expect(Object.values(got.rows)).toEqual([3, 0, 12, 1, 2, 0, 5]);
  expect(got.liveSchemaVersion).toBe("7");
  // A statement the store refused leaves only its own count null.
  const partial = storeAnswered(platform, [answer([[4096]]).results[0]?.response?.result, undefined]);
  expect(partial.answers).toBe(true);
  expect(partial.sizeBytes).toBe(4096);
  expect(Object.values(partial.rows).every((n) => n === null)).toBe(true);
});

test("read through the fleet binary and a stand-in store: live rows for the answering stores, plain errors for the rest", async () => {
  const seen: { url: string; auth: string }[] = [];
  const fetchImpl: Fetch = async (url, init) => {
    seen.push({ url, auth: init.headers.Authorization });
    if (url.startsWith(platform.address)) {
      return { ok: true, status: 200, json: async () => answer([[8192], [1], [2], [3], [4], [5], [6], [7], [9]]) };
    }
    if (url.startsWith(queue.address)) return { ok: false, status: 503, json: async () => ({}) };
    // The mirror's token is never fetched (below), so it is never asked.
    throw new Error(`unexpected ask: ${url}`);
  };
  const fetched: string[] = [];
  const ops = opsWith(
    { "turso.fleet-queue-rw-token": "queue-token", "turso.fleet-platform-rw-token": "platform-token" },
    fetchImpl,
    fetched,
  );
  const got = await ops.platformStores();
  expect(got.malformed).toBe(0);
  expect(got.stores.map((s) => s.name)).toEqual(listed.map((s) => s.name));
  const byName = new Map(got.stores.map((s) => [s.name, s]));
  // The platform store answered, with the stand-in's own numbers.
  const livePlatform = byName.get("fleet-platform");
  expect(livePlatform).toMatchObject({ answers: true, sizeBytes: 8192, liveSchemaVersion: "9" });
  expect(Object.values(livePlatform?.rows ?? {})).toEqual([1, 2, 3, 4, 5, 6, 7]);
  // The queue store answered http 503: said as not answering, never as data.
  expect(byName.get("fleet-queue")).toMatchObject({
    answers: false,
    error: "the store answered http 503",
    sizeBytes: null,
    liveSchemaVersion: null,
  });
  // The mirror's token could not be fetched: said plainly, not asked.
  expect(byName.get("fleet-mirror")?.answers).toBe(false);
  expect(byName.get("fleet-mirror")?.error).toContain("its token could not be fetched");
  // The voice line's token is a Worker secret, not a name: not asked.
  expect(byName.get("voice-line-ledger")?.answers).toBe(false);
  expect(byName.get("voice-line-ledger")?.error).toContain("not held under a secret's name");
  // Tokens were fetched by name, and no token value leaves in the result.
  expect(new Set(fetched)).toEqual(new Set(["turso.fleet-queue-rw-token", "turso.fleet-platform-rw-token", "turso.fleet-mirror-rw-token"]));
  expect(JSON.stringify(got)).not.toContain("queue-token");
  expect(JSON.stringify(got)).not.toContain("platform-token");
  expect(seen.every((s) => s.auth.startsWith("Bearer "))).toBe(true);
});

test("when size is refused but counts and schema answer, the store stays answered with those numbers", async () => {
  const fetchImpl: Fetch = async (url) => {
    if (url.startsWith(platform.address)) {
      return { ok: true, status: 200, json: async () => answer([[8192], [1], [2], [3], [4], [5], [6], [7], [9]], [0]) };
    }
    return { ok: false, status: 503, json: async () => ({}) };
  };
  const ops = opsWith(
    { "turso.fleet-queue-rw-token": "queue-token", "turso.fleet-platform-rw-token": "platform-token" },
    fetchImpl,
  );
  const got = await ops.platformStores();
  const livePlatform = got.stores.find((s) => s.name === "fleet-platform");
  expect(livePlatform).toMatchObject({
    answers: true,
    error: null,
    sizeBytes: null,
    rows: { incidents: 1 },
    liveSchemaVersion: "9",
  });
  expect(Object.values(livePlatform?.rows ?? {})).toEqual([1, 2, 3, 4, 5, 6, 7]);
});

test("when the store refuses every measurement statement, it is said as not answering", async () => {
  const count = 2 + platform.tables.length;
  const fetchImpl: Fetch = async (url) => {
    if (url.startsWith(platform.address)) {
      const values = Array.from({ length: count }, () => [0]);
      const refused = Array.from({ length: count }, (_, i) => i);
      return { ok: true, status: 200, json: async () => answer(values, refused) };
    }
    return { ok: false, status: 503, json: async () => ({}) };
  };
  const ops = opsWith(
    { "turso.fleet-queue-rw-token": "queue-token", "turso.fleet-platform-rw-token": "platform-token" },
    fetchImpl,
  );
  const got = await ops.platformStores();
  const livePlatform = got.stores.find((s) => s.name === "fleet-platform");
  expect(livePlatform).toMatchObject({
    answers: false,
    error: "the store refused the asking",
    sizeBytes: null,
    liveSchemaVersion: null,
  });
  expect(Object.values(livePlatform?.rows ?? {}).every((n) => n === null)).toBe(true);
});

test("a rotated token is fetched again, once, and the store then answers", async () => {
  let asks = 0;
  const fetchImpl: Fetch = async (url) => {
    if (!url.startsWith(queue.address)) return { ok: false, status: 404, json: async () => ({}) };
    asks += 1;
    if (asks === 1) return { ok: false, status: 401, json: async () => ({}) };
    return { ok: true, status: 200, json: async () => answer([[1024], [0]]) };
  };
  const fetched: string[] = [];
  const one = JSON.stringify({ checked: "2026-11-25", stores: [JSON.parse(file).stores[0]] });
  const run: Run = async (argv) => {
    if (argv[1] === "gh") return { code: 0, stdout: JSON.stringify({ content: btoa(one) }), stderr: "" };
    if (argv[1] === "secrets") {
      fetched.push(argv[3]);
      return { code: 0, stdout: "rotated-token\n", stderr: "" };
    }
    return { code: 1, stdout: "", stderr: "not here" };
  };
  const ops = new Ops(readConfig({ HOME: "/h" }), run, fetchImpl);
  const got = await ops.platformStores();
  expect(got.stores).toHaveLength(1);
  expect(got.stores[0]).toMatchObject({ name: "fleet-queue", answers: true, sizeBytes: 1024, liveSchemaVersion: "0" });
  expect(asks).toBe(2);
  expect(fetched).toEqual(["turso.fleet-queue-rw-token", "turso.fleet-queue-rw-token"]);
});

test("the screen's words: an answering store shows its numbers; one that does not is said as not answering", () => {
  const liveStore = storeAnswered(queue, answer([[2048], [3]]).results.map((r) => r.response?.result));
  expect(storeSaid(liveStore).text).toContain("Answers. Size 2.0 KiB. Schema version 3.");
  const down = storeNotAnswering(queue, "the store answered http 503");
  const said = storeSaid(down);
  expect(said.tone).toBe("warn");
  expect(said.text).toContain("Not answering: the store answered http 503.");
  // Zero stores and many stores are both just the list, as given.
  expect(storesList({ stores: [] }).stores).toEqual([]);
  expect(storesList(JSON.parse(file)).stores.length).toBeGreaterThanOrEqual(4);
});
