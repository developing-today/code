// Notifications: the message a phone receives, and when one is owed.
//
// Oracles: the test plays the browser. It makes the subscription's key
// pair itself and decrypts what the server sends with the receiving
// half of RFC 8291, written here from the RFC and not shared with the
// code under test; it verifies the VAPID signature with the server's
// public key. "Owed" is decided by a table of before and after states.
import { describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Docs } from "../src/docs.ts";
import { fileStore } from "../src/file-store.ts";
import { changes, NOTHING_SEEN, Notifier, type Seen, SUSTAINED_MS } from "../src/notify.ts";
import { contact, encrypt, endpointMark, Push, validSubscription } from "../src/push.ts";
import { b64u, browser, raw } from "./support.ts";

describe("the message a phone receives", () => {
  test("only the subscribed browser can read it, and each one is sealed afresh", async () => {
    const b = await browser();
    const text = JSON.stringify({ title: "The fleet replied on #12501", body: "Done — and “quoted” ✓" });
    const one = await encrypt(b.subscription, new TextEncoder().encode(text));
    const two = await encrypt(b.subscription, new TextEncoder().encode(text));
    expect(await b.open(one)).toBe(text);
    expect(await b.open(two)).toBe(text);
    expect(b64u(one)).not.toBe(b64u(two)); // a new salt and a new sender key every time
    const other = await browser();
    await expect(other.open(one)).rejects.toThrow(); // not readable by anyone else
    const flipped = one.slice();
    flipped[flipped.length - 3] ^= 1;
    await expect(b.open(flipped)).rejects.toThrow(); // and not alterable on the way
  });

  test("a send is signed for the push service it goes to, and reaches every device", async () => {
    const dir = mkdtempSync(join(tmpdir(), "pm-push-"));
    try {
      const a = await browser();
      const m = await browser("updates.push.services.mozilla.com");
      const posts: { url: string; headers: Record<string, string>; body: Uint8Array }[] = [];
      const NOW = 1_800_000_000_000;
      const push = new Push(
        fileStore(join(dir, "push.json")),
        "mailto:owner@example.com",
        async (url, init) => (posts.push({ url, headers: init.headers, body: init.body }), { status: 201 }),
        () => NOW,
      );
      const status = await push.status();
      expect(raw(status.key).length).toBe(65); // an uncompressed P-256 point, as the browser expects
      expect(status.devices).toBe(0);
      await push.subscribe(a.subscription, "phone");
      await push.subscribe(a.subscription, "phone again"); // the same device is not counted twice
      expect((await push.subscribe(m.subscription, "desktop")).devices).toBe(2);
      expect(await push.send("replies", { title: "t", body: "b", url: "/console/#ask", tag: "x" })).toEqual({
        sent: 2,
        gone: 0,
        failed: 0,
      });
      expect(posts.map((p) => p.url)).toEqual([a.subscription.endpoint, m.subscription.endpoint]);
      expect(JSON.parse(await a.open(posts[0].body))).toEqual({
        title: "t",
        body: "b",
        url: "/console/#ask",
        tag: "x",
        class: "replies",
      });
      expect(JSON.parse(await m.open(posts[1].body)).class).toBe("replies");
      for (const p of posts) {
        expect(p.headers["Content-Encoding"]).toBe("aes128gcm");
        expect(Number(p.headers.TTL)).toBeGreaterThan(0);
        const [, jwt, key] = /^vapid t=([^,]+), k=(.+)$/.exec(p.headers.Authorization) ?? [];
        expect(key).toBe(status.key);
        const [head, body, sig] = jwt.split(".");
        expect(JSON.parse(Buffer.from(head, "base64url").toString())).toEqual({ typ: "JWT", alg: "ES256" });
        const claims = JSON.parse(Buffer.from(body, "base64url").toString());
        expect(claims.aud).toBe(new URL(p.url).origin); // signed for this push service and no other
        expect(claims.sub).toBe("mailto:owner@example.com");
        expect(claims.exp).toBe(NOW / 1000 + 12 * 3600); // within the 24 hours a push service allows
        const pub = await crypto.subtle.importKey("raw", raw(key), { name: "ECDSA", namedCurve: "P-256" }, false, [
          "verify",
        ]);
        expect(
          await crypto.subtle.verify(
            { name: "ECDSA", hash: "SHA-256" },
            pub,
            raw(sig),
            new TextEncoder().encode(`${head}.${body}`),
          ),
        ).toBe(true);
      }
      // The key pair and the devices survive a restart; the file is private.
      const again = new Push(fileStore(join(dir, "push.json")), "mailto:owner@example.com", async () => ({
        status: 201,
      }));
      // It still holds both devices, each known by its mark and not by its address.
      const marks = [await endpointMark(a.subscription.endpoint), await endpointMark(m.subscription.endpoint)];
      // And what came of the send above, which a restart does not lose.
      const last = {
        at: new Date(NOW).toISOString(),
        class: "replies" as const,
        sent: 2,
        gone: 0,
        failed: 0,
        refused: [],
      };
      expect(await again.status()).toEqual({ ...status, devices: 2, known: marks, last });
      expect(status.known).toEqual([]);
      expect(marks[0]).toMatch(/^[A-Za-z0-9_-]{16}$/);
      expect(marks[0]).not.toBe(marks[1]);
      expect(JSON.stringify(await again.status())).not.toContain("fcm.googleapis.com");
      expect((await again.status()).key).toBe(status.key);
      expect(JSON.parse(readFileSync(join(dir, "push.json"), "utf8")).vapid.privateJwk.d.length).toBeGreaterThan(20);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("a device that is gone is forgotten, a class switched off sends nothing, a failure is counted", async () => {
    const dir = mkdtempSync(join(tmpdir(), "pm-push-"));
    try {
      const a = await browser();
      const b = await browser("web.push.apple.com");
      let answer: (url: string) => number = (url: string) => (url === a.subscription.endpoint ? 410 : 201);
      let posts = 0;
      const push = new Push(
        fileStore(join(dir, "push.json")),
        "mailto:o@example.com",
        async (url) => (posts++, { status: answer(url) }),
      );
      await push.subscribe(a.subscription, "");
      await push.subscribe(b.subscription, "");
      expect(await push.send("needs", { title: "t", body: "b", url: "/" })).toEqual({ sent: 1, gone: 1, failed: 0 });
      expect((await push.status()).devices).toBe(1);
      expect(await push.setClass("needs", false)).toEqual({
        replies: true,
        needs: false,
        questions: true,
        trouble: true,
      });
      posts = 0;
      expect(await push.send("needs", { title: "t", body: "b", url: "/" })).toEqual({ sent: 0, gone: 0, failed: 0 });
      expect(posts).toBe(0);
      expect((await push.send("test", { title: "t", body: "b", url: "/" })).sent).toBe(1); // a test ignores the switches
      await push.setClass("needs", true);
      answer = () => 500;
      expect(await push.send("needs", { title: "t", body: "b", url: "/" })).toEqual({ sent: 0, gone: 0, failed: 1 });
      expect((await push.status()).devices).toBe(1); // a failure is not "gone"
      await expect(push.setClass("everything", true)).rejects.toThrow(/class/);
      expect((await push.unsubscribe(b.subscription.endpoint)).devices).toBe(0);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("what came of the last send is kept: how many it was taken for, and which push service refused", async () => {
    const dir = mkdtempSync(join(tmpdir(), "pm-push-"));
    try {
      const a = await browser(); // Google's
      const m = await browser("updates.push.services.mozilla.com");
      const w = await browser("web.push.apple.com");
      const MSG = { title: "t", body: "b", url: "/console/" };
      let answer: (url: string) => number = () => 201;
      let now = Date.parse("2026-11-23T05:00:00Z");
      const file = join(dir, "push.json");
      const make = () =>
        new Push(
          fileStore(file),
          "mailto:o@example.com",
          async (url) => {
            const status = answer(url);
            if (status < 0) throw new Error("no route to the push service");
            return { status };
          },
          () => now,
        );
      const push = make();
      expect((await push.status()).last).toBeNull(); // nothing has been sent
      await push.send("replies", MSG);
      expect((await push.status()).last).toBeNull(); // with nobody to send to, nothing was
      for (const b of [a, m, w]) await push.subscribe(b.subscription, "");
      await push.send("replies", MSG);
      expect((await push.status()).last).toEqual({
        at: "2026-11-23T05:00:00.000Z",
        class: "replies",
        sent: 3,
        gone: 0,
        failed: 0,
        refused: [],
      });
      // One service refuses, one cannot be reached, one says the device is gone.
      answer = (url) => (url === a.subscription.endpoint ? 403 : url === m.subscription.endpoint ? -1 : 410);
      now += 60_000;
      await push.send("test", MSG);
      const last = (await make().status()).last; // as a restarted service reads it back
      expect(last).toEqual({
        at: "2026-11-23T05:01:00.000Z",
        class: "test",
        sent: 0,
        gone: 1,
        failed: 2,
        refused: [
          { host: "fcm.googleapis.com", status: 403 },
          { host: "updates.push.services.mozilla.com", status: 0 },
        ],
      });
      // It names the service, never the device: no address, no key.
      for (const secret of ["abc123", a.subscription.keys.auth, a.subscription.keys.p256dh]) {
        expect(JSON.stringify(await push.status())).not.toContain(secret);
      }
      // A class that is switched off sends nothing, and the account stays as it was.
      await push.setClass("needs", false);
      now += 60_000;
      await push.send("needs", MSG);
      expect((await push.status()).last?.at).toBe("2026-11-23T05:01:00.000Z");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("the server only posts to push services", async () => {
    const good = (await browser()).subscription;
    expect(validSubscription(good)).toEqual(good);
    // What browsers really name. The second Google host is the one a
    // Chrome off the stable channel gives (a real Chrome 153 did, when
    // this was checked against Google's service); both are in Chromium's
    // push_messaging_constants.cc.
    for (const endpoint of [
      "https://fcm.googleapis.com/fcm/send/dQw:APA91b",
      "https://fcm.googleapis.com/wp/dQw:APA91b",
      "https://jmt17.google.com/fcm/send/dQw:APA91b",
      "https://updates.push.services.mozilla.com/wpush/v2/gAAAA",
      "https://web.push.apple.com/QGx",
      "https://wns2-by3p.notify.windows.com/w/?token=AwYAAAB",
    ]) {
      expect(validSubscription({ ...good, endpoint })?.endpoint).toBe(endpoint);
    }
    for (const endpoint of [
      "https://jmt17.google.com.evil.example/fcm/send/x",
      "https://x.jmt17.google.com/fcm/send/x",
      "https://jmt18.google.com/fcm/send/x",
      "https://www.google.com/fcm/send/x",
      "https://google.com/fcm/send/x",
      "http://fcm.googleapis.com/fcm/send/x", // not https
      "https://127.0.0.1:4870/api/alerts", // this machine
      "https://fcm.googleapis.com.evil.example/x",
      "https://evil.example/fcm.googleapis.com",
      "https://user:pw@fcm.googleapis.com/x",
      "https://queue-broker.security.cab/enqueue",
      "not a url",
    ]) {
      expect(validSubscription({ ...good, endpoint })).toBeNull();
    }
    expect(validSubscription({ ...good, keys: { ...good.keys, p256dh: "AAAA" } })).toBeNull();
    expect(validSubscription({ ...good, keys: { ...good.keys, auth: "AAAA" } })).toBeNull();
    expect(validSubscription({ endpoint: good.endpoint })).toBeNull();
    expect(validSubscription(null)).toBeNull();
    const dir = mkdtempSync(join(tmpdir(), "pm-push-"));
    try {
      await expect(
        new Push(fileStore(join(dir, "p.json")), "mailto:o@example.com").subscribe(
          { ...good, endpoint: "https://127.0.0.1/x" },
          "",
        ),
      ).rejects.toThrow(/push service/);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("when a notification is owed", () => {
  const reply = (at: string, you = false) => ({ you, at, text: "Done. #12279 is first in the queue." });
  const state = (over: Record<string, unknown> = {}) => ({
    asks: [
      { issue: 12501, replies: [reply("2026-11-22T06:00:00")] },
      { issue: 12502, replies: [] },
    ],
    census: { hands: [{ ref: "o/r#1", title: "t1", action: "Power on the host." }] },
    sessions: { waiting: [] },
    questionCount: { total: 360 },
    services: [
      { unit: "a.service", active: "active" },
      { unit: "b.service", active: "failed" },
    ],
    ...over,
  });

  test("turning notifications on takes stock and says nothing", () => {
    const first = changes(NOTHING_SEEN, state(), 1_800_000_000_000);
    expect(first.messages).toEqual([]);
    expect(first.seen).toEqual({
      primed: true,
      asks: { "12501": "2026-11-22T06:00:00" },
      hands: ["o/r#1"],
      questions: 360,
      failed: ["b.service"],
      // Already failed when notifications were turned on: counted as told.
      failing: { "b.service": { since: 1_800_000_000_000, told: true, last: 1_800_000_000_000 } },
    });
    expect(changes(first.seen, state()).messages).toEqual([]); // and nothing is said twice
  });

  test("each new thing is announced once, with where to go", () => {
    const seen: Seen = changes(NOTHING_SEEN, state()).seen;
    const later = state({
      asks: [
        { issue: 12501, replies: [reply("2026-11-22T06:00:00"), reply("2026-11-22T07:00:00")] },
        { issue: 12502, replies: [reply("2026-11-22T07:05:00", true)] },
      ],
      census: {
        hands: [
          { ref: "o/r#1", title: "t1", action: "Power on the host." },
          { ref: "o/r#2", title: "Sign the form", action: "" },
        ],
      },
      sessions: { waiting: [{ id: "porkbun", title: "Log into the main account" }] },
      questionCount: { total: 363 },
      services: [
        { unit: "a.service", active: "failed" },
        { unit: "b.service", active: "failed" },
      ],
    });
    const out = changes(seen, later);
    expect(out.messages.map((m) => [m.cls, m.title, m.body, m.url])).toEqual([
      ["replies", "The fleet replied on #12501", "Done. #12279 is first in the queue.", "/console/#ask"],
      ["needs", "Something needs you", "Sign the form", "/console/#needs/hands"],
      ["needs", "Something needs you", "Log into the main account", "/console/#needs/hands"],
      ["questions", "3 new questions for you", "363 open in all.", "/console/#needs/questions"],
      ["trouble", "A unit failed", "a.service", "/console/#more/services"],
    ]);
    expect(changes(out.seen, later).messages).toEqual([]);
    // His own reply is not news to him; fewer questions is not news either,
    // but it lowers the mark, so the next new one is.
    const fewer = changes(out.seen, { ...later, questionCount: { total: 300 } });
    expect(fewer.messages).toEqual([]);
    expect(changes(fewer.seen, { ...later, questionCount: { total: 301 } }).messages.map((m) => m.title)).toEqual([
      "1 new question for you",
    ]);
    // Something that went away and came back needs him again.
    const gone = changes(out.seen, { ...later, census: { hands: [] }, sessions: { waiting: [] } });
    expect(changes(gone.seen, later).messages.filter((m) => m.cls === "needs").length).toBe(3);
  });

  test("a source that could not be read changes nothing", () => {
    const seen = changes(NOTHING_SEEN, state()).seen;
    const blind = changes(seen, { asks: null, census: null, sessions: null, questionCount: null, services: null });
    expect(blind.messages).toEqual([]);
    expect(blind.seen).toEqual(seen); // nothing is forgotten, so nothing is announced again when it reads once more
  });

  test("the notifier reads the sources itself, sends what is owed, and remembers across restarts", async () => {
    const dir = mkdtempSync(join(tmpdir(), "pm-notify-"));
    try {
      let now = state();
      const still = { ttlMs: 1e12, load: async () => "x" };
      const sections = { board: still, prs: still, priorities: still, agents: still, more: still, questions: still };
      const live = (name: string) => ({ ttlMs: 1e12, load: async () => (now as Record<string, unknown>)[name] });
      const docs = new Docs({
        ...sections,
        asks: live("asks"),
        census: live("census"),
        sessions: live("sessions"),
        questionCount: live("questionCount"),
        services: live("services"),
      });
      const sent: string[] = [];
      const b = await browser();
      const push = new Push(
        fileStore(join(dir, "push.json")),
        "mailto:o@example.com",
        async (_u, init) => (sent.push(JSON.parse(await b.open(init.body)).title), { status: 201 }),
      );
      const notifier = new Notifier(docs, push, fileStore(join(dir, "notified.json")));
      expect(await notifier.tick()).toBe(0); // no device: nothing is read, nothing is sent
      await push.subscribe(b.subscription, "phone");
      expect(await notifier.tick()).toBe(0); // first pass: take stock
      now = state({ questionCount: { total: 361 } });
      expect(await notifier.tick()).toBe(1);
      expect(sent).toEqual(["1 new question for you"]);
      const restarted = new Notifier(docs, push, fileStore(join(dir, "notified.json")));
      expect(await restarted.tick()).toBe(0);
      expect(sent.length).toBe(1);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

test("a unit on a timer is reported once it keeps failing, not for one bad run, and again when it mends", () => {
  const T0 = 1_800_000_000_000;
  const primed: Seen = { ...NOTHING_SEEN, primed: true };
  const sync = (active: string) => ({
    services: [
      { unit: "pm-git-sync.service", active, description: "pm-git-sync — keep ~/git/code on clean main" },
      { unit: "pm-git-sync.timer", active: "active" },
      { unit: "pm-console.service", active: "active" },
    ],
  });
  // One failed run: nothing is said.
  let step = changes(primed, sync("failed"), T0);
  expect(step.messages).toEqual([]);
  // The next run mends it: forgotten, and still nothing said.
  step = changes(step.seen, sync("inactive"), T0 + 60_000);
  expect(step.messages).toEqual([]);
  expect(step.seen.failing).toEqual({});
  // Now it fails run after run. Being caught in the middle of a run is not a recovery.
  step = changes(step.seen, sync("failed"), T0 + 120_000);
  step = changes(step.seen, sync("activating"), T0 + 180_000);
  expect(step.messages).toEqual([]);
  step = changes(step.seen, sync("failed"), T0 + 120_000 + SUSTAINED_MS - 1);
  expect(step.messages).toEqual([]);
  step = changes(step.seen, sync("failed"), T0 + 120_000 + SUSTAINED_MS);
  expect(step.messages.map((m) => [m.cls, m.title, m.body, m.url, m.tag])).toEqual([
    [
      "trouble",
      "A unit keeps failing",
      "pm-git-sync.service, every run for 5 minutes. pm-git-sync — keep ~/git/code on clean main",
      "/console/#more/services",
      "unit-pm-git-sync.service",
    ],
  ]);
  // Said once, however long it goes on.
  for (const active of ["failed", "activating", "failed"]) {
    step = changes(step.seen, sync(active), T0 + 20 * 60_000);
    expect(step.messages).toEqual([]);
  }
  // Mended: that is said once too.
  step = changes(step.seen, sync("inactive"), T0 + 21 * 60_000);
  expect(step.messages.map((m) => `${m.title}: ${m.body}`)).toEqual(["A unit is working again: pm-git-sync.service"]);
  expect(changes(step.seen, sync("inactive"), T0 + 22 * 60_000).messages).toEqual([]);
  // And if it starts failing again later, the count starts again.
  step = changes(step.seen, sync("failed"), T0 + 30 * 60_000);
  expect(step.messages).toEqual([]);
  expect(changes(step.seen, sync("failed"), T0 + 30 * 60_000 + SUSTAINED_MS).messages).toHaveLength(1);

  // A service with no timer is still reported the moment it is seen failed, and its recovery is not news.
  const console_ = (active: string) => ({ services: [{ unit: "pm-console.service", active }] });
  step = changes(primed, console_("failed"), T0);
  expect(step.messages.map((m) => [m.title, m.body])).toEqual([["A unit failed", "pm-console.service"]]);
  expect(changes(step.seen, console_("failed"), T0 + SUSTAINED_MS * 3).messages).toEqual([]);
  expect(changes(step.seen, console_("active"), T0 + 1_000).messages).toEqual([]);
  // A timer by another name does not make a service periodic.
  const other = {
    services: [
      { unit: "pm-console.service", active: "failed" },
      { unit: "pm-console-expose.timer", active: "active" },
    ],
  };
  expect(changes(primed, other, T0).messages.map((m) => m.title)).toEqual(["A unit failed"]);

  // A state kept from before units on timers were told apart: what was reported is not reported again.
  const older: Seen = { ...NOTHING_SEEN, primed: true, failed: ["pm-git-sync.service"] };
  step = changes(older, sync("failed"), T0);
  expect(step.messages).toEqual([]);
  expect(changes(step.seen, sync("failed"), T0 + SUSTAINED_MS * 2).messages).toEqual([]);
  // Seen failed, then not looked at for longer than the wait: what happened in between is
  // not known, so the count starts again from the next sighting.
  step = changes(primed, sync("failed"), T0);
  step = changes(step.seen, sync("failed"), T0 + SUSTAINED_MS + 1);
  expect(step.messages).toEqual([]);
  step = changes(step.seen, sync("failed"), T0 + SUSTAINED_MS * 2);
  expect(step.messages).toEqual([]);
  step = changes(step.seen, sync("failed"), T0 + SUSTAINED_MS * 2 + 1);
  expect(step.messages.map((m) => m.body)).toEqual([
    "pm-git-sync.service, every run for 5 minutes. pm-git-sync — keep ~/git/code on clean main",
  ]);
  // Caught in the middle of a run is still a sighting of the unit not being well: it joins
  // two failed sightings that would otherwise be too far apart to count as one failing.
  step = changes(primed, sync("failed"), T0);
  step = changes(step.seen, sync("activating"), T0 + 4 * 60_000);
  step = changes(step.seen, sync("failed"), T0 + 8 * 60_000);
  expect(step.messages.map((m) => m.title)).toEqual(["A unit keeps failing"]);
  // A unit that is no longer listed is forgotten, not remembered as failing.
  step = changes(primed, sync("failed"), T0);
  step = changes(step.seen, { services: [{ unit: "pm-console.service", active: "active" }] }, T0 + 1_000);
  expect(step.seen.failing).toEqual({});
  // The services could not be read: nothing changes and nothing is said.
  step = changes(primed, sync("failed"), T0);
  const blind = changes(step.seen, { services: null }, T0 + SUSTAINED_MS * 2);
  expect(blind.messages).toEqual([]);
  expect(blind.seen.failing).toEqual(step.seen.failing);
});

test("a list of units that has stopped being read cannot make one failed run look sustained", async () => {
  const now = { t: 1_800_000_000_000 };
  const units = {
    readable: true,
    list: [
      { unit: "pm-git-sync.service", active: "failed", description: "keep the checkout on main" },
      { unit: "pm-git-sync.timer", active: "active" },
    ],
  };
  const empty = { ttlMs: 1_000, load: async () => null };
  const docs = new Docs(
    {
      ...Object.fromEntries(
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
          "more",
        ].map((name) => [name, empty]),
      ),
      services: {
        ttlMs: 1_000,
        load: async () => {
          if (!units.readable) throw new Error("systemctl did not answer");
          return structuredClone(units.list);
        },
      },
    },
    () => now.t,
  );
  const sent: string[] = [];
  const push = {
    status: async () => ({ devices: 1 }),
    send: async (_cls: string, m: { title: string }) => void sent.push(m.title),
  };
  const kept = { text: null as string | null };
  const notifier = new Notifier(
    docs,
    push as never,
    { read: async () => kept.text, write: async (text) => void (kept.text = text) },
    () => now.t,
  );
  const pass = async (ms: number) => {
    now.t += ms;
    await notifier.tick();
  };
  await pass(0); // takes stock: the unit is already failed, and that is not news
  units.list[0].active = "inactive";
  await pass(30_000);
  // One failed run is seen, and then the units cannot be read for half an hour.
  units.list[0].active = "failed";
  await pass(30_000);
  expect(await notifier.pending()).toBe(true);
  units.readable = false;
  for (let i = 0; i < 60; i++) await pass(30_000);
  // Nothing was observed in that time, so nothing is claimed about it.
  expect(sent).toEqual([]);
  // Read again and still failing: the count goes on from what was seen, and is reported when it has been seen for five minutes.
  units.readable = true;
  await pass(30_000);
  expect(sent).toEqual([]);
  for (let i = 0; i < 10; i++) await pass(30_000);
  expect(sent).toEqual(["A unit keeps failing"]);
  expect(await notifier.pending()).toBe(false);
});
