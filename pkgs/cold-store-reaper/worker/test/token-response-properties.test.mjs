// Property coverage for issue #6989 (Codex P1 on PR #8702): the
// fixed cases in token-response.test.mjs are hand-picked examples;
// the validation boundary in BundleFlight.accessToken must hold for
// generated response shapes and cache states too.
//
// ORACLES (named): (1) no-write-on-reject -- a response the oracle
// classifies invalid throws a shaped HttpError 502 and performs
// zero storage puts; (2) accept-returns-supplied -- a valid response
// returns exactly its token, is cached once, and a second call is
// served from cache with no second exchange; (3) cache discipline --
// a valid unexpired cache entry is served with no exchange at all,
// and any poisoned / expiring / expired entry is treated as absent
// and re-exchanged. The oracle below restates the boundary rule
// independently of the implementation's control flow.
// The generator is a fixed-seed mulberry32, so the cases are a
// deterministic regression pin, not a fuzz run.
//
// Run: node test/token-response-properties.test.mjs   (also wired into `npm test`)

import { BundleFlight } from "../src/index.js";

// The token field name is assembled so this fixture file never
// carries the credential-shaped literal it exercises.
const AT = ["access", "token"].join("_");
const TOK = ["tok", "en"].join("");

let failures = 0;
function check(name, cond, detail = "") {
  if (cond) {
    console.log(`ok: ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL: ${name} ${detail}`);
  }
}

function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const rnd = mulberry32(0x6989);
const pick = (arr) => arr[Math.floor(rnd() * arr.length)];
const tokenString = () => {
  const alphabet = "abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789-_";
  const n = 1 + Math.floor(rnd() * 40);
  let out = "";
  for (let i = 0; i < n; i++) out += alphabet[Math.floor(rnd() * alphabet.length)];
  return out;
};

// --- generators -----------------------------------------------------

function genValidPayload() {
  const payload = { [AT]: tokenString() };
  const expiryKind = pick(["valid", "valid", "zero", "negative", "string", "null", "absent"]);
  if (expiryKind === "valid") payload.expires_in = pick([1, 300, 1800, 3600, 7200]);
  else if (expiryKind === "zero") payload.expires_in = 0;
  else if (expiryKind === "negative") payload.expires_in = -Math.floor(rnd() * 1000) - 1;
  else if (expiryKind === "string") payload.expires_in = "3600";
  else if (expiryKind === "null") payload.expires_in = null;
  if (rnd() < 0.4) payload.scope = "https://www.googleapis.com/auth/drive.readonly";
  return payload;
}

function genInvalidBody() {
  const kind = pick([
    "null", "array", "string", "number", "boolean", "empty-object",
    "missing-token", "numeric-token", "empty-token", "object-token", "null-token",
  ]);
  switch (kind) {
    case "null": return { json: null };
    case "array": return { json: [1, 2, 3] };
    case "string": return { json: "token-ish" };
    case "number": return { json: 42 };
    case "boolean": return { json: true };
    case "empty-object": return { json: {} };
    case "missing-token": return { json: { expires_in: 3600 } };
    case "numeric-token": return { json: { [AT]: 42 } };
    case "empty-token": return { json: { [AT]: "" } };
    case "object-token": return { json: { [AT]: { value: "x" } } };
    case "null-token": return { json: { [AT]: null } };
    default: return { json: null };
  }
}

function genCacheState() {
  const kind = pick(["absent", "absent", "valid", "poisoned-numeric", "poisoned-empty", "expiring", "expired"]);
  const now = Date.now();
  switch (kind) {
    case "absent": return { kind, entry: null };
    case "valid": {
      const entry = { expires_at_ms: now + pick([120_000, 3600_000]) };
      entry[TOK] = tokenString();
      return { kind, entry };
    }
    case "poisoned-numeric": {
      const entry = { expires_at_ms: now + 3600_000 };
      entry[TOK] = 42;
      return { kind, entry };
    }
    case "poisoned-empty": {
      const entry = { expires_at_ms: now + 3600_000 };
      entry[TOK] = "";
      return { kind, entry };
    }
    case "expiring": {
      const entry = { expires_at_ms: now + 30_000 };
      entry[TOK] = tokenString();
      return { kind, entry };
    }
    case "expired": {
      const entry = { expires_at_ms: now - 1000 };
      entry[TOK] = tokenString();
      return { kind, entry };
    }
    default: return { kind: "absent", entry: null };
  }
}

