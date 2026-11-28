// Slice 1 of #13492: a second machine enrolls under its own key with
// owner approval, reports under that key, and can be removed. Legacy
// single-key reports keep working.
// Oracle: the Machines answer lists each enrolled machine by the name it
// reported, a removed key's reports are refused, and the first machine's
// reports still succeed.
import { describe, expect, test } from "bun:test";
import { Edge, type KV } from "../edge/src/edge.ts";
import { readConfig } from "../src/config.ts";
import { clearJwksCache } from "../src/auth.ts";
import type { EdgeConfig } from "../src/edge-proto.ts";
import { keyId, newKeyPair, sign } from "../src/edge-proto.ts";
import { asThePlatformFetch, signer } from "./support.ts";

const HOST = "pm.example";
const OWNER = "owner@example.com";
const ISSUER = {
  name: "cloudflare-access",
  issuer: "https://team.cloudflareaccess.com",
  jwksUri: "https://team.cloudflareaccess.com/cdn-cgi/access/certs",
  audience: "aud-1",
  source: "access" as const,
};
const START = Date.parse("2026-11-22T12:00:00Z");

function makeKv() {
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
  return { kv, stored };
}

async function setup() {
  clearJwksCache();
  const now = { t: START };
  const clock = () => now.t;
  const access = await signer("k1");
  const outward = async (url: string | URL | Request) => {
    const u = String(url);
    if (u === ISSUER.jwksUri) return { ok: true, status: 200, json: async () => ({ keys: [access.jwk] }) };
    throw new Error(`unexpected fetch ${u}`);
  };
  const { kv } = makeKv();
  const freshEdge = () => new Edge({ kv, now: clock, fetchImpl: asThePlatformFetch(outward) as never });
  let edge = freshEdge();
  const cfg = readConfig({ HOME: "/h", PM_CONSOLE_OWNER_EMAILS: OWNER });
  cfg.issuers = [ISSUER];
  const edgeConfig: EdgeConfig = {
    owners: [OWNER],
    certNames: [],
    issuers: [ISSUER],
    repo: cfg.repo,
    queue: cfg.queue,
    androidPackage: cfg.androidPackage,
    androidFingerprints: [],
    androidApp: false,
    queueDbUrl: cfg.queueDbUrl,
    queueTokenName: cfg.queueTokenName,
    promptLabel: cfg.promptLabel,
    needsRepos: cfg.needsRepos,
  };
  const token = (claims: Record<string, unknown> = {}) =>
    access.sign({
      iss: ISSUER.issuer,
      aud: [ISSUER.audience],
      exp: now.t / 1000 + 600,
      iat: now.t / 1000 - 5,
      sub: "u1",
      email: OWNER,
      ...claims,
    });
  const browse = (path: string, init: { method?: string; body?: unknown; token?: string | null } = {}) =>
    edge.fetch(
      new Request(`https://${HOST}${path}`, {
        method: init.method ?? (init.body === undefined ? "GET" : "POST"),
        body: init.body === undefined ? undefined : JSON.stringify(init.body),
        headers: {
          ...(init.token === null ? {} : { "cf-access-jwt-assertion": init.token ?? "t" }),
          ...(init.body === undefined && init.method !== "DELETE"
            ? {}
            : { "content-type": "application/json", "x-pm-console": "1" }),
        },
      }),
    );
  const ownerBrowse = async (path: string, init: { method?: string; body?: unknown } = {}) =>
    browse(path, { ...init, token: await token() });
  return {
    get edge() {
      return edge;
    },
    restart: () => {
      edge = freshEdge();
    },
    kv,
    now,
    clock,
    edgeConfig,
    browse,
    ownerBrowse,
    token,
  };
}

async function syncAs(
  s: Awaited<ReturnType<typeof setup>>,
  pair: { publicJwk: { kty: "EC"; crv: "P-256"; x: string; y: string }; privateJwk: JsonWebKey },
  name: string,
  at: number,
  options: { actions?: string[]; legacy?: boolean; acks?: { id: string; ok: boolean }[] } = {},
) {
  const kid = await keyId(pair.publicJwk);
  const body = new TextEncoder().encode(
    JSON.stringify({
      v: 1,
      at,
      ...(options.legacy ? {} : { machine: { name, actions: options.actions ?? ["task.move"], sections: ["board"] } }),
      config: s.edgeConfig,
      sections: [],
      have: {},
      acks: options.acks ?? [],
      phones: { devices: [], removed: [] },
    }),
  );
  return {
    kid,
    res: await s.edge.fetch(
      new Request(`https://${HOST}/console/edge/sync`, {
        method: "POST",
        headers: { "x-pm-key": kid, "x-pm-signature": await sign(pair.privateJwk, body) },
        body,
      }),
    ),
  };
}

