#!/usr/bin/env node
// #16086 read-only live acceptance probe. Entire CLI contract: --help,
// --base-url URL (or --base-url=URL); no positionals or other options.
// Like bin/provider-catalog-sync.mjs, parse all arguments before effects.
// RP credentials come ONLY from FLEET_HOSTED_RP_TOKEN, never CLI/files.
// Exit 0: all live checks pass; 1: incomplete/failed proof; 2: usage error.
// No deploy, fixture provisioning, queue mutation, UA override or redirect.
import { pathToFileURL } from "node:url";

export const USAGE = "Usage: node smoke/live-identity.mjs [--base-url HTTPS_ORIGIN] [--help]\nSet FLEET_HOSTED_RP_TOKEN to a valid RP JWT. Exit 1 means live proof is incomplete.";
export const INVALID_RP_TOKEN = "eyJhbGciOiJub25lIn0.e30.AA";

export function parseArgs(args) {
  let baseUrl = "https://fleet-hosted.developing-today.workers.dev";
  let help = false;
  const seen = new Set();
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    const name = arg.split("=", 1)[0];
    if (seen.has(name)) throw new Error(`duplicate argument: ${arg}`);
    seen.add(name);
    if (arg === "--help") help = true;
    else if (arg === "--base-url" || arg.startsWith("--base-url=")) {
      const value = arg === "--base-url" ? args[++i] : arg.slice("--base-url=".length);
      if (!value || value.startsWith("--")) throw new Error(`missing value: ${arg}`);
      baseUrl = value;
    } else throw new Error(`unknown argument: ${arg}`);
  }
  let url;
  try { url = new URL(baseUrl); } catch { throw new Error(`invalid --base-url: ${baseUrl}`); }
  if (url.protocol !== "https:" || url.username || url.password || url.search || url.hash || url.pathname !== "/") {
    throw new Error("--base-url must be an HTTPS origin without credentials, path, query or fragment");
  }
  return { baseUrl: url.origin, help };
}

export async function verifyLiveIdentity({ baseUrl, token, fetchImpl = globalThis.fetch }) {
  const checks = [];
  async function check(name, path, options, accepts) {
    try {
      const response = await fetchImpl(`${baseUrl}${path}`, {
        ...options, redirect: "error", signal: AbortSignal.timeout(15000),
      });
      let body;
      try { body = await response.json(); } catch { /* WAF/non-JSON is not auth evidence. */ }
      const incomplete = response.status === 503 && body?.error === "auth_not_configured";
      const passed = !incomplete && accepts(response.status, body);
      checks.push({ name, status: response.status, result: passed ? "passed" : "incomplete",
        ...(passed && name.endsWith("identity") ? { version: body.version ?? body.build.version } : {}),
        ...(incomplete ? { error: "auth_not_configured" } : {}) });
    } catch {
      // Do not echo errors/bodies/credentials: even transport errors can contain secrets.
      checks.push({ name, result: "incomplete", error: "request_failed" });
    }
  }
  await check("health identity", "/health", {}, (status, body) =>
    status === 200 && body?.status === "ok" && body.service === "fleet-hosted" && typeof body.version === "string" && body.version.length > 0);
  await check("capabilities identity", "/capabilities", {}, (status, body) =>
    status === 200 && body?.build?.name === "fleet-hosted" && body.build.invoked_as === "fleet-hosted" &&
    body.build.edition === "hosted" && body.build.profile === "hosted" && typeof body.build.version === "string" && body.build.version.length > 0);
  if (checks[0].version && checks[1].version && checks[0].version !== checks[1].version) {
    checks.push({ name: "identity version agreement", result: "incomplete", error: "version_mismatch" });
  }
  const protectedOptions = (credential) => ({
    method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${credential}` }, body: "{}",
  });
  // alg:none is deliberately unsupported by src/jwt.ts: expected 401 invalid_token,
  // not missing_token, a WAF 403, a generic 503, or an accepted anonymous request.
  await check("invalid RP rejected", "/v1/queue/health", protectedOptions(INVALID_RP_TOKEN),
    (status, body) => status === 401 && body?.error === "invalid_token");
  if (!token) checks.push({ name: "valid RP accepted", result: "incomplete", error: "missing_rp_token" });
  else await check("valid RP accepted", "/v1/queue/health", protectedOptions(token),
    (status, body) => status === 200 && body?.ok === true);
  return { result: checks.every((c) => c.result === "passed") ? "passed" : "incomplete", checks };
}

export async function main(args, { env = process.env, fetchImpl = globalThis.fetch, output = console.log } = {}) {
  let options;
  try { options = parseArgs(args); }
  catch (error) { output(error.message); return 2; }
  if (options.help) { output(USAGE); return 0; }
  const token = env.FLEET_HOSTED_RP_TOKEN?.trim();
  if (token && !/^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(token)) {
    output("FLEET_HOSTED_RP_TOKEN must be a compact RP JWT (not an opaque service token)");
    return 2;
  }
  const report = await verifyLiveIdentity({ ...options, token, fetchImpl });
  output(JSON.stringify(report));
  return report.result === "passed" ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = await main(process.argv.slice(2));
}
