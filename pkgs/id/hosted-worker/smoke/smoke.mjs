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
import { spawn, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";

const PKG = new URL("..", import.meta.url).pathname;

// The schema version has one source: SCHEMA_VERSION in src/user-do.ts.
// Reading it here rather than repeating a literal means a future bump
// updates the DO, the bootstrap and this assertion together (#13673).
const SCHEMA_VERSION = (() => {
  const src = readFileSync(new URL("../src/user-do.ts", import.meta.url), "utf8");
  const m = src.match(/export const SCHEMA_VERSION = (\d+);/);
  if (!m) throw new Error("smoke: SCHEMA_VERSION not found in src/user-do.ts");
  return Number(m[1]);
})();
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

// The front door reads the central tenancy registry (TENANCY_DB)
// before it touches a namespace, and an unmigrated local D1 fails
// closed there (503 tenancy_registry_unavailable). Migrate the local
// registry and register binding A so the valid-context request
// reaches the restricted-namespace step this part checks.
function wranglerSync(args) {
  const r = spawnSync("wrangler", args, { cwd: PKG, stdio: ["ignore", "pipe", "pipe"], encoding: "utf8", env: { ...process.env, CI: "1" } });
  return r.status === 0 ? null : (r.stderr || r.stdout || "").trim().slice(-400);
}
{
  const migrated = wranglerSync(["d1", "migrations", "apply", "TENANCY_DB", "--local"]);
  check("local tenancy registry migrates", migrated === null, migrated ?? undefined);
  const seeded = wranglerSync([
    "d1", "execute", "TENANCY_DB", "--local", "--command",
    "INSERT OR IGNORE INTO account (account_id, sub, email_contact, created_at, state, tier) VALUES ('smoke-acct', 'smoke-subject', NULL, '2026-11-26T00:00:00Z', 'active', 'standard'); " +
      "INSERT OR IGNORE INTO binding (binding_id, account_id, iroh_id, status, bound_at, superseded_by) VALUES ('" + A + "', 'smoke-acct', 'smoke-iroh', 'active', '2026-11-26T00:00:00Z', NULL);",
  ]);
  check("binding A registered in the local tenancy registry", seeded === null, seeded ?? undefined);
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
    check("state reports binding, jurisdiction, current schema version (source: SCHEMA_VERSION)", body.binding_id === A && body.jurisdiction === "us" && body.schema_version === SCHEMA_VERSION, JSON.stringify(body));

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

    // Slice A (#9954): blob round-trip via reference indirection,
    // user isolation by hash, dedup, reference-only delete.
    // Fresh bindings per run: wrangler dev persists DO/R2 state
    // under .wrangler/state, and a previous run's references for
    // the fixed A/B bindings would (correctly) still resolve.
    const hex32 = () => Array.from({ length: 32 }, () => "0123456789abcdef"[Math.floor(Math.random() * 16)]).join("");
    const BA = hex32();
    const BB = hex32();
    const blob = (name, binding, j, hash) => "/v1/blobs" + (hash ? "/" + hash : "") + "?name=" + name + "&binding=" + binding + "&jurisdiction=" + j;
    const bytes = "hosted blob bytes";
    r = await fetch(base + blob(BA, BA, "us"), { method: "PUT", body: bytes });
    const putBody = await r.json();
    check("PUT blob on A", r.status === 200 && putBody.size === bytes.length && /^[0-9a-f]{64}$/.test(putBody.hash), JSON.stringify(putBody));
    const hash = putBody.hash;
    r = await fetch(base + blob(BA, BA, "us", hash));
    check("GET blob on A round-trips", r.status === 200 && (await r.text()) === bytes);
    r = await fetch(base + blob(BB, BB, "us", hash));
    check("B cannot read A's blob by hash (isolation)", r.status === 404);
    r = await fetch(base + blob(BB, BB, "us"), { method: "PUT", body: bytes });
    const putB = await r.json();
    check("PUT same bytes on B dedups to the same hash", r.status === 200 && putB.hash === hash && putB.deduped === true, JSON.stringify(putB));
    r = await fetch(base + blob(BB, BB, "us", hash));
    check("GET blob on B via its own reference", r.status === 200 && (await r.text()) === bytes);
    r = await fetch(base + blob(BA, BA, "us", hash), { method: "DELETE" });
    check("DELETE blob reference on A", r.status === 200);
    r = await fetch(base + blob(BA, BA, "us", hash));
    check("A can no longer read after delete", r.status === 404);
    r = await fetch(base + blob(BB, BB, "us", hash));
    check("B still reads after A's delete (shared bytes kept)", r.status === 200);

    // Slice B (#9955): the usage read path reports live metering
    // and the configured ceiling.
    const usageUrl = (name, binding, j) => "/v1/usage?name=" + name + "&binding=" + binding + "&jurisdiction=" + j;
    r = await fetch(base + usageUrl(BB, BB, "us"));
    const usage = await r.json();
    check("usage reports metering + ceiling", r.status === 200 && usage.blob_count === 1 && usage.blob_bytes === bytes.length && usage.ceiling_bytes === 1073741824 && usage.requests > 0 && usage.at_cap === false, JSON.stringify(usage));
    r = await fetch(base + "/v1/export?name=" + BB + "&binding=" + BB + "&jurisdiction=us");
    const exp = await r.json();
    check("export carries the user's blobs and kv", r.status === 200 && Array.isArray(exp.blobs) && exp.blobs.length === 1 && exp.usage.storage_bytes >= bytes.length, JSON.stringify(exp).slice(0, 160));
  } finally { await stopDev(dev); }
}

console.log(failures === 0 ? "SMOKE OK" : "SMOKE FAILURES: " + failures);
process.exit(failures === 0 ? 0 : 1);
