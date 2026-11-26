import assert from "node:assert/strict";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
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
  url: string;
  child: ChildProcess;
}

async function startServer(file: string, keyFile: string, port: number): Promise<Server> {
  const child = spawn("turso", ["dev", "--db-file", file, "--auth-jwt-key-file", keyFile, "--port", String(port)], {
    stdio: "ignore",
  });
  const url = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error("turso dev exited during startup");
    try {
      const response = await fetch(`${url}/v2/pipeline`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ requests: [{ type: "execute", stmt: { sql: "SELECT 1", args: [] } }] }),
      });
      if (response.status !== 503) return { url, child };
    } catch {
      // Wait until local sqld is accepting requests.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  child.kill("SIGTERM");
  throw new Error("turso dev did not become ready");
}

async function execute(url: string, token: string, sql: string): Promise<Response> {
  return fetch(`${url}/v2/pipeline`, {
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
  if (spawnSync("turso", ["--version"], { stdio: "ignore" }).status !== 0) {
    t.skip("optional local sqld prerequisite unavailable: turso CLI missing");
    return;
  }

  const dir = mkdtempSync(join(tmpdir(), "fleet-direct-read-live-"));
  const servers: Server[] = [];
  try {
    const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
    const pkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", pair.privateKey));
    const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
    const metadataAuthKey = join(dir, "metadata.pub");
    const queueAuthKey = join(dir, "queue.pub");
    const otherDbAuthKey = join(dir, "other-db.pub");
    writeFileSync(metadataAuthKey, b64url(publicKey));
    for (const path of [queueAuthKey, otherDbAuthKey]) {
      const targetPair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
      writeFileSync(path, b64url(new Uint8Array(await crypto.subtle.exportKey("raw", targetPair.publicKey))));
    }

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
    let metadata: Server;
    let queue: Server;
    let otherDb: Server;
    try {
      metadata = await startServer(join(dir, "metadata.db"), metadataAuthKey, basePort);
      servers.push(metadata);
      queue = await startServer(join(dir, "queue.db"), queueAuthKey, basePort + 1);
      servers.push(queue);
      otherDb = await startServer(join(dir, "other.db"), otherDbAuthKey, basePort + 2);
      servers.push(otherDb);
    } catch (error) {
      t.skip(`optional local sqld prerequisite unavailable: ${error instanceof Error ? error.message : String(error)}`);
      return;
    }

    const read = await execute(metadata.url, token, "SELECT 1 AS allowed");
    assert.equal(read.status, 200, `metadata read should pass: ${await read.text()}`);
    const write = await execute(metadata.url, token, "CREATE TABLE forbidden_write (id INTEGER)");
    await assertRefused(write, "sqld must refuse writes");

    const queueRead = await execute(queue.url, token, "SELECT 1");
    await assertRefused(queueRead, "queue DB must refuse metadata token");

    const wrongDb = await execute(otherDb.url, token, "SELECT 1");
    await assertRefused(wrongDb, "another database must refuse the metadata token");

    const expiredToken = await mintDirectReadToken(config, { bindingId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", ttlSeconds: 1, now: 1 });
    const expired = await execute(metadata.url, expiredToken, "SELECT 1");
    await assertRefused(expired, "sqld must refuse expired token");
  } finally {
    for (const server of servers) server.child.kill("SIGTERM");
    rmSync(dir, { recursive: true, force: true });
  }
});
