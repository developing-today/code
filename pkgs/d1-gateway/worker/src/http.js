// d1-gateway: Turso/libSQL-compatible HTTP endpoint (Hrana v2/v3 JSON
// pipelines) over Cloudflare D1, so clients written for Turso move to
// D1 by changing their URL.
//
//   POST /db/<name>/v2/pipeline   (and /v3/pipeline)
//   GET  /db/<name>/v2            version probe (200)
//   GET  /db/<name>/v3            404, so @libsql/client stays on v2
//                                 (v3 cursors are not served)
//   GET  /health                  status, deployed version, databases
//
// <name> picks the D1 binding DB_<NAME> (dashes become underscores).
// Auth: "Authorization: Bearer <token>" whose SHA-256 hex is listed
// for that database in TOKEN_SHA256 (a JSON var: {"<name>": [hex...]}).
// Hashes only; token values never live in the repo.
//
// Other Workers call it over a service binding to the Gateway
// entrypoint: env.D1GW.pipeline("<name>", body), no token needed.
import { pipeline } from "./hrana.js";

export function bindingName(db) {
  return `DB_${db.toUpperCase().replace(/-/g, "_")}`;
}

export function databases(env) {
  return Object.keys(env)
    .filter((k) => k.startsWith("DB_") && env[k] && typeof env[k].prepare === "function")
    .map((k) => k.slice(3).toLowerCase().replace(/_/g, "-"))
    .sort();
}

async function sha256Hex(text) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export async function authorized(env, db, header) {
  const m = /^Bearer\s+(.+)$/i.exec(header ?? "");
  if (!m) return false;
  let table = env.TOKEN_SHA256 ?? {};
  if (typeof table === "string") {
    try {
      table = JSON.parse(table);
    } catch {
      return false;
    }
  }
  const allowed = table[db] ?? [];
  if (allowed.length === 0) return false;
  const hash = await sha256Hex(m[1].trim());
  return allowed.includes(hash);
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function logger(db) {
  return (what, e, sqls) => {
    console.error(
      JSON.stringify({
        db,
        what,
        error: String(e?.message ?? e).slice(0, 500),
        sql: sqls.map((s) => s.replace(/\s+/g, " ").slice(0, 300)),
      }),
    );
  };
}

export async function runPipeline(env, db, body) {
  const binding = env[bindingName(db)];
  if (!binding) throw new Error(`unknown database ${db}`);
  return pipeline(binding, body, logger(db));
}

export async function handle(request, env) {
  const url = new URL(request.url);
  if (url.pathname === "/health" || url.pathname === "/") {
    return json({
      ok: true,
      service: "d1-gateway",
      version: env.CF_VERSION_METADATA?.id ?? null,
      databases: databases(env),
    });
  }
  const m = /^\/db\/([a-z0-9][a-z0-9-]*)(\/.*)?$/.exec(url.pathname);
  if (!m) return json({ error: "not found" }, 404);
  const db = m[1];
  const rest = (m[2] ?? "/").replace(/\/+$/, "") || "/";
  if (!env[bindingName(db)]) return json({ error: `unknown database ${db}` }, 404);
  if (request.method === "GET" && (rest === "/v2" || rest === "/")) return new Response("ok");
  if (request.method === "GET" && rest === "/v3") return json({ error: "v3 is not served; use v2" }, 404);
  if (!(await authorized(env, db, request.headers.get("authorization")))) {
    return json({ error: "Unauthorized" }, 401);
  }
  if (request.method === "POST" && (rest === "/v2/pipeline" || rest === "/v3/pipeline")) {
    let body;
    try {
      body = await request.json();
    } catch {
      return json({ error: "request body is not JSON" }, 400);
    }
    try {
      return json(await runPipeline(env, db, body));
    } catch (e) {
      return json({ error: String(e?.message ?? e) }, 500);
    }
  }
  return json({ error: "not found" }, 404);
}

