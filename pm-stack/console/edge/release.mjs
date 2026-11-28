#!/usr/bin/env node
// Puts this checkout's Worker into service, starting from whatever is
// there, including nothing.
//
//   node release.mjs --sha <commit>
//       the Worker named in wrangler.jsonc
//   node release.mjs --sha <commit> --scratch
//       the same steps on a Worker made for the purpose: removed first
//       if it is there, so the start is nothing; removed again at the
//       end. It shows that a stand-up from nothing works without the
//       real Worker being touched.
//
// The steps:
//   1. No Worker of that name: stand up an empty one (init/). A version
//      cannot be uploaded to a Worker that does not exist, and the
//      storage behind the Durable Object class is only made by a deploy.
//   2. Upload this checkout as a version, tagged with the commit and
//      stamped with the full commit id and the time (VERSION and
//      DEPLOYED_AT vars, which the health check reports back). It
//      serves nothing yet.
//   3. Put that version at 100%.
//   4. Ask the Worker which version it is. If it does not answer as
//      this one, with an age that matches its stamp, put back the
//      version that was serving, and fail.
//
// When wrangler.jsonc has a Durable Object migration the Worker has not
// had, steps 2 and 3 are one `wrangler deploy`: a version cannot carry a
// migration.
//
// Needs CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID in the
// environment, and `node build.mjs` to have been run. The decisions are
// in release-plan.mjs, where they are tested; this file does the doing.
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  configOf,
  existence,
  healthSays,
  listed,
  names,
  plan,
  removable,
  scratchName,
  servingVersion,
  uploadedVersion,
} from "./release-plan.mjs";

const here = dirname(fileURLToPath(import.meta.url));
// The PM_RELEASE_* settings exist for the tests, which stand in for
// Cloudflare and for the Worker's address (../test/release.test.ts).
const API = process.env.PM_RELEASE_API ?? "https://api.cloudflare.com/client/v4";
// A Worker's first address can take a while to answer; one already
// there answers a new version within seconds.
const HEALTH_TRIES = Number(process.env.PM_RELEASE_HEALTH_TRIES ?? 40);
const HEALTH_WAIT_MS = Number(process.env.PM_RELEASE_HEALTH_WAIT_MS ?? 6_000);
// Standing a Worker up from nothing is tried more than once. Cloudflare
// is sometimes not done making a new Worker when wrangler, having
// uploaded it, goes on to ask about it, and answers "This Worker does
// not exist" (code 10007); a moment later it does.
const STAND_UP_TRIES = Number(process.env.PM_RELEASE_STAND_UP_TRIES ?? 4);
const STAND_UP_WAIT_MS = Number(process.env.PM_RELEASE_STAND_UP_WAIT_MS ?? 5_000);

/** Stop before anything has been done: a bad argument, a missing setting. */
function fail(message) {
  console.error(`release.mjs: ${message}`);
  process.exit(1);
}

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const value = (name) => {
  const at = args.indexOf(name);
  return at >= 0 ? args[at + 1] : undefined;
};
for (const arg of args) {
  if (arg.startsWith("--") && !["--sha", "--scratch"].includes(arg)) fail(`unknown option ${arg}`);
}

let commit;
try {
  commit = names(value("--sha") ?? "");
} catch (error) {
  fail(error.message);
}
const token = process.env.CLOUDFLARE_API_TOKEN ?? "";
const account = process.env.CLOUDFLARE_ACCOUNT_ID ?? "";
if (token === "" || account === "") fail("CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID must be set");

const real = configOf(readFileSync(join(here, "wrangler.jsonc"), "utf8"));
const empty = configOf(readFileSync(join(here, "init", "wrangler.jsonc"), "utf8"));
const scratch = flag("--scratch");
const worker = scratch ? scratchName(real.name) : real.name;
const run = process.env.GITHUB_RUN_ID ? `workflow run ${process.env.GITHUB_RUN_ID}` : "a manual run";
// When this version says it was deployed. The tests pin it through the
// environment so the health gate's age check is deterministic.
const stampedAt = process.env.PM_RELEASE_DEPLOYED_AT ?? new Date().toISOString();

const say = (line) => console.log(`release.mjs: ${line}`);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function api(method, path) {
  const res = await fetch(`${API}${path}`, { method, headers: { authorization: `Bearer ${token}` } });
  const body = await res.json().catch(() => null);
  return { status: res.status, body };
}

async function look() {
  const { status, body } = await api("GET", `/accounts/${account}/workers/services/${worker}`);
  const found = existence(status, body);
  if (found.exists) return found;
  // Absence is answered by deploying an empty Worker under this name,
  // so it is asked a second way before it is believed.
  const list = await api("GET", `/accounts/${account}/workers/scripts`);
  if (listed(list.body, worker)) {
    throw new Error(`Cloudflare says ${worker} does not exist and also lists it; doing nothing`);
  }
  return found;
}

function wrangler(argv, outputFile) {
  const done = spawnSync("npx", ["wrangler", ...argv], {
    cwd: here,
    stdio: "inherit",
    env: { ...process.env, ...(outputFile ? { WRANGLER_OUTPUT_FILE_PATH: outputFile } : {}) },
  });
  if (done.status !== 0) throw new Error(`wrangler ${argv.slice(0, 2).join(" ")} failed`);
}

/** Stand an empty Worker up under this name. The command is the same
 * each time and leaves the same thing, an empty Worker, whether or not
 * an earlier try got as far as uploading it: so a try that fails is
 * simply made again, a few times, before the run is failed for it. */
