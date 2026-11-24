import assert from "node:assert/strict";
import { test } from "node:test";
import { MemoryStore } from "../src/messages.js";
import { noteWindowSeconds, signalUuid } from "../src/notes.js";
import { route } from "../src/routes.js";

const OWNER = "o".repeat(40);
const DREW = "d".repeat(40);
const ARCHER = "a".repeat(40);
const OTHER = "b".repeat(40);
const env = { ACCESS_TOKEN: OWNER };
const BASE = "https://bridge.example";
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

function harness() {
  const store = new MemoryStore(() => "2026-11-23T00:00:00.000Z");
  const call = (path, init) => route(new Request(BASE + path, init), env, () => store);
  const bearer = (token) => ({ authorization: `Bearer ${token}`, "content-type": "application/json" });
  const register = (identity, token, extra = {}) =>
    call("/notes/register", {
      method: "POST",
      headers: bearer(OWNER),
      body: JSON.stringify({ identity, token, ...extra }),
    });
  const send = (token, to, note) =>
    call("/notes/send", { method: "POST", headers: bearer(token), body: JSON.stringify({ to, note }) });
  return { store, call, bearer, register, send };
}

test("signal UUIDs are stable in a window and rotate across windows", () => {
  const now = 1_800_000_000_000;
  const a = signalUuid("nobody/here", 0, now);
  assert.match(a, UUID);
  assert.equal(signalUuid("nobody/here", 0, now + 1000), a, "same bucket, same UUID");
  const later = now + noteWindowSeconds("nobody/here") * 1000 + 1000;
  assert.notEqual(signalUuid("nobody/here", 0, later), a, "next bucket rotates the UUID");
  assert.notEqual(signalUuid("nobody/here", 1, now), a, "a delivery counter changes the UUID at once");
  assert.notEqual(signalUuid("other/path", 0, now), a, "paths have independent signals");
});

test("public signal answers registered and unregistered paths in the same shape", async () => {
  const { call, register } = harness();
  assert.equal((await register("muse/archer", ARCHER)).status, 201);
  for (const path of ["/muse/archer", "/nobody/here", "/some-class/some-name"]) {
    const res = await call(path);
    assert.equal(res.status, 200, path);
    assert.match((await res.text()).trim(), UUID, path);
    assert.equal(res.headers.get("cache-control"), "no-store");
  }
  // Operational token routes keep their own shape: the page path is not a signal.
  assert.equal((await call(`/p/${OWNER}`)).status, 200);
});

test("registration is owner-only and tokens are not echoed", async () => {
  const { call, register } = harness();
  const denied = await call("/notes/register", {
    method: "POST",
    headers: { authorization: `Bearer ${ARCHER}`, "content-type": "application/json" },
    body: JSON.stringify({ identity: "muse/archer", token: ARCHER }),
  });
  assert.equal(denied.status, 404);
  const ok = await register("muse/archer", ARCHER);
  assert.equal(ok.status, 201);
  const body = await ok.text();
  assert.doesNotMatch(body, new RegExp(ARCHER));
  assert.deepEqual(JSON.parse(body), { identity: "muse/archer", can_send: true, can_inbox: true });
});

test("direct and wildcard delivery, inbox isolation, ack, and signal change", async () => {
  const { call, bearer, register, send } = harness();
  await register("user/drew", DREW);
  await register("muse/archer", ARCHER);
  await register("muse/other", OTHER);

  const before = (await (await call("/muse/archer")).text()).trim();
  const direct = await send(DREW, "muse/archer", "direct note");
  assert.equal(direct.status, 201);
  assert.deepEqual((await direct.json()).recipients, ["muse/archer"]);
  const after = (await (await call("/muse/archer")).text()).trim();
  assert.notEqual(after, before, "delivery changes the public signal");

  const wild = await send(DREW, "muse/*", "class note");
  assert.equal(wild.status, 201);
  assert.deepEqual((await wild.json()).recipients, ["muse/archer", "muse/other"]);

  const inboxRes = await call("/notes/inbox", { headers: bearer(ARCHER) });
  assert.equal(inboxRes.status, 200);
  const inbox = await inboxRes.json();
  assert.equal(inbox.identity, "muse/archer");
  assert.deepEqual(
    inbox.notes.map((n) => [n.from, n.note]),
    [
      ["user/drew", "direct note"],
      ["user/drew", "class note"],
    ],
  );

  // Another identity cannot ack Archer's notes.
  const foreign = await call("/notes/ack", {
    method: "POST",
    headers: bearer(OTHER),
    body: JSON.stringify({ ids: inbox.notes.map((n) => n.id) }),
  });
  assert.deepEqual(await foreign.json(), { acked: 0 });
  assert.equal((await (await call("/notes/inbox", { headers: bearer(ARCHER) })).json()).notes.length, 2);

  const ack = await call("/notes/ack", {
    method: "POST",
    headers: bearer(ARCHER),
    body: JSON.stringify({ ids: inbox.notes.map((n) => n.id) }),
  });
  assert.deepEqual(await ack.json(), { acked: 2 });
  assert.deepEqual((await (await call("/notes/inbox", { headers: bearer(ARCHER) })).json()).notes, []);

  // Name and broadcast wildcards resolve against registered identities.
  const byName = await send(DREW, "*/other", "name note");
  assert.deepEqual((await byName.json()).recipients, ["muse/other"]);
  const all = await send(ARCHER, "*/*", "broadcast note");
  assert.deepEqual((await all.json()).recipients, ["muse/archer", "muse/other", "user/drew"]);

  // Unknown tokens and unmatched addresses fail without storing anything.
  assert.equal((await send("z".repeat(40), "muse/archer", "nope")).status, 404);
  const unmatched = await send(DREW, "ghost/nobody", "nope");
  assert.equal(unmatched.status, 400);
});