describe("key-bound multi-machine routing (#13492 slice 2)", () => {
  test("distinct capabilities, duplicate names, restart, wrong-key ack and legacy reports", async () => {
    // Oracle: the selected signing key alone receives and settles work;
    // no directive may relay a second machine's action to the first tunnel.
    const s = await setup();
    const first = await newKeyPair();
    const second = await newKeyPair();
    const firstKid = await keyId(first.publicJwk);
    const secondKid = await keyId(second.publicJwk);
    await s.edge.fetch(
      new Request("https://pm.internal/internal/key", {
        method: "POST",
        body: JSON.stringify({ key: first.publicJwk }),
      }),
    );
    // The first report supplies Access/owner configuration before an
    // authenticated owner can mint the second machine's enrollment code.
    s.now.t += 1000;
    expect((await syncAs(s, first, "same-name", s.now.t)).res.status).toBe(200);
    const { code } = (await (await s.ownerBrowse("/console/api/machines/enroll-code", { body: {} })).json()) as {
      code: string;
    };
    expect(
      (
        await s.edge.fetch(
          new Request(`https://${HOST}/console/edge/enroll`, {
            method: "POST",
            body: JSON.stringify({ key: second.publicJwk, code }),
          }),
        )
      ).status,
    ).toBe(200);
    const sync = async (
      pair: typeof first,
      actions: string[],
      extra: { legacy?: boolean; acks?: { id: string; ok: boolean }[] } = {},
    ) => {
      s.now.t += 1000;
      return (await syncAs(s, pair, "same-name", s.now.t, { actions, ...extra })).res;
    };
    await sync(first, ["task.move"]);
    await sync(second, ["issue.comment"]);
    const act = (id: string, kind: string, args: unknown) =>
      s.ownerBrowse("/console/api/act", { body: { id, kind, args } });
    const relayed = await act("00000000-0000-4000-8000-000000000001", "task.move", { task: 21016, to: "first" });
    expect(relayed.headers.get("x-pm-next")).toBe("relay");
    expect(await relayed.json()).toMatchObject({ forKid: firstKid });
    const held = await act("00000000-0000-4000-8000-000000000002", "issue.comment", {
      issue: 301,
      text: "second only",
    });
    expect(held.headers.get("x-pm-next")).toBeNull();
    expect(await held.json()).toMatchObject({ held: true });
    s.restart();
    const wrong = await sync(first, ["task.move", "issue.comment"], {
      acks: [{ id: "00000000-0000-4000-8000-000000000002", ok: true }],
    });
    expect(await wrong.json()).toMatchObject({ commands: [] });
    const machines = (await (await s.ownerBrowse("/console/api/machines")).json()) as {
      held: { id: string; forKid: string }[];
    };
    expect(machines.held).toContainEqual(
      expect.objectContaining({ id: "00000000-0000-4000-8000-000000000002", forKid: secondKid }),
    );
    // Capability changes on the other key and identical names do not steal it.
    const delivered = await sync(second, ["issue.comment"]);
    expect(await delivered.json()).toMatchObject({
      commands: [{ id: "00000000-0000-4000-8000-000000000002", forKid: secondKid }],
    });
    // A retry with the same action id cannot relay to the first machine,
    // even after that machine starts advertising the same capability.
    const retry = await act("00000000-0000-4000-8000-000000000002", "issue.comment", {
      issue: 301,
      text: "second only",
    });
    expect(retry.headers.get("x-pm-next")).toBeNull();
    expect(await retry.json()).toMatchObject({ held: true });
    expect(await (await sync(first, ["task.move", "issue.comment"])).json()).toMatchObject({ commands: [] });
    s.restart();
    // Same signed key can acknowledge even with an older, nameless report.
    await sync(second, [], { legacy: true, acks: [{ id: "00000000-0000-4000-8000-000000000002", ok: true }] });
    expect(((await (await s.ownerBrowse("/console/api/machines")).json()) as { held: unknown[] }).held).toEqual([]);
    const settledRetry = await act("00000000-0000-4000-8000-000000000002", "issue.comment", {
      issue: 301,
      text: "second only",
    });
    expect(settledRetry.headers.get("x-pm-next")).toBeNull();
    expect(await settledRetry.json()).toMatchObject({ ok: true, again: true });
    await sync(second, ["issue.comment"]);
    const none = await act("none-act", "pr.merge", { pr: 7 });
    expect(none.status).toBe(503);
    expect(none.headers.get("x-pm-next")).toBeNull();
    expect(await none.json()).toMatchObject({ noneCapable: true });
    // Subject order crosses key boundaries: a newer action assigned to
    // the first machine cannot pass older work waiting for the second.
    await sync(first, ["task.move"]);
    const olderId = "00000000-0000-4000-8000-000000000004";
    const newerId = "00000000-0000-4000-8000-000000000005";
    expect(await (await act(olderId, "issue.comment", { issue: 302, text: "older" })).json()).toMatchObject({
      held: true,
    });
    await sync(first, ["task.move", "issue.comment"]);
    expect(await (await act(newerId, "issue.comment", { issue: 302, text: "newer" })).json()).toMatchObject({
      held: true,
    });
    expect(await (await sync(first, ["task.move", "issue.comment"])).json()).toMatchObject({ commands: [] });
    expect(await (await sync(second, ["issue.comment"])).json()).toMatchObject({ commands: [{ id: olderId }] });
    await sync(second, ["issue.comment"], { acks: [{ id: olderId, ok: true }] });
    // Retrying completed work while a newer subject action waits must
    // return its terminal answer, never enqueue it behind that action.
    expect(await (await act(olderId, "issue.comment", { issue: 302, text: "older" })).json()).toMatchObject({
      ok: true,
      again: true,
    });
    expect(await (await sync(first, ["task.move", "issue.comment"])).json()).toMatchObject({
      commands: [{ id: newerId }],
    });
    await sync(first, ["task.move"], { acks: [{ id: newerId, ok: true }] });
    // A second machine's fresh report does not revive a stale first tunnel.
    s.now.t += 121000;
    await sync(second, ["issue.comment"]);
    const stale = await act("00000000-0000-4000-8000-000000000003", "task.move", { task: 21017, to: "first" });
    expect(stale.headers.get("x-pm-next")).toBeNull();
    expect(await stale.json()).toMatchObject({ held: true });
    expect(await (await sync(second, ["issue.comment"])).json()).toMatchObject({ commands: [] });
    expect(await (await sync(first, ["task.move"])).json()).toMatchObject({
      commands: [{ id: "00000000-0000-4000-8000-000000000003", forKid: firstKid }],
    });
  });
});

