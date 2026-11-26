// A second machine enrolled under its own key (#13492 slice 1).
//
// The edge is driven directly: the first machine's key is learned the
// way it is today (through the tunnel, /internal/key), a second
// machine asks to be enrolled with an ask signed by its own key, the
// owner approves or turns it away from the Machines screen's actions,
// and its reports are taken under that key — kept apart from the first
// machine's copies, and said to be its in the Machines answer.
//
// Oracles: storage keys (each machine's copies live under its own
// prefix, the first machine's under the keys they have always had),
// the Machines answer's attribution, and refusal after removal.
import { describe, expect, test } from "bun:test";
import { Edge, type KV } from "../edge/src/edge.ts";
import { type EdgeConfig, keyId, type MachinesAnswer, newKeyPair, sign } from "../src/edge-proto.ts";
import { signer } from "./support.ts";

const HOST = "pm.example";
const OWNER = "owner@example.com";
const ISSUER = {
  name: "cloudflare-access",
  issuer: "https://team.cloudflareaccess.com",
  jwksUri: "https://team.cloudflareaccess.com/cdn-cgi/access/certs",
  audience: "aud-1",
  source: "access" as const,
};
const START = Date.parse("2026-11-26T12:00:00Z");
const CONFIG: EdgeConfig = {
  owners: [OWNER],
  certNames: [],
  issuers: [ISSUER],
  repo: "developing-today/code",
  queue: "repo-health",
  androidPackage: "today.developing.pm",
  androidFingerprints: [],
  androidApp: false,
  queueDbUrl: "https://queue.example/db",
  queueTokenName: "queue-token",
  promptLabel: "prompt",
  needsRepos: [],
};

async function setup() {
  const now = { t: START };
  const clock = () => now.t;
  const access = await signer("k1");
  const outward = async (url: string | URL | Request) => {
    if (String(url) === ISSUER.jwksUri) return { ok: true, status: 200, json: async () => ({ keys: [access.jwk] }) };
    throw new Error(`unexpected fetch ${url}`);
  };
  const stored = new Map<string, unknown>();
  const kv: KV = {
    get: async (key) => structuredClone(stored.get(key)),
    put: async (key, value) => {
      stored.set(key, structuredClone(value));
    },
    delete: async (key) => {
      stored.delete(key);
    },
  };
  const edge = new Edge({ kv, now: clock, fetchImpl: outward as never, setAlarm: async () => {} });
  const token = () =>
    access.sign({
      iss: ISSUER.issuer,
      aud: [ISSUER.audience],
      exp: now.t / 1000 + 600,
      iat: now.t / 1000 - 5,
      sub: "u1",
      email: OWNER,
    });
  const call = (path: string, init: RequestInit = {}) =>
    edge.fetch(new Request(`https://${HOST}${path}`, init));
  const ownerCall = async (path: string, body?: unknown) =>
    call(path, {
      method: body === undefined ? "GET" : "POST",
      headers: {
        "cf-access-jwt-assertion": await token(),
        ...(body === undefined ? {} : { "content-type": "application/json", "x-pm-console": "1" }),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
  const act = async (kind: string, args: Record<string, unknown>) => ownerCall("/console/api/act", { kind, args });
  const machines = async (): Promise<MachinesAnswer> => {
    const res = await ownerCall("/console/api/machines");
    expect(res.status).toBe(200);
    return (await res.json()) as MachinesAnswer;
  };

  type Pair = Awaited<ReturnType<typeof newKeyPair>>;
  const learnKey = async (pair: Pair) => {
    const res = await call("/internal/key", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ key: pair.publicJwk }),
    });
    expect(res.status).toBe(200);
  };
  const syncAs = async (
    pair: Pair,
    opts: { at: number; board?: unknown; machine?: unknown; fetched?: boolean },
  ): Promise<Response> => {
    const kid = await keyId(pair.publicJwk);
    const board = opts.board ?? { counts: { claimed: 1, queued: 0 }, now: [], next: [], blocked: [] };
    const body = new TextEncoder().encode(
      JSON.stringify({
        v: 1,
        at: opts.at,
        ...(opts.machine === undefined ? {} : { machine: opts.machine }),
        config: CONFIG,
        sections: [{ name: "board", at: opts.at, hash: `h-${opts.at}`, data: board }],
        have: { board: { hash: `h-${opts.at}`, at: opts.at } },
        acks: [],
        phones: { devices: [], removed: [] },
      }),
    );
    return call("/console/edge/sync", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-pm-key": kid,
        "x-pm-signature": await sign(pair.privateJwk, body),
        ...(opts.fetched ? { "x-pm-key-fetched": "1" } : {}),
      },
      body,
    });
  };
  const enrollAs = async (pair: Pair, opts: { name?: string; signWith?: Pair } = {}): Promise<Response> => {
    const body = new TextEncoder().encode(
      JSON.stringify({
        v: 1,
        at: now.t,
        key: pair.publicJwk,
        machine: { name: opts.name ?? "Second", actions: ["issue.comment"], sections: ["board"] },
      }),
    );
    return call("/console/edge/enroll", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-pm-signature": await sign((opts.signWith ?? pair).privateJwk, body),
      },
      body,
    });
  };
  return { now, stored, learnKey, syncAs, enrollAs, act, machines };
}

