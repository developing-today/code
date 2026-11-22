#!/usr/bin/env node
// Creates disposable staging-only runtime secrets, then verifies the staged
// one-use flow using unmistakably fake credentials and a separate DO namespace.
import { randomBytes } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const STAGING_HOST = "muse-agent-handoff-staging.developing-today.workers.dev";
const FAKE_BWS = "muse-handoff-stage-dummy-bws-not-a-credential";
const FAKE_OP = "muse-handoff-stage-dummy-op-not-a-credential";
const NAME = "stage-smoke";
const SHA_RE = /^[0-9a-f]{40}$/i;
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function usage() {
  console.error(
    "usage: stage-smoke.mjs prepare --secrets-file <path> | verify --base-url <https-url> --secrets-file <path> --source-sha <sha> --version-id <uuid> --proof-file <path>",
  );
  process.exit(2);
}

function options(args, allowed) {
  const parsed = new Map();
  for (let i = 0; i < args.length; i += 2) {
    const key = args[i];
    const value = args[i + 1];
    if (!allowed.includes(key) || value === undefined || parsed.has(key)) usage();
    parsed.set(key, value);
  }
  if (parsed.size !== allowed.length) usage();
  return Object.fromEntries(parsed);
}

async function prepare(args) {
  const { "--secrets-file": path } = options(args, ["--secrets-file"]);
  const secrets = {
    ADMIN_TOKEN: randomBytes(48).toString("base64url"),
    HANDOFF_ENCRYPTION_KEY: randomBytes(32).toString("base64"),
  };
  const target = resolve(path);
  await mkdir(dirname(target), { recursive: true, mode: 0o700 });
  await writeFile(target, `${JSON.stringify(secrets)}\n`, { mode: 0o600 });
}

async function verify(args) {
  const {
    "--base-url": baseUrlArg,
    "--secrets-file": secretsPath,
    "--source-sha": sourceSha,
    "--version-id": versionId,
    "--proof-file": proofPath,
  } = options(args, ["--base-url", "--secrets-file", "--source-sha", "--version-id", "--proof-file"]);
  if (!SHA_RE.test(sourceSha) || !UUID_RE.test(versionId)) usage();
  const base = new URL(baseUrlArg);
  if (base.protocol !== "https:" || base.hostname !== STAGING_HOST) {
    throw new Error(`refusing to make smoke requests outside ${STAGING_HOST}`);
  }
  base.pathname = base.pathname.replace(/\/$/, "");
  const secrets = JSON.parse(await readFile(secretsPath, "utf8"));
  if (typeof secrets.ADMIN_TOKEN !== "string" || typeof secrets.HANDOFF_ENCRYPTION_KEY !== "string") {
    throw new Error("staging-only secret file is incomplete");
  }
  const request = async (path, init) => fetch(new URL(`${base.pathname}${path}`, base), {
    ...init,
    signal: AbortSignal.timeout(20_000),
  });

  const rootResponse = await request("/", { method: "GET" });
  const rootBody = await rootResponse.text();
  if (rootResponse.status !== 200 || !rootBody.includes("Muse Fleet handoff service")) {
    throw new Error("staging root smoke check failed");
  }

  const unauthorizedResponse = await request("/api/create", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: NAME, bws_access_token: FAKE_BWS, op_service_account_token: FAKE_OP }),
  });
  if (unauthorizedResponse.status !== 401) throw new Error("staging admin authorization smoke check failed");

  const createResponse = await request("/api/create", {
    method: "POST",
    headers: {
      "Authorization": `Bearer ${secrets.ADMIN_TOKEN}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      name: NAME,
      bws_access_token: FAKE_BWS,
      op_service_account_token: FAKE_OP,
    }),
  });
  if (createResponse.status !== 201) throw new Error("staging create smoke check failed");
  const created = await createResponse.json();
  if (created.expires_in_seconds !== 900 || typeof created.claim_url !== "string") {
    throw new Error("staging create response did not contain the expected claim URL and TTL");
  }
  const claimUrl = new URL(created.claim_url);
  if (claimUrl.origin !== base.origin || !claimUrl.hash.startsWith("#")) {
    throw new Error("staging create returned an unexpected claim origin");
  }
  const [id, claim] = claimUrl.hash.slice(1).split(".");
  if (!UUID_RE.test(id ?? "") || !/^[A-Za-z0-9_-]{40,48}$/.test(claim ?? "")) {
    throw new Error("staging claim fragment did not match the expected format");
  }
  const claimBody = JSON.stringify({ id, claim });
  const claimHeaders = { "Content-Type": "application/json" };
  const claimResponse = await request("/api/claim", {
    method: "POST", headers: claimHeaders, body: claimBody,
  });
  if (claimResponse.status !== 200) throw new Error("staging claim smoke check failed");
  const claimed = await claimResponse.json();
  const roundTripMatches = claimed.agent_name === NAME &&
    claimed.bws_access_token === FAKE_BWS &&
    claimed.op_service_account_token === FAKE_OP;
  if (!roundTripMatches) throw new Error("staging claim did not return the stage-only dummy credentials");
  const replayResponse = await request("/api/claim", {
    method: "POST", headers: claimHeaders, body: claimBody,
  });
  if (replayResponse.status !== 410) throw new Error("staging one-use replay check failed");

  const proof = {
    schema: 1,
    worker: "muse-agent-handoff-staging",
    sourceSha: sourceSha.toLowerCase(),
    versionId,
    smoke: {
      rootStatus: rootResponse.status,
      unauthorizedCreateStatus: unauthorizedResponse.status,
      createStatus: createResponse.status,
      claimStatus: claimResponse.status,
      replayStatus: replayResponse.status,
      roundTripMatches,
      credentials: "stage-only dummy values; no production handoff data",
    },
  };
  const target = resolve(proofPath);
  await mkdir(dirname(target), { recursive: true, mode: 0o700 });
  await writeFile(target, `${JSON.stringify(proof, null, 2)}\n`, { mode: 0o600 });
}

async function main() {
  const [command, ...args] = process.argv.slice(2);
  if (command === "prepare") await prepare(args);
  else if (command === "verify") await verify(args);
  else usage();
}

if (fileURLToPath(import.meta.url) === resolve(process.argv[1] ?? "")) {
  main().catch((error) => {
    console.error(`stage-smoke: ${error.message}`);
    process.exitCode = 1;
  });
}
