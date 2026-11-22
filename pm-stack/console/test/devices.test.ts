// A phone linked to read the summary its home-screen widget shows.
//
// Oracles: the challenge is computed here with node's own SHA-256, not by
// the code under test; the store is a real file read back by the test, so
// "never keeps a token" is checked against the bytes on disk; and the
// HTTP checks run through the same `handle` the service uses, with a
// sign-in that always refuses, so anything answered was answered without
// it.
import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { actions, perform } from "../src/actions.ts";
import { AuthError } from "../src/auth.ts";
import { readConfig } from "../src/config.ts";
import { challengeOf, Devices, Limiter, summary } from "../src/devices.ts";
import { Docs } from "../src/docs.ts";
import { OpError, Ops } from "../src/ops.ts";
import { type Deps, handle } from "../src/server.ts";
import { Work } from "../src/work.ts";

const CFG = readConfig({ HOME: "/h" });
const VERIFIER = "v".repeat(43);
const OTHER = "w".repeat(43);
const hashOf = (text: string) => createHash("sha256").update(text).digest("base64url");

function store(now = { t: 1_800_000_000_000 }) {
  const dir = mkdtempSync(join(tmpdir(), "pm-devices-"));
  const file = join(dir, "devices.json");
  return {
    now,
    file,
    devices: new Devices(file, () => now.t),
    again: () => new Devices(file, () => now.t),
    bytes: () => readFileSync(file, "utf8"),
    done: () => rmSync(dir, { recursive: true, force: true }),
  };
}

describe("linking a phone", () => {
  test("the challenge is the SHA-256 of the verifier", () => {
    expect(challengeOf(VERIFIER)).toBe(hashOf(VERIFIER));
    expect(challengeOf(VERIFIER)).toHaveLength(43);
  });

  test("an approved phone gets a token once, and only for its own verifier", async () => {
    const s = store();
    try {
      // Nothing approved: no verifier is worth anything.
      expect(await s.devices.link(VERIFIER)).toBeNull();
      const { expires } = await s.devices.approve({ challenge: hashOf(VERIFIER), name: "  Pocket  " });
      expect(Date.parse(expires) - s.now.t).toBe(10 * 60_000);
      // Another phone's verifier does not match the approved challenge.
      expect(await s.devices.link(OTHER)).toBeNull();
      const linked = await s.devices.link(VERIFIER);
      expect(linked?.token).toMatch(/^[A-Za-z0-9_-]{43}$/);
      // The approval is spent.
      expect(await s.devices.link(VERIFIER)).toBeNull();
      expect(await s.devices.list()).toEqual([
        { id: linked?.id as string, name: "Pocket", createdAt: new Date(s.now.t).toISOString(), seenAt: null },
      ]);
      // The token is the phone's alone: the file holds neither it nor the verifier.
      const onDisk = s.bytes();
      expect(onDisk).not.toContain(linked?.token as string);
      expect(onDisk).not.toContain(VERIFIER);
      expect(onDisk).toContain(
        createHash("sha256")
          .update(linked?.token as string)
          .digest("hex"),
      );
      // A restart reads the same store: the token still works.
      expect((await s.again().check(linked?.token))?.id).toBe(linked?.id as string);
    } finally {
      s.done();
    }
  });

  test("an approval runs out, and malformed requests are refused", async () => {
    const s = store();
    try {
      await s.devices.approve({ challenge: hashOf(VERIFIER) });
      s.now.t += 10 * 60_000 + 1;
      expect(await s.devices.link(VERIFIER)).toBeNull();
      for (const challenge of ["", "short", `${hashOf(VERIFIER)}x`, "!".repeat(43), 42, null]) {
        await expect(s.devices.approve({ challenge })).rejects.toBeInstanceOf(OpError);
      }
      for (const verifier of ["", "x".repeat(42), "x".repeat(129), "é".repeat(43), 7, null, {}]) {
        expect(await s.devices.link(verifier)).toBeNull();
      }
    } finally {
      s.done();
    }
  });

  test("a token is checked by its hash, seen times are kept, and removing a phone ends it", async () => {
    const s = store();
    try {
      await s.devices.approve({ challenge: hashOf(VERIFIER) });
      const linked = await s.devices.link(VERIFIER);
      const token = linked?.token as string;
      for (const wrong of ["", "x".repeat(43), `${token}x`, token.slice(1), 5, null]) {
        expect(await s.devices.check(wrong)).toBeNull();
      }
      expect((await s.devices.check(token))?.seenAt).toBe(new Date(s.now.t).toISOString());
      // Seen again soon after: the time is not rewritten.
      const first = s.now.t;
      s.now.t += 60_000;
      expect((await s.devices.check(token))?.seenAt).toBe(new Date(first).toISOString());
      s.now.t += 10 * 60_000;
      expect((await s.devices.check(token))?.seenAt).toBe(new Date(s.now.t).toISOString());
      await expect(s.devices.remove({ id: "nope" })).rejects.toThrow(/not linked/);
      expect(await s.devices.remove({ id: linked?.id })).toEqual({ removed: "Android phone" });
      expect(await s.devices.check(token)).toBeNull();
      expect(await s.again().check(token)).toBeNull();
    } finally {
      s.done();
    }
  });

  test("no more than eight phones, and no more than four requests waiting", async () => {
    const s = store();
    try {
      for (let n = 0; n < 8; n++) {
        const verifier = String(n).repeat(43);
        await s.devices.approve({ challenge: hashOf(verifier) });
        expect(await s.devices.link(verifier)).not.toBeNull();
      }
      await expect(s.devices.approve({ challenge: hashOf(VERIFIER) })).rejects.toThrow(/8 phones/);
      const fresh = store();
      try {
        // Five requests approved: the oldest falls off, the newest four stand.
        const verifiers = ["a", "b", "c", "d", "e"].map((c) => c.repeat(43));
        for (const verifier of verifiers) await fresh.devices.approve({ challenge: hashOf(verifier) });
        expect(await fresh.devices.link(verifiers[0])).toBeNull();
        expect(await fresh.devices.link(verifiers[4])).not.toBeNull();
      } finally {
        fresh.done();
      }
    } finally {
      s.done();
    }
  });
});

