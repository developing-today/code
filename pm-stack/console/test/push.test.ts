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
import { changes, NOTHING_SEEN, Notifier, type Seen } from "../src/notify.ts";
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
      expect(await again.status()).toEqual({ ...status, devices: 2, known: marks });
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
      let answer = (url: string) => (url === a.subscription.endpoint ? 410 : 201);
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

  test("the server only posts to push services", async () => {
    const good = (await browser()).subscription;
    expect(validSubscription(good)).toEqual(good);
    for (const endpoint of [
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
    const first = changes(NOTHING_SEEN, state());
    expect(first.messages).toEqual([]);
    expect(first.seen).toEqual({
      primed: true,
      asks: { "12501": "2026-11-22T06:00:00" },
      hands: ["o/r#1"],
      questions: 360,
      failed: ["b.service"],
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
