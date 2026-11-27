import assert from "node:assert/strict";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { lookup } from "node:dns/promises";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { configFromEnv, type HostedEnv } from "../src/config.ts";
import { mintDirectReadToken } from "../src/direct-read.ts";

function b64url(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64url");
}

interface Server {
  /** Base user-API URL, e.g. http://127.0.0.1:19992 */
  url: string;
  adminUrl: string;
  child: ChildProcess;
}

async function waitForServer(url: string, child: ChildProcess): Promise<void> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error("sqld exited during startup");
    try {
      const response = await fetch(`${url}/v2/pipeline`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ requests: [{ type: "execute", stmt: { sql: "SELECT 1", args: [] } }] }),
      });
      // Any non-503 means the server is up (401: auth on, no token — expected).
      if (response.status !== 503) return;
    } catch {
      // Wait until local sqld is accepting requests.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("sqld did not become ready");
}

async function startNamespacedServer(dir: string, keyFile: string, port: number): Promise<Server> {
  const child = spawn(
    "sqld",
    [
      "--db-path",
      join(dir, "data"),
      "--http-listen-addr",
      `127.0.0.1:${port}`,
      "--admin-listen-addr",
      `127.0.0.1:${port + 1}`,
      "--enable-namespaces",
      "--auth-jwt-key-file",
      keyFile,
    ],
    { stdio: "ignore" },
  );
  const server: Server = { url: `http://127.0.0.1:${port}`, adminUrl: `http://127.0.0.1:${port + 1}`, child };
  try {
    await waitForServer(server.url, child);
  } catch (error) {
    child.kill("SIGTERM");
    throw error;
  }
  return server;
}

async function createNamespace(adminUrl: string, name: string): Promise<void> {
  const response = await fetch(`${adminUrl}/v1/namespaces/${name}/create`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: "{}",
  });
  if (!response.ok) throw new Error(`create namespace ${name}: HTTP ${response.status}: ${await response.text()}`);
}

/**
 * Execute one Hrana pipeline statement against a namespace.
 *
 * Namespace routing is Host-based (`<namespace>.<...>` → that
 * namespace), so the request goes to the public wildcard that
 * resolves to loopback (`*.db.sarna.dev` → 127.0.0.1) with the
 * namespace as the first label. Undici sets Host from the URL, so no
 * header override is needed.
 */
async function execute(server: Server, namespace: string, token: string, sql: string): Promise<Response> {
  return fetch(`http://${namespace}.db.sarna.dev:${new URL(server.url).port}/v2/pipeline`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify({ requests: [{ type: "execute", stmt: { sql, args: [] } }] }),
  });
}

async function assertRefused(response: Response, reason: string): Promise<void> {
  const body = await response.text();
  assert.ok(
    response.status >= 400 || /"type"\s*:\s*"error"/.test(body),
    `${reason}: expected sqld refusal, received HTTP ${response.status}: ${body}`,
  );
}

test("live direct-read: sqld enforces minted metadata token and rejects other scopes", async (t) => {
  if (spawnSync("sqld", ["--version"], { stdio: "ignore" }).status !== 0) {
    t.skip("optional local sqld prerequisite unavailable: sqld missing from PATH");
    return;
  }
  try {
    const resolved = await lookup("metadata-db.db.sarna.dev");
    if (resolved.address !== "127.0.0.1") {
      t.skip(`optional local sqld prerequisite unavailable: wildcard loopback DNS resolves to ${resolved.address}`);
      return;
    }
  } catch {
    t.skip("optional local sqld prerequisite unavailable: wildcard loopback DNS does not resolve");
    return;
  }

  const dir = mkdtempSync(join(tmpdir(), "fleet-direct-read-live-"));
  let server: Server | undefined;
  try {
    const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
    const pkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", pair.privateKey));
    const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
    const authKey = join(dir, "auth.pub");
    writeFileSync(authKey, b64url(publicKey));

    const env: HostedEnv = {
      METADATA_DB_ID: "metadata-db",
      QUEUE_DB_ID: "queue-db",
      DIRECT_READ_SIGNING_KEY_PKCS8: b64url(pkcs8),
      DIRECT_READ_SIGNING_KEY_ID: "metadata-key",
      DIRECT_READ_VERIFY_KEYS_JSON: JSON.stringify({ "metadata-key": b64url(publicKey) }),
    };
    const config = configFromEnv(env).directRead;
    assert.ok(config, "complete direct-read configuration is required");
    const token = await mintDirectReadToken(config, { bindingId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", ttlSeconds: 60 });

    const basePort = 19_000 + (process.pid % 1_000);
    try {
      server = await startNamespacedServer(dir, authKey, basePort);
    } catch (error) {
      t.skip(`optional local sqld prerequisite unavailable: ${error instanceof Error ? error.message : String(error)}`);
      return;
    }
    await createNamespace(server.adminUrl, "metadata-db");
    await createNamespace(server.adminUrl, "queue-db");

    const read = await execute(server, "metadata-db", token, "SELECT 1 AS allowed");
    assert.equal(read.status, 200, `metadata read should pass: ${await read.text()}`);
    const write = await execute(server, "metadata-db", token, "CREATE TABLE forbidden_write (id INTEGER)");
    await assertRefused(write, "sqld must refuse writes");

    const queueRead = await execute(server, "queue-db", token, "SELECT 1");
    await assertRefused(queueRead, "queue DB must refuse metadata token");

    const wrongDb = await execute(server, "other-db", token, "SELECT 1");
    await assertRefused(wrongDb, "another database must refuse the metadata token");

    const expiredToken = await mintDirectReadToken(config, { bindingId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", ttlSeconds: 1, now: 1 });
    const expired = await execute(server, "metadata-db", expiredToken, "SELECT 1");
    await assertRefused(expired, "sqld must refuse expired token");
  } finally {
    server?.child.kill("SIGTERM");
    rmSync(dir, { recursive: true, force: true });
  }
});
