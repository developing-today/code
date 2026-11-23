// The Worker's release (edge/release.mjs): what it decides, and what it
// does, run for real against a stand-in for Cloudflare's API and a
// stand-in for wrangler that records what it was asked to do.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  configOf,
  existence,
  healthSays,
  isVersionId,
  listed,
  names,
  plan,
  removable,
  scratchName,
  servingVersion,
  uploadedVersion,
} from "../edge/release-plan.mjs";

const EDGE = join(import.meta.dir, "..", "edge");
const SHA = "0123456789abcdef0123456789abcdef01234567";
const SHORT = SHA.slice(0, 12);
const V = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;

describe("what a release decides", () => {
  test("a commit's names", () => {
    expect(names(SHA)).toEqual({ short: SHORT, tag: `git-${SHORT}`, message: `git ${SHA}` });
    for (const bad of ["", "abc", SHORT, `${SHA}0`, SHA.toUpperCase(), `${SHA.slice(0, 39)}g`]) {
      expect(() => names(bad)).toThrow();
    }
  });

  test("the two configurations agree on everything that is not a version's own", () => {
    const real = configOf(readFileSync(join(EDGE, "wrangler.jsonc"), "utf8"));
    const empty = configOf(readFileSync(join(EDGE, "init", "wrangler.jsonc"), "utf8"));
    expect(real.name).toBe("pm-console-edge");
    expect(real.migrationTag).toBe("v1");
    // The empty Worker is stood up under the same name, with the same
    // class behind the same binding and the same migrations: that is
    // what lets the real one be uploaded over it as a version.
    for (const key of [
      "name",
      "compatibility_date",
      "compatibility_flags",
      "workers_dev",
      "preview_urls",
      "observability",
      "durable_objects",
      "migrations",
    ]) {
      expect(empty.config[key]).toEqual(real.config[key]);
    }
    expect(empty.migrationTag).toBe(real.migrationTag);
    // And it has nothing of the app in it.
    expect(empty.config.assets).toBeUndefined();
    expect(empty.config.main).toBe("worker.ts");
  });

  test("a configuration is read past its comments, and one that names no Worker is refused", () => {
    const text = `{
      // a line comment with "quotes" and a // second marker
      "name": "pm-x", /* a block
      comment */ "vars": { "URL": "https://example.test//path", "S": "a /* not a comment */ b" },
      "migrations": [{ "tag": "v1" }, { "tag": "v2", "new_sqlite_classes": ["A"] },],
    }`;
    const got = configOf(text);
    expect(got.name).toBe("pm-x");
    expect(got.migrationTag).toBe("v2");
    expect(got.config.vars).toEqual({ URL: "https://example.test//path", S: "a /* not a comment */ b" });
    expect(configOf('{"name":"pm-x"}').migrationTag).toBeNull();
    expect(() => configOf("{}")).toThrow();
    expect(() => configOf('{"name":"Not A Worker"}')).toThrow();
    expect(() => configOf('{"name":"x" /* never ends')).toThrow();
  });

  test("only a plain 'no such Worker' counts as absent", () => {
    const there = { success: true, result: { default_environment: { script: { migration_tag: "v1" } } } };
    expect(existence(200, there)).toEqual({ exists: true, migrationTag: "v1" });
    expect(existence(200, { success: true, result: {} })).toEqual({ exists: true, migrationTag: null });
    for (const code of [10007, 10090]) {
      expect(existence(404, { success: false, errors: [{ code }] })).toEqual({ exists: false, migrationTag: null });
    }
    // Anything else is not an answer, and nothing is deployed on it.
    expect(() => existence(404, { success: false, errors: [{ code: 7003 }] })).toThrow();
    expect(() => existence(404, null)).toThrow();
    expect(() => existence(403, { success: false, errors: [{ code: 10000 }] })).toThrow();
    expect(() => existence(500, { success: false, errors: [{ code: 10007 }] })).toThrow();
    expect(() => existence(200, { success: false })).toThrow();
  });

  test("the list of Workers is read by exact name", () => {
    const body = { success: true, result: [{ id: "pm-console-edge-scratch" }, { id: "message-bridge" }] };
    expect(listed(body, "pm-console-edge-scratch")).toBe(true);
    expect(listed(body, "pm-console-edge")).toBe(false);
    expect(() => listed({ success: false }, "x")).toThrow();
    expect(() => listed(null, "x")).toThrow();
  });

  test("the steps for what is there", () => {
    expect(plan({ exists: false, migrationTag: null }, "v1")).toEqual(["stand-up", "upload", "promote"]);
    expect(plan({ exists: true, migrationTag: "v1" }, "v1")).toEqual(["upload", "promote"]);
    // A migration the Worker has not had cannot travel in a version.
    expect(plan({ exists: true, migrationTag: "v1" }, "v2")).toEqual(["deploy"]);
    expect(plan({ exists: true, migrationTag: null }, "v1")).toEqual(["deploy"]);
  });

  test("only the scratch Worker may be removed", () => {
    expect(scratchName("pm-console-edge")).toBe("pm-console-edge-scratch");
    expect(removable("pm-console-edge-scratch", "pm-console-edge")).toBe(true);
    expect(removable("pm-console-edge", "pm-console-edge")).toBe(false);
    expect(removable("message-bridge", "pm-console-edge")).toBe(false);
    expect(removable("pm-console-edge-scratch-scratch", "pm-console-edge")).toBe(false);
    expect(removable("", "")).toBe(false);
  });

  test("the uploaded version is the one wrangler names for this Worker", () => {
    const lines = [
      JSON.stringify({ type: "wrangler-session", version: 1 }),
      "not json",
      JSON.stringify({ type: "version-upload", worker_name: "another", version_id: V(1) }),
      JSON.stringify({ type: "version-upload", worker_name: "pm-console-edge", version_id: V(2) }),
      "",
    ].join("\n");
    expect(uploadedVersion(lines, "pm-console-edge")).toBe(V(2));
    expect(() => uploadedVersion(lines, "pm-console-edge-scratch")).toThrow();
    expect(() => uploadedVersion("", "pm-console-edge")).toThrow();
    const odd = JSON.stringify({
      type: "version-upload",
      worker_name: "pm-console-edge",
      version_id: "latest; rm -rf",
    });
    expect(() => uploadedVersion(odd, "pm-console-edge")).toThrow();
  });

  test("the version to put back is the one serving everything, or none", () => {
    const one = { result: { deployments: [{ versions: [{ version_id: V(7), percentage: 100 }] }] } };
    expect(servingVersion(one)).toBe(V(7));
    const split = {
      result: {
        deployments: [
          {
            versions: [
              { version_id: V(7), percentage: 90 },
              { version_id: V(8), percentage: 10 },
            ],
          },
        ],
      },
    };
    expect(servingVersion(split)).toBeNull();
    expect(servingVersion({ result: { deployments: [] } })).toBeNull();
    expect(servingVersion(null)).toBeNull();
    expect(
      servingVersion({ result: { deployments: [{ versions: [{ version_id: "x", percentage: 100 }] }] } }),
    ).toBeNull();
    expect(isVersionId(V(1))).toBe(true);
    expect(isVersionId("latest")).toBe(false);
  });

  test("what a health answer means", () => {
    expect(healthSays({ ok: true, edge: true, version: SHORT }, SHORT)).toBe("this");
    expect(healthSays({ ok: true, edge: true, version: "ffffffffffff" }, SHORT)).toBe("another");
    expect(healthSays({ ok: false, edge: true, empty: true }, SHORT)).toBe("empty");
    // Something else answering on that address is not this Worker.
    expect(healthSays({ ok: true, version: SHORT }, SHORT)).toBe("silent");
    expect(healthSays(null, SHORT)).toBe("silent");
    expect(healthSays("<html>", SHORT)).toBe("silent");
  });
});

