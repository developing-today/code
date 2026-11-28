// Web Push (code#6369): endpoint allow-list, VAPID JWT, delivery
// outcomes, and the alert/recovery push items in runPass.
import assert from "node:assert/strict";
import { test } from "node:test";
import { runPass } from "../src/run.js";
import { b64url, pushAll, validEndpoint, vapidAuth, vapidKeys } from "../src/webpush.js";
import { MANIFEST, PAGE_SCRIPT, SERVICE_WORKER } from "../src/pwa.js";

class MemStorage {
  constructor() {
    this.m = new Map();
  }
  async get(k) {
    return this.m.get(k);
  }
  async put(k, v) {
    this.m.set(k, v);
  }
}

const unb64 = (s) => Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (s.length % 4)) % 4)), (c) => c.charCodeAt(0));

test("only real push services are accepted as endpoints", () => {
  assert.ok(validEndpoint("https://fcm.googleapis.com/fcm/send/abc:def"));
  assert.ok(validEndpoint("https://updates.push.services.mozilla.com/wpush/v2/x"));
  assert.ok(validEndpoint("https://web.push.apple.com/QGx"));
  assert.equal(validEndpoint("http://fcm.googleapis.com/fcm/send/x"), null);
  assert.equal(validEndpoint("https://evil.example/fcm.googleapis.com"), null);
  assert.equal(validEndpoint("https://fcm.googleapis.com.evil.example/x"), null);
  assert.equal(validEndpoint("https://user:pw@fcm.googleapis.com/x"), null);
  assert.equal(validEndpoint(42), null);
  assert.equal(validEndpoint(`https://fcm.googleapis.com/${"a".repeat(2000)}`), null);
});

test("VAPID keys are made once and the JWT verifies with the public key", async () => {
  const storage = new MemStorage();
  const keys = await vapidKeys(storage);
  assert.deepEqual(await vapidKeys(storage), keys, "stable across calls");
  assert.equal(unb64(keys.publicKey).length, 65, "uncompressed P-256 point");
  const auth = await vapidAuth("https://fcm.googleapis.com/fcm/send/x", keys, "https://status.example", 1_800_000_000);
  const m = auth.match(/^vapid t=([^.]+)\.([^.]+)\.([^,]+), k=(.+)$/);
  assert.ok(m, auth);
  assert.equal(m[4], keys.publicKey);
  const claims = JSON.parse(new TextDecoder().decode(unb64(m[2])));
  assert.equal(claims.aud, "https://fcm.googleapis.com");
  assert.equal(claims.sub, "https://status.example");
  assert.equal(claims.exp, 1_800_000_000 + 12 * 3600);
  const pub = await crypto.subtle.importKey("raw", unb64(keys.publicKey), { name: "ECDSA", namedCurve: "P-256" }, false, ["verify"]);
  const ok = await crypto.subtle.verify(
    { name: "ECDSA", hash: "SHA-256" },
    pub,
    unb64(m[3]),
    new TextEncoder().encode(`${m[1]}.${m[2]}`),
  );
  assert.ok(ok, "signature verifies");
  assert.equal(b64url(new Uint8Array([251, 255])), "-_8");
});

test("pushAll counts sends, reports gone devices and failures", async () => {
  const keys = await vapidKeys(new MemStorage());
  const seen = [];
  const status = { a: 201, b: 410, c: 500 };
  const subs = ["a", "b", "c"].map((x) => ({ endpoint: `https://fcm.googleapis.com/fcm/send/${x}` }));
  const res = await pushAll(subs, keys, "https://s.example", Date.now(), async (url, init) => {
    seen.push(init);
    return new Response(null, { status: status[url.slice(-1)] });
  });
  assert.equal(res.sent, 1);
  assert.deepEqual(res.gone, [subs[1].endpoint]);
  assert.equal(res.failed.length, 1);
  assert.match(res.failed[0], /HTTP 500 from fcm\.googleapis\.com/);
  assert.equal(seen[0].method, "POST");
  assert.equal(seen[0].headers.TTL, "86400");
  assert.equal(seen[0].headers.Urgency, "high");
  assert.equal(seen[0].body, undefined, "payloadless");
});

test("alert and recovery each push once, and a failed push is retried", async () => {
  const pushes = [];
  let failing = true;
  const push = async (msg) => {
    if (failing) throw new Error("web push down");
    pushes.push(msg);
  };
  const t = (s) => Date.parse(s);
  let state = null;
  const pass = async (ok, at) => {
    const r = await runPass(state, { ok, detail: ok ? "" : "HTTP 530" }, null, t(at), 2, "x", { push });
    state = r.state;
    return r;
  };
  await pass(false, "2026-11-19T04:00:00Z");
  const r = await pass(false, "2026-11-19T04:10:00Z");
  assert.match(r.errors.join(" "), /web push down/);
  failing = false;
  await pass(false, "2026-11-19T04:20:00Z");
  assert.equal(pushes.length, 1);
  assert.equal(pushes[0].title, "Queue broker DOWN");
  assert.match(pushes[0].body, /unreachable for/);
  await pass(true, "2026-11-19T05:00:00Z");
  assert.equal(pushes.length, 2);
  assert.equal(pushes[1].title, "Queue broker back UP");
  await pass(true, "2026-11-19T05:10:00Z");
  assert.equal(pushes.length, 2, "steady state pushes nothing");
});

test("PWA pieces parse and point at the right paths", () => {
  const m = JSON.parse(MANIFEST);
  assert.equal(m.start_url, "/");
  assert.equal(m.display, "standalone");
  new Function(SERVICE_WORKER); // syntax check
  new Function(PAGE_SCRIPT);
  assert.match(SERVICE_WORKER, /fetch\("\/status\.json"/);
  assert.match(PAGE_SCRIPT, /"\/vapid\.json"/);
  assert.match(PAGE_SCRIPT, /"\/subscribe"/);
});