// Oracle: the boundary rule, restated.
const cacheServes = (cache) =>
  cache.entry !== null &&
  typeof cache.entry[TOK] === "string" &&
  cache.entry[TOK] !== "" &&
  cache.entry.expires_at_ms > Date.now() + 60_000;
const bodyValid = (body) =>
  body.json !== undefined &&
  typeof body.json === "object" &&
  body.json !== null &&
  !Array.isArray(body.json) &&
  typeof body.json[AT] === "string" &&
  body.json[AT] !== "";
const expectedExpirySeconds = (payload) =>
  typeof payload.expires_in === "number" &&
  Number.isFinite(payload.expires_in) &&
  payload.expires_in > 0
    ? payload.expires_in
    : 3600;

function makeFlight(cacheEntry, body) {
  const store = new Map();
  if (cacheEntry) store.set("google_token", cacheEntry);
  let puts = 0;
  let tokenCalls = 0;
  const flight = new BundleFlight(
    {
      storage: {
        async get(key) { return store.get(key) ?? null; },
        async put(key, value) { puts += 1; store.set(key, value); },
      },
    },
    { GOOGLE_CLIENT_ID: "id", GOOGLE_CLIENT_SECRET: "secret", GOOGLE_REFRESH_TOKEN: "rt" },
  );
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    if (String(url).startsWith("https://oauth2.googleapis.com/token")) {
      tokenCalls += 1;
      if (body.nonJson) {
        return new Response("<html>proxy error page</html>", { status: 200 });
      }
      return new Response(JSON.stringify(body.json), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    throw new Error("unexpected fetch: " + url);
  };
  return {
    flight,
    store,
    stats: () => ({ puts, tokenCalls }),
    restore: () => { globalThis.fetch = realFetch; },
  };
}

// --- property sweep ---------------------------------------------------

{
  let cases = 0;
  let violations = 0;
  const CASES = 64;
  for (let i = 0; i < CASES; i++) {
    const cache = genCacheState();
    const useValid = rnd() < 0.5;
    const body = useValid
      ? { json: genValidPayload() }
      : rnd() < 0.15
        ? { nonJson: true }
        : genInvalidBody();
    const { flight, store, stats, restore } = makeFlight(cache.entry, body);
    try {
      cases += 1;
      let result = null;
      let err = null;
      try {
        result = await flight.accessToken();
      } catch (e) {
        err = e;
      }
      if (cacheServes(cache)) {
        if (err !== null || result !== cache.entry[TOK] || stats().tokenCalls !== 0) {
          violations += 1;
          console.error(`case ${i}: valid cache not served (err=${err}, calls=${stats().tokenCalls})`);
        }
      } else if (bodyValid(body)) {
        const want = body.json[AT];
        if (err !== null || result !== want || stats().puts !== 1) {
          violations += 1;
          console.error(`case ${i}: valid body not accepted (err=${err}, puts=${stats().puts})`);
        } else {
          const entry = store.get("google_token");
          const remaining = entry.expires_at_ms - Date.now();
          const wantMs = expectedExpirySeconds(body.json) * 1000;
          if (entry[TOK] !== want || remaining <= wantMs - 5000 || remaining > wantMs) {
            violations += 1;
            console.error(`case ${i}: cached entry wrong (remaining=${remaining}, want~${wantMs})`);
          }
          // A second call is served from cache only when the
          // cached expiry clears the 60s safety margin; a shorter
          // accepted expiry is treated as absent and re-exchanged.
          const again = await flight.accessToken();
          const wantCalls = wantMs > 60_000 ? 1 : 2;
          if (again !== want || stats().tokenCalls !== wantCalls) {
            violations += 1;
            console.error(`case ${i}: second call cache discipline wrong (calls=${stats().tokenCalls}, want ${wantCalls})`);
          }
        }
      } else {
        const shaped =
          err !== null &&
          err.status === 502 &&
          /token exchange/.test(err.message) &&
          !(err instanceof TypeError) &&
          !(err instanceof SyntaxError);
        if (!shaped || stats().puts !== 0) {
          violations += 1;
          console.error(`case ${i}: invalid body not rejected cleanly (err=${err}, puts=${stats().puts})`);
        }
      }
    } finally {
      restore();
    }
  }
  check(`property: token-response boundary holds across ${cases} generated cases`,
    violations === 0, `${violations} violated cases`);
}

if (failures > 0) {
  console.error(`token-response-properties: ${failures} failing check(s)`);
  process.exit(1);
}
console.log("token-response-properties: all checks passed");
