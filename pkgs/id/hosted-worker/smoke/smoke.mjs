// Smoke for #5199 against local wrangler dev (workerd), in two parts.
//
// Part 1 — the application worker (wrangler.toml). Local workerd does
// not implement jurisdiction restrictions (namespace.jurisdiction()
// throws), so every request that would need a restricted namespace
// must fail closed (503 jurisdiction_unavailable) and requests with
// missing/invalid context must fail closed before that (401/403).
// Nothing may fall back to the unrestricted namespace.
//
// Part 2 — the UserDO through the test-only harness
// (wrangler.test.toml): lazy creation + idempotent schema bootstrap,
// concurrent first requests, immutable identity binding, cross-binding
// isolation, and guard failures.
//
// Usage: node smoke/smoke.mjs
import { spawn } from "node:child_process";

const PKG = new URL("..", import.meta.url).pathname;
const A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
let failures = 0;
function check(label, cond, extra) {
  console.log((cond ? "PASS" : "FAIL") + " " + label + (extra ? " — " + extra : ""));
  if (!cond) failures++;
}

function startDev(config, port) {
  const args = ["dev", "--port", String(port), "--local"];
  if (config) args.push("-c", config);
  const child = spawn("wrangler", args, { cwd: PKG, stdio: ["ignore", "pipe", "pipe"], detached: true });
  child.stderr.on("data", (d) => process.stderr.write("[wrangler] " + d));
  return child;
}
async function stopDev(child) {
  try { process.kill(-child.pid, "SIGTERM"); } catch {}
  await new Promise((r) => setTimeout(r, 1500));
}
async function waitUp(port, tries) {
  for (let i = 0; i < (tries || 45); i++) {
    try { const r = await fetch("http://127.0.0.1:" + port + "/v1/state"); if (r.status !== 404) return true; } catch {}
    await new Promise((r) => setTimeout(r, 2000));
  }
  return false;
}

// ---------- Part 1: application worker fails closed locally ----------
{
  const port = 8797;
  const dev = startDev(null, port);
  try {
    check("app worker starts", await waitUp(port));
    const base = "http://127.0.0.1:" + port;
    let r = await fetch(base + "/v1/state");
    check("no context -> 401 missing_binding", r.status === 401 && (await r.json()).error === "missing_binding");
    r = await fetch(base + "/v1/state", { headers: { "x-binding-id": A } });
    check("binding without jurisdiction -> 403", r.status === 403 && (await r.json()).error === "missing_jurisdiction");
    r = await fetch(base + "/v1/state", { headers: { "x-binding-id": A, "x-jurisdiction": "apac" } });
    check("unsupported jurisdiction -> 403", r.status === 403 && (await r.json()).error === "unsupported_jurisdiction");
    r = await fetch(base + "/v1/state", { headers: { "x-binding-id": "google:123", "x-jurisdiction": "us" } });
    check("subject-derived id -> 401 invalid_binding", r.status === 401 && (await r.json()).error === "invalid_binding");
    r = await fetch(base + "/v1/state", { headers: { "x-binding-id": A, "x-jurisdiction": "us" } });
    const body = await r.json();
    check("valid context -> 503 jurisdiction_unavailable (no unrestricted fallback)", r.status === 503 && body.error === "jurisdiction_unavailable", JSON.stringify(body));
  } finally { await stopDev(dev); }
}

// ---------- Part 2: UserDO via test harness ----------
{
  const port = 8798;
  const dev = startDev("wrangler.test.toml", port);
  try {
    const base = "http://127.0.0.1:" + port;
    const q = (name, binding, j, extra) => "/v1/state?name=" + name + "&binding=" + binding + "&jurisdiction=" + j + (extra || "");
    let up = false;
    for (let i = 0; i < 45 && !up; i++) {
      try { const r = await fetch(base + q(A, A, "us")); up = r.status === 200; } catch {}
      if (!up) await new Promise((r) => setTimeout(r, 2000));
    }
    check("harness + DO start, lazy create", up);
    let r = await fetch(base + q(A, A, "us"));
    let body = await r.json();
    check("state reports binding, jurisdiction, schema v1", body.binding_id === A && body.jurisdiction === "us" && body.schema_version === 1, JSON.stringify(body));

    const many = await Promise.all(Array.from({ length: 8 }, () => fetch(base + q("cccccccccccccccccccccccccccccccc", "cccccccccccccccccccccccccccccccc", "eu"))));
    check("concurrent first requests all 200 (idempotent bootstrap)", many.every((x) => x.status === 200));

    const kv = (name, binding, j, key) => "/v1/kv/" + key + "?name=" + name + "&binding=" + binding + "&jurisdiction=" + j;
    r = await fetch(base + kv(A, A, "us", "greeting"), { method: "PUT", body: "hello-a" });
    check("PUT kv on A", r.status === 200);
    r = await fetch(base + kv(A, A, "us", "greeting"));
    check("GET kv on A returns value", r.status === 200 && (await r.json()).value === "hello-a");
    r = await fetch(base + kv(B, B, "us", "greeting"));
    check("B cannot see A data (isolation)", r.status === 404);
    r = await fetch(base + q(A, B, "us"));
    check("mismatched binding on bound object -> 409", r.status === 409);
    r = await fetch(base + q(A, A, "eu"));
    check("mismatched jurisdiction on bound object -> 409", r.status === 409);
    r = await fetch(base + q(A, A, "us", "&routed=no"));
    check("unrouted request -> 403", r.status === 403);
  } finally { await stopDev(dev); }
}

console.log(failures === 0 ? "SMOKE OK" : "SMOKE FAILURES: " + failures);
process.exit(failures === 0 ? 0 : 1);
