// #15793 criterion 1: the edge worker-checks probe
// (pm-stack/console/edge/src/worker-checks.ts) asks every in-service
// Worker for the commit it was deployed from, trying GET /version
// first and reading the `version` field of the answer (commitOf).
// cold-store-rehydrator named no commit: /version fell through to the
// 404 and /health answers { ok: true } only, so the probe recorded
// this Worker answering with commit null.
//
// Oracle: #15793 criterion 1 plus the probe contract — an
// unauthenticated GET /version answering {ok, service, version} from
// the VERSION var stamped per deploy (the pm-service / github-ci /
// fleet-git-rehost / cf-container-hello shape), "unknown" when
// unstamped. These checks pin:
//
// - the stamped and unstamped /version answers,
// - non-GET /version refused with the module's 405 error envelope,
// - the neighbours are untouched: /health still answers { ok: true }
//   with no auth, and /fetch/<sha256> still refuses an unauthenticated
//   caller with 401 before any manifest lookup.
//
// Run: node test/version-route.test.mjs   (also wired into `npm test`)

import worker from "../src/index.js";

let failures = 0;
function check(name, cond, detail = "") {
  if (cond) {
    console.log(`ok: ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL: ${name} ${detail}`);
  }
}

const SHA = "a".repeat(64);
const COMMIT = "1633533a31492c7f80f9aaec1819c633322f3512";
const get = (path, env) => worker.fetch(new Request(`https://cold-store-rehydrator.test${path}`), env);

// No network may be reached by any of these routes.
const realFetch = globalThis.fetch;
let fetches = 0;
globalThis.fetch = async () => {
  fetches += 1;
  throw new Error("network disabled in this test");
};

try {
  const stamped = await get("/version", { VERSION: COMMIT });
  const stampedJson = await stamped.json();
  check("stamped /version answers 200 with the deployed commit",
    stamped.status === 200 && stampedJson.ok === true &&
      stampedJson.service === "cold-store-rehydrator" && stampedJson.version === COMMIT,
    `got ${stamped.status} ${JSON.stringify(stampedJson)}`);

  const unstamped = await get("/version", {});
  const unstampedJson = await unstamped.json();
  check("unstamped /version answers \"unknown\" honestly",
    unstamped.status === 200 && unstampedJson.version === "unknown",
    `got ${unstamped.status} ${JSON.stringify(unstampedJson)}`);

  const post = await worker.fetch(
    new Request("https://cold-store-rehydrator.test/version", { method: "POST" }),
    { VERSION: COMMIT },
  );
  const postJson = await post.json();
  check("non-GET /version is a 405 with the module's error envelope",
    post.status === 405 && typeof postJson.error === "string",
    `got ${post.status} ${JSON.stringify(postJson)}`);

  const health = await get("/health", {});
  const healthJson = await health.json();
  check("/health still answers { ok: true } with no auth",
    health.status === 200 && healthJson.ok === true,
    `got ${health.status} ${JSON.stringify(healthJson)}`);

  const gated = await get(`/fetch/${SHA}`, {});
  const gatedJson = await gated.json();
  check("/fetch/<sha256> still refuses an unauthenticated caller with 401",
    gated.status === 401 && gatedJson.error === "unauthorized",
    `got ${gated.status} ${JSON.stringify(gatedJson)}`);

  check("no route reached the network", fetches === 0, `fetches=${fetches}`);
} finally {
  globalThis.fetch = realFetch;
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall version-route checks passed");