describe("second machine enrollment (#13492 slice 1)", () => {
  test("owner-approved enroll, per-key reports, removal, legacy compat", async () => {
    const s = await setup();
    // First machine enrolls the legacy way (through the tunnel).
    const first = await newKeyPair();
    const firstKid = await keyId(first.publicJwk);
    const keyed = await s.edge.fetch(
      new Request("https://pm.internal/internal/key", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ key: first.publicJwk }),
      }),
    );
    expect(await keyed.json()).toEqual({ ok: true });
    s.now.t += 1000;
    expect((await syncAs(s, first, "laptop", s.now.t)).res.status).toBe(200);

    // Owner mints an enrollment code for the second machine.
    s.now.t += 1000;
    const codeRes = await s.ownerBrowse("/console/api/machines/enroll-code", { method: "POST", body: {} });
    expect(codeRes.status).toBe(200);
    const { code } = (await codeRes.json()) as { code: string };
    expect(typeof code).toBe("string");

    // Second machine enrolls with the code and its own public key.
    const second = await newKeyPair();
    const enrollRes = await s.edge.fetch(
      new Request(`https://${HOST}/console/edge/enroll`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ key: second.publicJwk, code }),
      }),
    );
    expect(enrollRes.status).toBe(200);

    // Both report under their own keys; the Machines screen names both.
    s.now.t += 1000;
    expect((await syncAs(s, second, "desktop", s.now.t)).res.status).toBe(200);
    s.now.t += 1000;
    expect((await syncAs(s, first, "laptop", s.now.t)).res.status).toBe(200);
    const machines = (await (await s.ownerBrowse("/console/api/machines")).json()) as {
      machines: { name: string }[];
    };
    expect(machines.machines.map((m) => m.name).sort()).toEqual(["desktop", "laptop"]);

    // Removal revokes: the removed key's reports are refused, the first still works.
    const secondKid = await keyId(second.publicJwk);
    const removeRes = await s.ownerBrowse(`/console/api/machines/${secondKid}`, { method: "DELETE" });
    expect(removeRes.status).toBe(200);
    s.now.t += 1000;
    // Removed: gone (410 names the removal; it is not a bad signature).
    expect((await syncAs(s, second, "desktop", s.now.t)).res.status).toBe(410);
    s.now.t += 1000;
    expect((await syncAs(s, first, "laptop", s.now.t)).res.status).toBe(200);
    const after = (await (await s.ownerBrowse("/console/api/machines")).json()) as {
      machines: { name: string }[];
    };
    expect(after.machines.map((m) => m.name)).toEqual(["laptop"]);
    expect(firstKid).not.toBe(secondKid);
  });
});