describe("the limiter", () => {
  test("so many in a window, then none until the window turns", () => {
    const now = { t: 0 };
    const limit = new Limiter(3, 1000, () => now.t);
    expect([limit.allow(), limit.allow(), limit.allow(), limit.allow()]).toEqual([true, true, true, false]);
    now.t = 999;
    expect(limit.allow()).toBe(false);
    now.t = 1000;
    expect(limit.allow()).toBe(true);
  });
});

describe("the summary", () => {
  const sections = {
    board: {
      counts: { claimed: 4, queued: 2, blocked: 400 },
      now: [
        { title: "Fix the  broker\nreconnect", step: "Run the soak test" },
        { title: "Two", goal: "It passes" },
        { title: "Three" },
        { title: "Four" },
      ],
      next: [{ title: "N1" }, { title: "N2" }],
    },
    sessions: { moving: [{}, {}], waiting: [{}, {}] },
    census: { hands: [{}, {}, {}], reviews: [{}], other: [{}, {}, {}, {}, {}] },
    asks: [{ status: "answered" }, { status: "working" }, { status: "done" }, { status: "queued" }],
    questionCount: { total: 7, sets: 2 },
  };

  test("counts and the top lines, with the same needs sum as the app's badge", () => {
    expect(summary(sections, 1_800_000_000_000)).toEqual({
      v: 1,
      at: "2027-01-15T08:00:00.000Z",
      now: {
        count: 4,
        top: [
          { title: "Fix the broker reconnect", step: "Run the soak test" },
          { title: "Two", step: "It passes" },
          { title: "Three", step: "" },
        ],
      },
      next: { count: 2, top: ["N1", "N2"] },
      // 7 questions + (3 hands + 2 sessions waiting) + 1 review; "other" is the fleet's to decide.
      needs: { total: 13, questions: 7, hands: 5, reviews: 1 },
      asks: { answered: 1, waiting: 2 },
    });
  });

  test("sections that have never been read count as nothing, and long lines are cut", () => {
    expect(summary({}, 0)).toEqual({
      v: 1,
      at: null,
      now: { count: 0, top: [] },
      next: { count: 0, top: [] },
      needs: { total: 0, questions: 0, hands: 0, reviews: 0 },
      asks: { answered: 0, waiting: 0 },
    });
    const long = summary({ board: { counts: {}, now: [{ title: "x".repeat(300), step: "y".repeat(300) }] } }, 1) as {
      now: { top: { title: string; step: string }[] };
    };
    expect(long.now.top[0].title).toHaveLength(90);
    expect(long.now.top[0].title.endsWith("…")).toBe(true);
    expect(long.now.top[0].step).toHaveLength(120);
  });
});