// ---- the release, run ------------------------------------------------------

interface FakeWorker {
  migrationTag: string | null;
  /** Version id to what that version answers on its health address. */
  versions: Map<string, { kind: "empty" } | { kind: "app"; short: string }>;
  serving: string | null;
}

describe("a release, run against stand-ins", () => {
  const account = "acct";
  const state = {
    workers: new Map<string, FakeWorker>(),
    /** What was done, in order: wrangler's commands and the API's changes. */
    did: [] as string[],
    next: 100,
    /** Knobs for what goes wrong. */
    uploadFails: false,
    appNeverAnswers: false,
    emptyNeverAnswers: false,
    existenceStatus: 0,
    hiddenFromServices: false,
    removeFails: false,
  };
  let server: ReturnType<typeof Bun.serve>;
  let bin: string;

  const json = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const flagValue = (argv: string[], name: string) => argv[argv.indexOf(name) + 1];

  /** wrangler's part: change the state the way the real command would. */
  const wrangler = (argv: string[]): { status: number; write?: string } => {
    state.did.push(`wrangler ${argv.join(" ")}`);
    const name = flagValue(argv, "--name");
    const id = V(state.next++);
    if (argv[0] === "deploy") {
      const config = flagValue(argv, "--config");
      const made = state.workers.get(name) ?? { migrationTag: null, versions: new Map(), serving: null };
      made.migrationTag = "v1";
      if (config === "init/wrangler.jsonc") made.versions.set(id, { kind: "empty" });
      else made.versions.set(id, { kind: "app", short: flagValue(argv, "--var").replace("VERSION:", "") });
      made.serving = id;
      state.workers.set(name, made);
      return { status: 0 };
    }
    const worker = state.workers.get(name);
    if (!worker) return { status: 1 };
    if (argv[0] === "versions" && argv[1] === "upload") {
      if (state.uploadFails) return { status: 1 };
      worker.versions.set(id, { kind: "app", short: flagValue(argv, "--var").replace("VERSION:", "") });
      return {
        status: 0,
        write: JSON.stringify({ type: "version-upload", version: 1, worker_name: name, version_id: id }),
      };
    }
    if (argv[0] === "versions" && argv[1] === "deploy") {
      const version = argv[2].replace("@100%", "");
      if (!worker.versions.has(version) || !argv.includes("--yes")) return { status: 1 };
      worker.serving = version;
      return { status: 0 };
    }
    return { status: 1 };
  };

  beforeAll(() => {
    server = Bun.serve({
      port: 0,
      hostname: "127.0.0.1",
      async fetch(req) {
        const url = new URL(req.url);
        const path = url.pathname;
        if (path === "/_wrangler") return json(wrangler((await req.json()) as string[]));
        const health = path.match(/^\/health\/([^/]+)$/);
        if (health) {
          const worker = state.workers.get(health[1]);
          const serving = worker?.serving ? worker.versions.get(worker.serving) : undefined;
          if (!serving) return new Response("no such Worker", { status: 404 });
          if (serving.kind === "empty") {
            if (state.emptyNeverAnswers) return new Response("error code: 1042", { status: 404 });
            return json({ ok: false, edge: true, empty: true }, 503);
          }
          if (state.appNeverAnswers) return new Response("error code: 1101", { status: 500 });
          return json({ ok: true, edge: true, version: serving.short });
        }
        if (req.headers.get("authorization") !== "Bearer cf-test")
          return json({ success: false, errors: [{ code: 10000 }] }, 403);
        const base = `/api/accounts/${account}/workers`;
        const service = path.match(new RegExp(`^${base}/services/([^/]+)$`));
        if (service && req.method === "GET") {
          if (state.existenceStatus) return json({ success: false, errors: [{ code: 10000 }] }, state.existenceStatus);
          const worker = state.workers.get(service[1]);
          if (!worker || state.hiddenFromServices) return json({ success: false, errors: [{ code: 10090 }] }, 404);
          return json({
            success: true,
            result: { default_environment: { script: { migration_tag: worker.migrationTag } } },
          });
        }
        if (path === `${base}/scripts` && req.method === "GET") {
          return json({ success: true, result: [...state.workers.keys()].map((id) => ({ id })) });
        }
        const deployments = path.match(new RegExp(`^${base}/scripts/([^/]+)/deployments$`));
        if (deployments && req.method === "GET") {
          const worker = state.workers.get(deployments[1]);
          if (!worker) return json({ success: false, errors: [{ code: 10007 }] }, 404);
          const versions = worker.serving ? [{ version_id: worker.serving, percentage: 100 }] : [];
          return json({ success: true, result: { deployments: [{ versions }] } });
        }
        const script = path.match(new RegExp(`^${base}/scripts/([^/]+)$`));
        if (script && req.method === "DELETE") {
          state.did.push(`api DELETE ${script[1]} force=${url.searchParams.get("force")}`);
          if (state.removeFails) return json({ success: false, errors: [{ code: 10013 }] }, 500);
          if (!state.workers.delete(script[1])) return json({ success: false, errors: [{ code: 10007 }] }, 404);
          return json({ success: true, result: null });
        }
        return json({ success: false, errors: [{ code: 7003 }] }, 404);
      },
    });
    // A stand-in for `npx`: it tells the server what wrangler was asked,
    // and does what the server says a real run would have done.
    bin = mkdtempSync(join(tmpdir(), "pm-release-test-"));
    writeFileSync(
      join(bin, "npx"),
      `#!/usr/bin/env bun
const argv = process.argv.slice(2);
if (argv[0] !== "wrangler") process.exit(64);
const res = await fetch(process.env.FAKE_CONTROL, { method: "POST", body: JSON.stringify(argv.slice(1)) });
const { status, write } = await res.json();
if (write) (await import("node:fs")).appendFileSync(process.env.WRANGLER_OUTPUT_FILE_PATH, write + "\\n");
process.exit(status);
`,
    );
    chmodSync(join(bin, "npx"), 0o755);
  });

  afterAll(() => {
    server.stop(true);
    rmSync(bin, { recursive: true, force: true });
  });

  const reset = (workers: Record<string, FakeWorker> = {}) => {
    state.workers = new Map(Object.entries(workers));
    state.did = [];
    state.next = 100;
    state.uploadFails = false;
    state.appNeverAnswers = false;
    state.emptyNeverAnswers = false;
    state.existenceStatus = 0;
    state.hiddenFromServices = false;
    state.removeFails = false;
  };
  const worker = (short: string, migrationTag: string | null = "v1"): FakeWorker => ({
    migrationTag,
    versions: new Map([[V(1), { kind: "app", short }]]),
    serving: V(1),
  });
  const release = async (...args: string[]) => {
    const origin = `http://127.0.0.1:${server.port}`;
    const run = Bun.spawn(["bun", join(EDGE, "release.mjs"), "--sha", SHA, ...args], {
      env: {
        ...process.env,
        PATH: `${bin}:${process.env.PATH}`,
        CLOUDFLARE_API_TOKEN: "cf-test",
        CLOUDFLARE_ACCOUNT_ID: account,
        PM_RELEASE_API: `${origin}/api`,
        PM_RELEASE_HEALTH: `${origin}/health`,
        PM_RELEASE_HEALTH_TRIES: "3",
        PM_RELEASE_HEALTH_WAIT_MS: "5",
        FAKE_CONTROL: `${origin}/_wrangler`,
        GITHUB_RUN_ID: "77",
      },
      stdout: "pipe",
      stderr: "pipe",
    });
    const [out, err, code] = await Promise.all([
      new Response(run.stdout).text(),
      new Response(run.stderr).text(),
      run.exited,
    ]);
    return { code, out, err };
  };
  const serving = (name: string) => {
    const w = state.workers.get(name);
    return w?.serving ? w.versions.get(w.serving) : undefined;
  };
  const REAL = "pm-console-edge";
  const SCRATCH = "pm-console-edge-scratch";

  test("a Worker that is there gets a new version, uploaded and then promoted", async () => {
    reset({ [REAL]: worker("aaaaaaaaaaaa") });
    const got = await release();
    expect(got.code).toBe(0);
    expect(state.did).toEqual([
      `wrangler versions upload --name ${REAL} --tag git-${SHORT} --message git ${SHA} --var VERSION:${SHORT}`,
      `wrangler versions deploy ${V(100)}@100% --name ${REAL} --yes --message git ${SHA}`,
    ]);
    expect(serving(REAL)).toEqual({ kind: "app", short: SHORT });
    expect(got.out).toContain(`${REAL} is serving ${SHORT}`);
  });

  test("with no Worker there, an empty one is stood up first and the version goes over it", async () => {
    reset();
    const got = await release();
    expect(got.code).toBe(0);
    expect(state.did).toEqual([
      `wrangler deploy --config init/wrangler.jsonc --name ${REAL} --message empty Worker, stood up by workflow run 77`,
      `wrangler versions upload --name ${REAL} --tag git-${SHORT} --message git ${SHA} --var VERSION:${SHORT}`,
      `wrangler versions deploy ${V(101)}@100% --name ${REAL} --yes --message git ${SHA}`,
    ]);
    expect(serving(REAL)).toEqual({ kind: "app", short: SHORT });
  });

  test("a Worker whose classes differ from the configuration gets one full deploy", async () => {
    reset({ [REAL]: worker("aaaaaaaaaaaa", "v0") });
    const got = await release();
    expect(got.code).toBe(0);
    expect(state.did).toEqual([`wrangler deploy --name ${REAL} --var VERSION:${SHORT} --message git ${SHA}`]);
    expect(serving(REAL)).toEqual({ kind: "app", short: SHORT });
    // If what it deployed does not answer, the run fails. There is no
    // putting back across a change of classes.
    reset({ [REAL]: worker("aaaaaaaaaaaa", "v0") });
    state.appNeverAnswers = true;
    const bad = await release();
    expect(bad.code).toBe(1);
    expect(state.did.length).toBe(1);
  });

  test("a version that does not answer is taken off again, and the run fails", async () => {
    reset({ [REAL]: worker("aaaaaaaaaaaa") });
    state.appNeverAnswers = true;
    const got = await release();
    expect(got.code).toBe(1);
    expect(state.did).toEqual([
      `wrangler versions upload --name ${REAL} --tag git-${SHORT} --message git ${SHA} --var VERSION:${SHORT}`,
      `wrangler versions deploy ${V(100)}@100% --name ${REAL} --yes --message git ${SHA}`,
      `wrangler versions deploy ${V(1)}@100% --name ${REAL} --yes --message put back: ${SHORT} did not answer its health check`,
    ]);
    // What was serving before is serving again.
    expect(serving(REAL)).toEqual({ kind: "app", short: "aaaaaaaaaaaa" });
    expect(got.err).toContain(`${REAL} is not serving ${SHORT}`);
    // And the run says what it heard in place of a health answer: an
    // error page is a different fault from no route, and "silent" alone
    // told them apart for nobody.
    expect(got.out + got.err).toContain("(last answer: silent; it said: http 500 ");
    expect(got.out + got.err).toContain("error code: 1101");
  });

  test("an upload that fails changes nothing that is serving", async () => {
    reset({ [REAL]: worker("aaaaaaaaaaaa") });
    state.uploadFails = true;
    const got = await release();
    expect(got.code).toBe(1);
    expect(state.did.length).toBe(1);
    expect(serving(REAL)).toEqual({ kind: "app", short: "aaaaaaaaaaaa" });
  });

  test("nothing is stood up on an answer that is not plainly 'no such Worker'", async () => {
    // Refused: the token cannot see Workers.
    reset({ [REAL]: worker("aaaaaaaaaaaa") });
    state.existenceStatus = 403;
    let got = await release();
    expect(got.code).toBe(1);
    expect(state.did).toEqual([]);
    // Two answers that disagree: one says it is not there, the list has it.
    reset({ [REAL]: worker("aaaaaaaaaaaa") });
    state.hiddenFromServices = true;
    got = await release();
    expect(got.code).toBe(1);
    expect(state.did).toEqual([]);
    expect(got.err).toContain("doing nothing");
    expect(serving(REAL)).toEqual({ kind: "app", short: "aaaaaaaaaaaa" });
  });

  test("the scratch run starts from nothing, shows each step, and leaves nothing behind", async () => {
    // A scratch Worker left by a run that was cut off, and the real one in service.
    reset({ [REAL]: worker("aaaaaaaaaaaa"), [SCRATCH]: worker("bbbbbbbbbbbb", "v0") });
    const got = await release("--scratch");
    expect(got.code).toBe(0);
    expect(state.did).toEqual([
      `api DELETE ${SCRATCH} force=true`,
      `wrangler deploy --config init/wrangler.jsonc --name ${SCRATCH} --message empty Worker, stood up by workflow run 77`,
      `wrangler versions upload --name ${SCRATCH} --tag git-${SHORT} --message git ${SHA} --var VERSION:${SHORT}`,
      `wrangler versions deploy ${V(101)}@100% --name ${SCRATCH} --yes --message git ${SHA}`,
      `api DELETE ${SCRATCH} force=true`,
    ]);
    expect(state.workers.has(SCRATCH)).toBe(false);
    // The real Worker was never named.
    expect(serving(REAL)).toEqual({ kind: "app", short: "aaaaaaaaaaaa" });
    expect(got.out).toContain("stood up from nothing");
  });

  test("the scratch Worker is removed even when its run fails", async () => {
    reset({ [REAL]: worker("aaaaaaaaaaaa") });
    state.uploadFails = true;
    let got = await release("--scratch");
    expect(got.code).toBe(1);
    expect(state.did.at(-1)).toBe(`api DELETE ${SCRATCH} force=true`);
    expect(state.workers.has(SCRATCH)).toBe(false);
    // And when its version never answers.
    reset({ [REAL]: worker("aaaaaaaaaaaa") });
    state.appNeverAnswers = true;
    got = await release("--scratch");
    expect(got.code).toBe(1);
    expect(state.workers.has(SCRATCH)).toBe(false);
    expect(state.workers.has(REAL)).toBe(true);
  });

  test("the scratch run waits to hear the empty Worker before it goes on", async () => {
    reset({ [REAL]: worker("aaaaaaaaaaaa") });
    state.emptyNeverAnswers = true;
    const got = await release("--scratch");
    expect(got.code).toBe(1);
    // It was stood up, never answered as empty, and was removed: no version was uploaded to it.
    expect(state.did).toEqual([
      `api DELETE ${SCRATCH} force=true`,
      `wrangler deploy --config init/wrangler.jsonc --name ${SCRATCH} --message empty Worker, stood up by workflow run 77`,
      `api DELETE ${SCRATCH} force=true`,
    ]);
    // The real Worker's stand-up does not wait on it: the upload follows at once.
    reset();
    state.emptyNeverAnswers = true;
    expect((await release()).code).toBe(0);
    expect(serving(REAL)).toEqual({ kind: "app", short: SHORT });
  });

  test("a scratch Worker that cannot be removed fails the run", async () => {
    reset({ [SCRATCH]: worker("bbbbbbbbbbbb") });
    state.removeFails = true;
    const got = await release("--scratch");
    expect(got.code).toBe(1);
    // Nothing was stood up on top of what could not be cleared.
    expect(state.did.filter((line) => line.startsWith("wrangler"))).toEqual([]);
  });

  test("bad arguments stop it before anything is asked or done", async () => {
    reset({ [REAL]: worker("aaaaaaaaaaaa") });
    const got = await release("--name", "message-bridge");
    expect(got.code).toBe(1);
    expect(got.err).toContain("unknown option --name");
    expect(state.did).toEqual([]);
  });
});