async function standUp() {
  for (let attempt = 1; ; attempt++) {
    try {
      wrangler(["deploy", "--config", "init/wrangler.jsonc", "--name", worker, "--message", `empty Worker, stood up by ${run}`]);
      return;
    } catch (error) {
      if (attempt >= STAND_UP_TRIES) throw error;
      say(`standing ${worker} up did not go through (try ${attempt} of ${STAND_UP_TRIES}); trying again`);
      await sleep(STAND_UP_WAIT_MS);
    }
  }
}

async function serving() {
  const { status, body } = await api("GET", `/accounts/${account}/workers/scripts/${worker}/deployments`);
  return status === 200 ? servingVersion(body) : null;
}

let address = null;
/** What the last health request heard, for the line that says a wait failed. */
let heard = "nothing asked yet";
async function health() {
  if (!address && process.env.PM_RELEASE_HEALTH) address = `${process.env.PM_RELEASE_HEALTH}/${worker}`;
  if (!address) {
    const { status, body } = await api("GET", `/accounts/${account}/workers/subdomain`);
    const sub = body?.result?.subdomain;
    if (status !== 200 || typeof sub !== "string") throw new Error("cannot read the account's workers.dev subdomain");
    address = `https://${worker}.${sub}.workers.dev/console/health`;
  }
  // What came back that was not a health answer is kept, in a line, so
  // that a Worker that did not answer is not just "silent": an error
  // page, a refusal and no route to it are different faults.
  let res;
  try {
    res = await fetch(address, { signal: AbortSignal.timeout(10_000), headers: { "cache-control": "no-cache" } });
  } catch (error) {
    heard = `no answer (${error.name}: ${String(error.cause?.code ?? error.cause?.message ?? error.message).slice(0, 80)})`;
    return null;
  }
  const text = await res.text().catch(() => "");
  try {
    const answer = JSON.parse(text);
    heard = `http ${res.status}, ${text.slice(0, 160)}`;
    return answer;
  } catch {
    heard = `http ${res.status} ${res.headers.get("content-type") ?? "of no type"}: ${text.replace(/\s+/g, " ").slice(0, 160)}`;
    return null;
  }
}

/** Wait for the Worker to answer as `want` says it should. */
async function answers(want) {
  let last = "silent";
  for (let attempt = 0; attempt < HEALTH_TRIES; attempt++) {
    last = healthSays(await health(), commit.full);
    if (last === want) return true;
    await sleep(HEALTH_WAIT_MS);
  }
  say(`${worker} did not answer as ${want === "this" ? commit.short : want} (last answer: ${last}; it said: ${heard})`);
  return false;
}

async function remove() {
  if (!removable(worker, real.name)) {
    throw new Error(`refusing to remove ${worker}: only the scratch Worker is ever removed`);
  }
  const { status, body } = await api("DELETE", `/accounts/${account}/workers/scripts/${worker}?force=true`);
  if (status !== 200 && status !== 404) {
    throw new Error(`could not remove ${worker}: http ${status} ${JSON.stringify(body?.errors ?? null)}`);
  }
  say(status === 404 ? `${worker} was not there` : `removed ${worker}`);
}

/** The steps, in order. Says whether the Worker ended up serving this commit. */
async function release() {
  if (scratch) {
    say(`starting from nothing on ${worker}`);
    await remove();
  }
  const found = await look();
  const steps = plan(found, real.migrationTag);
  say(
    `${worker}: ${found.exists ? `there (migration ${found.migrationTag ?? "none"})` : "not there"}; steps: ${steps.join(", ")}`,
  );

  if (steps.includes("stand-up")) {
    await standUp();
    say(`stood up ${worker} empty`);
    // On the scratch Worker the empty one is waited for: that it answers
    // as empty is part of what the run is there to show.
    if (scratch && !(await answers("empty"))) return false;
  }

  if (steps.includes("deploy")) {
    wrangler([
      "deploy",
      "--name",
      worker,
      "--var",
      `VERSION:${commit.full}`,
      "--var",
      `DEPLOYED_AT:${stampedAt}`,
      "--message",
      commit.message,
    ]);
    say(`deployed ${commit.short} to ${worker} in one step (its Durable Object classes changed)`);
    return answers("this");
  }

  const before = await serving();
  const dir = mkdtempSync(join(tmpdir(), "pm-edge-release-"));
  const out = join(dir, "wrangler.ndjson");
  let version;
  try {
    wrangler(
      [
        "versions",
        "upload",
        "--name",
        worker,
        "--tag",
        commit.tag,
        "--message",
        commit.message,
        "--var",
        `VERSION:${commit.full}`,
        "--var",
        `DEPLOYED_AT:${stampedAt}`,
      ],
      out,
    );
    version = uploadedVersion(readFileSync(out, "utf8"), worker);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  say(`uploaded ${commit.short} to ${worker} as version ${version}; it serves nothing yet`);
  wrangler(["versions", "deploy", `${version}@100%`, "--name", worker, "--yes", "--message", commit.message]);
  say(`put version ${version} at 100%`);
  if (await answers("this")) return true;
  if (before && before !== version) {
    wrangler([
      "versions",
      "deploy",
      `${before}@100%`,
      "--name",
      worker,
      "--yes",
      "--message",
      `put back: ${commit.short} did not answer its health check`,
    ]);
    say(`put version ${before} back at 100%`);
  }
  return false;
}

let ok = false;
let trouble = null;
try {
  ok = await release();
} catch (error) {
  trouble = error;
}
// The scratch Worker never outlives its run, however the run went.
if (scratch) {
  try {
    await remove();
  } catch (error) {
    trouble ??= error;
  }
}
if (trouble) fail(trouble.message);
if (!ok) fail(`${worker} is not serving ${commit.short}`);
say(scratch ? `${worker} stood up from nothing, served ${commit.short}, and is gone again` : `${worker} is serving ${commit.short}`);
