// Generated tests for the OAuth state machine and the sealed-blob format.
//
// Model-based: random operation sequences (sign-ins with right and wrong
// PKCE verifiers, code reuse, refreshes with the current or an old token,
// refreshes presented by another client, tampered tokens) run against the
// real routes. The oracle is a small model of refresh-token families: each
// family has one current token; redeeming it rotates; redeeming an older one
// revokes the family; a token is only ever redeemable by the client it was
// issued to, and a refusal for the wrong client changes nothing.
//
// Property-based: sealed blobs round-trip, carry their typ, die at expiry,
// don't verify under another key, and any one-character change is refused.
//
// Seeds are fixed so runs are reproducible; MB_PBT_SEED=<n> adds one, and
// MB_PBT_RUNS=<n> widens the fixed set.
import assert from "node:assert/strict";
import { test } from "node:test";
import { MemoryStore } from "../src/messages.js";
import { bearerChannel, seal, unseal } from "../src/oauth.js";
import { route } from "../src/routes.js";

const TOKEN = "o".repeat(20) + "0123456789abcdefXYZ_-";
const GPT = "g".repeat(20) + "0123456789abcdefXYZ_-";
const env = { ACCESS_TOKEN: TOKEN, CHATGPT_TOKEN: GPT };
const ORIGIN = "https://bridge.example";
const REDIRECT = "https://claude.ai/api/mcp/auth_callback";
const B64U = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

const RUNS = Number(process.env.MB_PBT_RUNS ?? 30);
const SEEDS = [...Array.from({ length: RUNS }, (_, i) => i + 1), ...(process.env.MB_PBT_SEED ? [Number(process.env.MB_PBT_SEED)] : [])];

/** mulberry32: small, deterministic, good enough to drive generators. */
function rng(seed) {
  let a = seed >>> 0;
  const next = () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const int = (n) => Math.floor(next() * n);
  return { next, int, pick: (xs) => xs[int(xs.length)], str: (n, alphabet = B64U) => Array.from({ length: n }, () => alphabet[int(alphabet.length)]).join("") };
}

const b64u = (buf) => Buffer.from(buf).toString("base64url");
const challengeOf = async (v) => b64u(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(v)));

/** One character changed to a different one, at a random position. */
function mutate(r, s) {
  const i = r.int(s.length);
  let c;
  do c = r.pick([...B64U, "."]);
  while (c === s[i]);
  return s.slice(0, i) + c + s.slice(i + 1);
}

function harness() {
  const store = new MemoryStore(() => "2026-11-20T00:00:00.000Z");
  const call = (path, init) => route(new Request(ORIGIN + path, init), env, () => store);
  const form = (data) => ({
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded", origin: ORIGIN },
    body: new URLSearchParams(data).toString(),
  });
  return { call, form };
}

async function register(call, method) {
  const res = await call("/register", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ client_name: "model", redirect_uris: [REDIRECT], ...(method ? { token_endpoint_auth_method: method } : {}) }),
  });
  assert.equal(res.status, 201);
  return res.json();
}

/** The system under test, driven only through its HTTP routes. */
function system() {
  const { call, form } = harness();
  const token = async (client, data) => {
    const res = await call("/token", form({ client_id: client.client_id, ...(client.client_secret ? { client_secret: client.client_secret } : {}), ...data }));
    return { status: res.status, body: await res.json() };
  };
  return {
    register: (method) => register(call, method),
    async approve(client, verifier, secret) {
      const q = new URLSearchParams({
        response_type: "code",
        client_id: client.client_id,
        redirect_uri: REDIRECT,
        code_challenge: await challengeOf(verifier),
        code_challenge_method: "S256",
      });
      const page = await call(`/authorize?${q}`);
      assert.equal(page.status, 200);
      const req = (await page.text()).match(/name="req" value="([^"]+)"/)[1].replace(/&amp;/g, "&");
      const res = await call("/authorize", form({ req, action: "allow", secret }));
      assert.equal(res.status, 302);
      return new URL(res.headers.get("location")).searchParams.get("code");
    },
    redeem: (client, code, verifier) => token(client, { grant_type: "authorization_code", code, redirect_uri: REDIRECT, code_verifier: verifier }),
    refresh: (client, rt) => token(client, { grant_type: "refresh_token", refresh_token: rt }),
    channelOf: (at) => bearerChannel(new Request(ORIGIN + "/mcp", { headers: { authorization: `Bearer ${at}` } }), env),
  };
}