describe("a second machine under its own key", () => {
  test("enroll, approve, report apart, remove", async () => {
    const s = await setup();
    const first = await newKeyPair();
    const second = await newKeyPair();
    const firstKid = await keyId(first.publicJwk);
    const secondKid = await keyId(second.publicJwk);
    await s.learnKey(first);

    // The first machine reports as it always has, with no word of
    // itself: a report older than the saying still works.
    s.now.t += 1000;
    const firstSync = await s.syncAs(first, { at: s.now.t, board: { counts: { claimed: 1 } } });
    expect(firstSync.status).toBe(200);

    // An ask signed by another key than the one it presents is refused.
    const forged = await s.enrollAs(second, { signWith: first });
    expect(forged.status).toBe(401);

    // The second machine asks; the ask is the owner's to answer, and
    // its reports are not taken meanwhile.
    const ask = await s.enrollAs(second);
    expect(ask.status).toBe(200);
    expect(((await ask.json()) as { state: string; code: string }).state).toBe("pending");
    expect(((await s.machines()).pending ?? []).map((p) => p.kid)).toEqual([secondKid]);
    s.now.t += 1000;
    const early = await s.syncAs(second, { at: s.now.t, fetched: true });
    expect(early.status).toBe(401);

    // The owner approves the code the machine shows.
    const approved = await s.act("machine.approve", { kid: secondKid });
    expect(approved.status).toBe(200);
    expect(((await approved.json()) as { result: { enrolled: string } }).result.enrolled).toBe("Second");
    expect((await s.machines()).pending ?? []).toEqual([]);
    // Approving what is no longer waiting is refused.
    expect((await s.act("machine.approve", { kid: secondKid })).status).toBe(404);

    // Its reports are taken under its own key, and kept apart: the
    // first machine's copy of the board is untouched by them.
    s.now.t += 1000;
    const secondSync = await s.syncAs(second, {
      at: s.now.t,
      board: { counts: { claimed: 99 } },
      machine: { name: "Second", actions: ["issue.comment"], sections: ["board"] },
    });
    expect(secondSync.status).toBe(200);
    expect(((await secondSync.json()) as { commands: unknown[] }).commands).toEqual([]);
    const firstCopy = JSON.stringify(s.stored.get("m:board:0") ?? "");
    expect(firstCopy).toContain('"claimed":1');
    expect(firstCopy).not.toContain("99");
    const secondCopy = JSON.stringify(s.stored.get(`mm:${secondKid}:m:board:0`) ?? "");
    expect(secondCopy).toContain('"claimed":99');

    // A report is not taken twice.
    const replay = await s.syncAs(second, {
      at: s.now.t,
      board: { counts: { claimed: 99 } },
      machine: { name: "Second", actions: ["issue.comment"], sections: ["board"] },
    });
    expect(replay.status).toBe(409);

    // The Machines answer names both, says which is enrolled, and says
    // which machine each section came from and whose is shown.
    const said = await s.machines();
    expect(said.machines).toHaveLength(2);
    expect(said.machines.find((m) => m.kid === secondKid)?.enrolled).toBe(true);
    expect(said.machines.filter((m) => m.enrolled)).toHaveLength(1);
    const boardFrom = said.sectionsFrom?.find((x) => x.name === "board");
    expect(boardFrom?.from.map((f) => f.kid).sort()).toEqual([firstKid, secondKid].sort());
    expect(boardFrom?.showing).toBe(firstKid);

    // Removal revokes the key and takes its copies with it.
    const removed = await s.act("machine.remove", { kid: secondKid });
    expect(removed.status).toBe(200);
    expect((await s.machines()).machines).toHaveLength(1);
    expect([...s.stored.keys()].some((k) => k.startsWith(`mm:${secondKid}:`))).toBe(false);
    s.now.t += 1000;
    const after = await s.syncAs(second, { at: s.now.t, fetched: true });
    expect(after.status).toBe(401);
    // The first machine still reports.
    s.now.t += 1000;
    expect((await s.syncAs(first, { at: s.now.t })).status).toBe(200);
  });

  test("an ask the owner turns away enrolls nothing", async () => {
    const s = await setup();
    const first = await newKeyPair();
    const third = await newKeyPair();
    const thirdKid = await keyId(third.publicJwk);
    await s.learnKey(first);
    // The first machine reports once, so the edge knows its config and
    // answers the owner at all.
    s.now.t += 1000;
    expect((await s.syncAs(first, { at: s.now.t })).status).toBe(200);
    const ask = await s.enrollAs(third, { name: "Third" });
    expect(ask.status).toBe(200);
    expect(((await ask.json()) as { state: string; code: string }).state).toBe("pending");
    expect(((await s.machines()).pending ?? []).map((p) => p.kid)).toEqual([thirdKid]);
    const rejected = await s.act("machine.reject", { kid: thirdKid });
    expect(rejected.status).toBe(200);
    expect((await s.machines()).pending ?? []).toEqual([]);
    expect((await s.act("machine.approve", { kid: thirdKid })).status).toBe(404);
    s.now.t += 1000;
    expect((await s.syncAs(third, { at: s.now.t, fetched: true })).status).toBe(401);
    // The first machine's own ask is answered as what it is.
    const own = await s.enrollAs(first);
    expect(((await own.json()) as { state: string }).state).toBe("primary");
  });
});