describe("over HTTP, without the sign-in", () => {
  function setup() {
    const s = store();
    const reads: string[] = [];
    const docs = new Docs(
      Object.fromEntries(
        [
          "board",
          "sessions",
          "census",
          "asks",
          "questionCount",
          "questions",
          "prs",
          "priorities",
          "agents",
          "services",
          "more",
        ].map((name) => [
          name,
          {
            ttlMs: 10_000,
            load: async () => {
              reads.push(name);
              return name === "board"
                ? { counts: { claimed: 2, queued: 1 }, now: [{ title: "T" }], next: [] }
                : name === "questionCount"
                  ? { total: 3 }
                  : [];
            },
          },
        ]),
      ),
      () => s.now.t,
    );
    const audited: Record<string, unknown>[] = [];
    const ops = new Ops(CFG, async () => ({ code: 0, stdout: "", stderr: "" }));
    const deps: Deps = {
      cfg: CFG,
      ops,
      docs,
      devices: s.devices,
      deviceLimits: { link: new Limiter(3, 60_000, () => s.now.t), refused: new Limiter(2, 60_000, () => s.now.t) },
      actions: actions(ops, new Work(ops), undefined, s.devices),
      // Sign-in always refuses: whatever answers below answered without it.
      identify: async () => {
        throw new AuthError(401, "sign-in required");
      },
      audit: async (e) => {
        audited.push(e);
      },
    };
    const link = (verifier: unknown, method = "POST") =>
      handle(
        new Request("https://pm.security.cab/console/device/link", {
          method,
          body: method === "POST" ? JSON.stringify({ verifier }) : undefined,
          headers: { host: "pm.security.cab", "content-type": "application/json" },
        }),
        deps,
      );
    const read = (token?: string, path = "/console/device/summary") =>
      handle(
        new Request(`https://pm.security.cab${path}`, {
          headers: { host: "pm.security.cab", ...(token === undefined ? {} : { authorization: `Bearer ${token}` }) },
        }),
        deps,
      );
    return { ...s, deps, docs, reads, audited, link, read };
  }

  test("a phone links with its verifier and then reads the summary with its token", async () => {
    const w = setup();
    try {
      expect((await w.link(VERIFIER)).status).toBe(404);
      await perform(w.deps.actions as NonNullable<Deps["actions"]>, w.docs, "device.approve", {
        challenge: hashOf(VERIFIER),
        name: "Pocket",
      });
      const res = await w.link(VERIFIER);
      expect(res.status).toBe(200);
      expect(res.headers.get("cache-control")).toBe("no-store");
      const { token, id } = (await res.json()) as { token: string; id: string };
      // The record of the link names the device and carries no token.
      expect(w.audited).toEqual([expect.objectContaining({ action: "device.link", who: `device:${id}`, ok: true })]);
      expect(JSON.stringify(w.audited)).not.toContain(token);

      const got = await w.read(token);
      expect(got.status).toBe(200);
      expect(await got.json()).toEqual({
        v: 1,
        at: new Date(w.now.t).toISOString(),
        now: { count: 2, top: [{ title: "T", step: "" }] },
        next: { count: 1, top: [] },
        needs: { total: 3, questions: 3, hands: 0, reviews: 0 },
        asks: { answered: 0, waiting: 0 },
      });
      // Only the sections the summary is built from were read.
      expect([...w.reads].sort()).toEqual(["asks", "board", "census", "questionCount", "sessions"]);
      // Read again at once: served from memory.
      await w.read(token);
      expect(w.reads).toHaveLength(5);
      // A widget's read does not count as someone looking: the loop stays idle.
      w.now.t += 60_000;
      w.docs.tick();
      expect(w.reads).toHaveLength(5);
      // The next read, after the sections have aged, reads them again and waits for them.
      expect(((await (await w.read(token)).json()) as { at: string }).at).toBe(new Date(w.now.t).toISOString());
      expect(w.reads).toHaveLength(10);
    } finally {
      w.done();
    }
  });

  test("without a linked token nothing is said, and only these two paths skip the sign-in", async () => {
    const w = setup();
    try {
      expect((await w.read()).status).toBe(401);
      expect(await (await w.read("x".repeat(43))).json()).toEqual({
        error: "this phone is not linked",
        unlinked: true,
      });
      expect(w.reads).toEqual([]);
      // Everything beside them is behind the sign-in, which refuses here.
      for (const path of [
        "/console/device",
        "/console/device/",
        "/console/device/summary/",
        "/console/device/other",
        "/console/devices",
        "/console/api/devices",
      ]) {
        expect(await (await w.read(undefined, path)).json()).toEqual({ error: "sign-in required" });
      }
      expect(await (await w.link(VERIFIER, "GET")).json()).toEqual({ error: "sign-in required" });
      const post = await handle(
        new Request("https://pm.security.cab/console/device/summary", {
          method: "POST",
          body: "{}",
          headers: { host: "pm.security.cab" },
        }),
        w.deps,
      );
      expect(await post.json()).toEqual({ error: "sign-in required" });
    } finally {
      w.done();
    }
  });

  test("tries are limited, and wrong tokens never slow a linked phone", async () => {
    const w = setup();
    try {
      await perform(w.deps.actions as NonNullable<Deps["actions"]>, w.docs, "device.approve", {
        challenge: hashOf(VERIFIER),
      });
      const { token } = (await (await w.link(VERIFIER)).json()) as { token: string };
      // Two wrong tokens are answered; the third is turned away unread.
      expect([
        (await w.read("a".repeat(43))).status,
        (await w.read("b".repeat(43))).status,
        (await w.read("c".repeat(43))).status,
      ]).toEqual([401, 401, 429]);
      expect((await w.read(token)).status).toBe(200);
      // The link path took one try above; two more are answered, then none.
      expect([(await w.link(OTHER)).status, (await w.link(OTHER)).status, (await w.link(OTHER)).status]).toEqual([
        404, 404, 429,
      ]);
      w.now.t += 60_000;
      expect((await w.link(OTHER)).status).toBe(404);
      // A body that is too large or not JSON is refused before anything is hashed.
      const big = await handle(
        new Request("https://pm.security.cab/console/device/link", {
          method: "POST",
          body: JSON.stringify({ verifier: "x".repeat(2000) }),
          headers: { host: "pm.security.cab" },
        }),
        w.deps,
      );
      expect(big.status).toBe(413);
      const junk = await handle(
        new Request("https://pm.security.cab/console/device/link", {
          method: "POST",
          body: "not json",
          headers: { host: "pm.security.cab" },
        }),
        w.deps,
      );
      expect(junk.status).toBe(400);
    } finally {
      w.done();
    }
  });

  test("the owner sees linked phones by name and can remove one; removing is recorded by id", async () => {
    const w = setup();
    try {
      const table = w.deps.actions as NonNullable<Deps["actions"]>;
      await perform(table, w.docs, "device.approve", { challenge: hashOf(VERIFIER), name: "Pocket" });
      const { token, id } = (await (await w.link(VERIFIER)).json()) as { token: string; id: string };
      const asOwner = {
        ...w.deps,
        identify: async () => ({ via: "cloudflare-access", email: "drewrypope@gmail.com" }),
      };
      const listed = await handle(
        new Request("https://pm.security.cab/console/api/devices", { headers: { host: "pm.security.cab" } }),
        asOwner,
      );
      const body = await listed.text();
      expect(JSON.parse(body)).toEqual([
        { id, name: "Pocket", createdAt: new Date(w.now.t).toISOString(), seenAt: null },
      ]);
      expect(body).not.toContain(token);
      expect(table["device.remove"].audit({ id, extra: "x" })).toEqual({ id });
      expect(table["device.approve"].audit({ challenge: hashOf(VERIFIER), name: "Pocket" })).toEqual({
        name: "Pocket",
      });
      await perform(table, w.docs, "device.remove", { id });
      expect((await w.read(token)).status).toBe(401);
    } finally {
      w.done();
    }
  });
});