async function runSequence(seed, steps) {
  const r = rng(seed);
  const sut = system();
  const clients = [await sut.register("none"), await sut.register("none"), await sut.register()];
  assert.ok(clients[2].client_secret, "a client registered with no method gets a secret");
  // Model state.
  const families = []; // { client, channel, current, revoked }
  const rts = []; // { value, fam }
  const trace = [];
  const fail = (msg) => `${msg} (seed ${seed}, after: ${trace.join(" ")})`;

  const issued = async (res, fam) => {
    assert.equal(res.status, 200, fail(`expected tokens, got ${JSON.stringify(res.body)}`));
    assert.equal(await sut.channelOf(res.body.access_token), families[fam].channel, fail("access token channel"));
    rts.push({ value: res.body.refresh_token, fam });
    families[fam].current = rts.length - 1;
  };
  const refused = (res, what) => {
    assert.equal(res.status, 400, fail(`${what} should be refused, got ${res.status}`));
    assert.equal(res.body.error, "invalid_grant", fail(what));
    assert.equal(res.body.access_token, undefined, fail(what));
  };

  for (let i = 0; i < steps; i++) {
    const roll = r.next();
    if (roll < 0.22 || rts.length === 0) {
      // Sign in; sometimes try a wrong verifier first and the code again after.
      const ci = r.int(clients.length);
      const client = clients[ci];
      const channel = r.next() < 0.3 ? "chatgpt" : "claude";
      const verifier = r.str(43 + r.int(40));
      const code = await sut.approve(client, verifier, channel === "chatgpt" ? GPT : `${ORIGIN}/p/${TOKEN}`);
      trace.push(`signin(c${ci},${channel})`);
      if (r.next() < 0.3) refused(await sut.redeem(client, code, mutate(r, verifier)), "a wrong verifier");
      if (r.next() < 0.2) refused(await sut.redeem(clients[(ci + 1) % clients.length], code, verifier), "another client's code");
      families.push({ client: ci, channel, current: -1, revoked: false });
      await issued(await sut.redeem(client, code, verifier), families.length - 1);
      if (r.next() < 0.3) refused(await sut.redeem(client, code, verifier), "a reused code");
    } else if (roll < 0.55) {
      // Refresh with some family's current token (the happy path, unless revoked).
      const fam = r.int(families.length);
      const f = families[fam];
      const res = await sut.refresh(clients[f.client], rts[f.current].value);
      trace.push(`refresh(f${fam})`);
      if (f.revoked) refused(res, "a revoked family's token");
      else await issued(res, fam);
    } else if (roll < 0.78) {
      // Refresh with any token ever issued: an old one is a replay.
      const ri = r.int(rts.length);
      const f = families[rts[ri].fam];
      const res = await sut.refresh(clients[f.client], rts[ri].value);
      trace.push(`redeem(rt${ri})`);
      if (f.revoked) refused(res, "a revoked family's token");
      else if (f.current === ri) await issued(res, rts[ri].fam);
      else {
        refused(res, "a replayed refresh token");
        f.revoked = true;
      }
    } else if (roll < 0.9) {
      // Another client presents the token: refused, and the family is untouched.
      const ri = r.int(rts.length);
      const owner = families[rts[ri].fam].client;
      const other = (owner + 1 + r.int(clients.length - 1)) % clients.length;
      refused(await sut.refresh(clients[other], rts[ri].value), "another client's refresh token");
      trace.push(`steal(rt${ri},c${other})`);
    } else {
      // A tampered token is refused and changes nothing.
      const ri = r.int(rts.length);
      const f = families[rts[ri].fam];
      refused(await sut.refresh(clients[f.client], mutate(r, rts[ri].value)), "a tampered refresh token");
      trace.push(`tamper(rt${ri})`);
    }
  }
  return families.length;
}

test("generated OAuth sequences match the refresh-family model", async () => {
  let families = 0;
  for (const seed of SEEDS) families += await runSequence(seed, 30);
  assert.ok(families >= SEEDS.length, "every sequence signed in at least once");
});

/** A random JSON-ish payload with a typ and maybe an expiry. */
function payload(r, typ) {
  const p = { typ };
  for (let i = r.int(5); i > 0; i--) {
    const k = r.str(1 + r.int(6), "abcdefghijklmnopqrstuvwxyz");
    if (k === "typ" || k === "exp") continue;
    p[k] = r.pick([() => r.str(r.int(30)), () => r.int(1e9), () => r.next() < 0.5, () => null, () => [r.int(9), r.str(3)]])();
  }
  if (r.next() < 0.5) p.exp = Math.floor(Date.now() / 1000) + 60 + r.int(1e6);
  return p;
}

test("sealed blobs round-trip, keep their typ and key, expire, and refuse any one-character change", async () => {
  const TYPES = ["req", "code", "at", "rt"];
  const otherEnv = { ACCESS_TOKEN: "x".repeat(41) };
  for (const seed of SEEDS) {
    const r = rng(1000 + seed);
    for (let i = 0; i < 20; i++) {
      const typ = r.pick(TYPES);
      const p = payload(r, typ);
      const blob = await seal(env, p);
      const why = `seed ${seed}, payload ${JSON.stringify(p)}`;
      assert.deepEqual(await unseal(env, blob, typ), p, `round trip: ${why}`);
      assert.equal(await unseal(env, blob, r.pick(TYPES.filter((t) => t !== typ))), null, `typ confusion: ${why}`);
      assert.equal(await unseal(otherEnv, blob, typ), null, `another key: ${why}`);
      const changed = mutate(r, blob);
      assert.equal(await unseal(env, changed, typ), null, `one-character change ${blob} -> ${changed}: ${why}`);
      const expired = { ...p, exp: Math.floor(Date.now() / 1000) - 1 - r.int(1e6) };
      assert.equal(await unseal(env, await seal(env, expired), typ), null, `expired: ${why}`);
    }
  }
});
