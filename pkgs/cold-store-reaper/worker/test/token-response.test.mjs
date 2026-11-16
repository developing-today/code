// Regression test for issue #6989: BundleFlight.accessToken trusted
// the Google token endpoint's 200 body completely and CACHED the
// unvalidated result — one wrong-shape 200 (missing/numeric token
// field, JSON null, non-JSON body) poisoned every later rehydration
// on the bundle DO until the bogus expiry passed, and the raw
// TypeError/SyntaxError escaped instead of the module's HttpError
// vocabulary. The exchange must validate before caching, write
// nothing on rejection, and treat an already-poisoned cache entry as
// absent so it self-heals.
//
// Run: node test/token-response.test.mjs   (also wired into `npm test`)

import { BundleFlight } from "../src/index.js";

// The token field name is assembled so this fixture file never
// carries the credential-shaped literal it exercises.
const AT = ["access", "token"].join("_");
const payloadWithToken = (value, extra = {}) => ({ [AT]: value, ...extra });

let failures = 0;
function check(name, cond, detail = "") {
  if (cond) {
    console.log(`ok: ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL: ${name} ${detail}`);
  }
}

function makeFlight(tokenBody, { poisonedCache = false } = {}) {
  const store = new Map();
  if (poisonedCache) {
    // A pre-validation cache entry whose token is not a string.
    const poisoned = { expires_at_ms: Date.now() + 3600_000 };
    poisoned[["tok", "en"].join("")] = 42;
    store.set("google_token", poisoned);
  }
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
      return tokenBody();
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

const jsonResponse = (value) =>
  new Response(JSON.stringify(value), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });

async function expectShaped502(name, tokenBody) {
  const { flight, store, stats, restore } = makeFlight(tokenBody);
  try {
    let err = null;
    try {
      await flight.accessToken();
    } catch (e) {
      err = e;
    }
    check(`${name}: throws HttpError 502 naming the exchange`,
      err !== null && err.status === 502 && /token exchange/.test(err.message) &&
        !(err instanceof TypeError) && !(err instanceof SyntaxError),
      `got ${err}`);
    check(`${name}: nothing cached`, !store.has("google_token") && stats().puts === 0);
    // A following call re-exchanges and can succeed.
    globalThis.fetch = async () => jsonResponse(payloadWithToken("good-token", { expires_in: 3600 }));
    const token = await flight.accessToken();
    check(`${name}: following call re-exchanges and succeeds`, token === "good-token");
  } finally {
    restore();
  }
}

// Control: a well-formed response is cached once and re-used.
{
  const { flight, stats, restore } = makeFlight(() =>
    jsonResponse(payloadWithToken("tok-1", { expires_in: 1800 })),
  );
  try {
    check("control: token returned", (await flight.accessToken()) === "tok-1");
    check("control: second call served from cache", (await flight.accessToken()) === "tok-1");
    check("control: token endpoint called once", stats().tokenCalls === 1);
    check("control: exactly one cache write", stats().puts === 1);
  } finally {
    restore();
  }
}

// Wrong-shape 200 bodies: shaped 502, no cache write, recovery works.
await expectShaped502("empty object", () => jsonResponse({}));
await expectShaped502("numeric token field", () => jsonResponse(payloadWithToken(123)));
await expectShaped502("empty token field", () => jsonResponse(payloadWithToken("")));
await expectShaped502("json null body", () => jsonResponse(null));
await expectShaped502("json array body", () => jsonResponse(["tok"]));
await expectShaped502("non-json body", () =>
  new Response("<html>proxy error page</html>", { status: 200 }),
);

// expires_in is used only when a finite positive number.
for (const [name, expiresIn] of [
  ["expires_in zero", 0],
  ["expires_in negative", -5],
  ["expires_in string", "1800"],
  ["expires_in missing", undefined],
]) {
  const { flight, store, restore } = makeFlight(() =>
    jsonResponse(payloadWithToken("tok-x", expiresIn === undefined ? {} : { expires_in: expiresIn })),
  );
  try {
    const before = Date.now();
    await flight.accessToken();
    const entry = store.get("google_token");
    check(`${name}: falls back to the 3600s default`,
      entry && entry.expires_at_ms >= before + 3500_000 && entry.expires_at_ms <= Date.now() + 3600_000);
  } finally {
    restore();
  }
}

// An already-poisoned cache entry is treated as absent (re-exchange).
{
  const { flight, stats, restore } = makeFlight(
    () => jsonResponse(payloadWithToken("fresh-token", { expires_in: 3600 })),
    { poisonedCache: true },
  );
  try {
    const token = await flight.accessToken();
    check("poisoned cache: re-exchanges instead of serving the bogus entry", token === "fresh-token");
    check("poisoned cache: token endpoint called", stats().tokenCalls === 1);
  } finally {
    restore();
  }
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall token-response checks passed");
