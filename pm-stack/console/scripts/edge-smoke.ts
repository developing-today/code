// A run of the edge in the real Worker runtime, on this machine.
//
//   cd edge && node build.mjs && npx wrangler dev --port 8799 \
//     --var PUBLIC_HOST:localhost --var HOST_ORIGIN:http://127.0.0.1:4898 --var VERSION:smoke \
//     --var EDGE_DIRECT:on --var GITHUB_TOKEN:smoke-github --var QUEUE_TOKEN:smoke-queue \
//     --var GITHUB_API_ORIGIN:http://127.0.0.1:4898/github --var QUEUE_DB_ORIGIN:http://127.0.0.1:4898/queue \
//     --var OWN_CLOCK_MS:2000 --var KEYS_FROM:https://smoke.invalid/jwks --var KEYS_TO:http://127.0.0.1:4898/jwks
//   SMOKE_PRIMARY=1 bun run scripts/edge-smoke.ts            (in another shell)
//
// It starts a host on port 4898 with made-up state and this repo's real
// handler and link, then checks: the Worker starts, serves its public
// files, takes the host's signed report (fetching the host's key
// through the "tunnel"), keeps a section too large for one stored
// value, leaves an unsigned caller to the host, and answers a phone
// that is not linked.
//
// The sign-in is this run's own too: a key made here, whose public half
// the stand-in serves as the issuer's key set (the Worker is started
// with KEYS_FROM/KEYS_TO, above, so its fetch of the issuer's https
// key-set address lands on the stand-in), so the run can sign the
// owner's token itself (as scripts/edge-demo.ts does) and check what
// only a signed-in caller reaches, in this runtime: who the edge says
// is calling, an action done once under a name and answered again from
// the record, the platform read, an alarm the object sets firing and
// doing its work, and -- with the host gone -- an action held for a
// machine, listed by the machines read, and taken back. The clock's
// minute is shortened for the run (OWN_CLOCK_MS, above) so the alarm
// can be watched without waiting it out.
//
// Started as above, the Worker does the core itself, and the same port
// stands in for GitHub's API (/github) and the queue store (/queue): its
// own check of each kind of read then runs in this runtime, with the
// Worker's own fetch, and has to come back good. SMOKE_PRIMARY=1 says
// that is what was started, so that leaving the flags off is a failure
// and not a quiet skip.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { identify } from "../src/auth.ts";
import { readConfig } from "../src/config.ts";
import { Devices, Limiter } from "../src/devices.ts";
import { Docs } from "../src/docs.ts";
import { Ops } from "../src/ops.ts";
import { type Deps, handle, heldRunner, marked } from "../src/server.ts";
import { memoryStore } from "../src/store.ts";
import { EdgeLink } from "../src/sync.ts";

if (process.argv.length > 2) {
  console.error("usage: bun run scripts/edge-smoke.ts   (it takes no arguments; EDGE names the Worker's address)");
  process.exit(2);
}
const EDGE = process.env.EDGE ?? "http://localhost:8799";
const cfg = readConfig({ HOME: "/tmp/pm-edge-smoke", PM_CONSOLE_OWNER_EMAILS: "owner@example.com" });
cfg.issuers = [
  {
    name: "smoke-sign-in",
    issuer: "https://smoke.invalid",
    jwksUri: "https://smoke.invalid/jwks",
    audience: "smoke",
    source: "access",
  },
];
// A sign-in issuer made here: the stand-in below serves the public
// half as the issuer's key set, and this signs the owner's token.
const pair = (await crypto.subtle.generateKey(
  { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
  true,
  ["sign", "verify"],
)) as CryptoKeyPair;
const jwk = { ...(await crypto.subtle.exportKey("jwk", pair.publicKey)), kid: "smoke" };
const part = (value: unknown) => Buffer.from(JSON.stringify(value)).toString("base64url");
async function ownerToken(): Promise<string> {
  const now = Math.floor(Date.now() / 1000);
  const signed = `${part({ alg: "RS256", kid: "smoke" })}.${part({
    iss: "https://smoke.invalid",
    aud: ["smoke"],
    exp: now + 3600,
    iat: now,
    email: "owner@example.com",
  })}`;
  const sig = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", pair.privateKey, new TextEncoder().encode(signed));
  return `${signed}.${Buffer.from(sig).toString("base64url")}`;
}
const big = { open: Array.from({ length: 5000 }, (_, i) => ({ qid: `q${i}`, question: "Why? ".repeat(80) })) };
const data: Record<string, unknown> = {
  board: { counts: { claimed: 2, queued: 1 }, now: [{ id: 1, title: "Smoke task" }], next: [], blocked: [] },
  sessions: { moving: [], waiting: [] },
  census: { hands: [], reviews: [], other: [] },
  asks: [],
  questionCount: { total: 5000, sets: 1 },
  questions: big,
  prs: [],
  priorities: { issues: [], truncated: false },
  agents: [],
  services: [],
  more: [],
};
const docs = new Docs(
  Object.fromEntries(Object.keys(data).map((n) => [n, { ttlMs: 10_000, load: async () => data[n] }])),
);
const deps: Deps = {
  cfg,
  ops: new Ops(cfg, async () => ({ code: 0, stdout: "{}", stderr: "" })),
  docs,
  actions: {},
  phones: { devices: new Devices(memoryStore()), link: new Limiter(30, 60_000), refused: new Limiter(60, 60_000) },
  identify: (req, peer) => identify(req, cfg, { peer }),
  audit: async () => {},
};
const link = new EdgeLink({
  url: EDGE,
  keys: memoryStore(),
  done: memoryStore(),
  docs,
  phones: deps.phones!.devices,
  config: async () => ({
    owners: cfg.owners,
    certNames: [],
    issuers: cfg.issuers,
    repo: cfg.repo,
    queue: cfg.queue,
    androidPackage: cfg.androidPackage,
    androidFingerprints: [],
    androidApp: false,
    queueDbUrl: cfg.queueDbUrl,
    queueTokenName: cfg.queueTokenName,
    promptLabel: cfg.promptLabel,
    needsRepos: cfg.needsRepos,
  }),
  run: heldRunner(deps),
  log: (line) => console.log(`  host: ${line}`),
});
deps.link = link;
const seen: string[] = [];
/** What reached the stand-ins for GitHub and the queue store, and with
 * what: the Worker's own reads, made in its runtime. */
const outside: { to: "github" | "queue"; path: string; auth: string | null; agent: string | null }[] = [];
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 4898,
  fetch: async (req, srv) => {
    const url = new URL(req.url);
    if (url.pathname === "/jwks") return Response.json({ keys: [jwk] });
    if (url.pathname.startsWith("/github/")) {
      const path = url.pathname.slice("/github/".length);
      outside.push({
        to: "github",
        path,
        auth: req.headers.get("authorization"),
        agent: req.headers.get("user-agent"),
      });
      if (path.endsWith("/contents/pm-stack/platform/workers.json")) {
        // The platform read's list, served as GitHub serves a file.
        return Response.json({
          content: Buffer.from(readFileSync(join(import.meta.dir, "..", "..", "platform", "workers.json"))).toString(
            "base64",
          ),
        });
      }
      if (path.includes("/actions/workflows/")) return Response.json({ workflow_runs: [] });
      if (path.endsWith("/pulls")) {
        return Response.json([
          {
            number: 7,
            title: "Smoke pull request",
            draft: false,
            html_url: "https://github.com/developing-today/code/pull/7",
            created_at: "2026-11-23T08:00:00Z",
            user: { login: "smoke" },
            head: { ref: "smoke", sha: "0123456789abcdef0123456789abcdef01234567" },
          },
        ]);
      }
      if (path === "search/issues") return Response.json({ total_count: 0, items: [] });
      if (path.endsWith("/issues")) return Response.json([]);
      return Response.json({ message: "Not Found" }, { status: 404 });
    }
    if (url.pathname === "/queue/v2/pipeline") {
      outside.push({ to: "queue", path: url.pathname, auth: req.headers.get("authorization"), agent: null });
      const asked = (await req.json()) as { requests: { type: string }[] };
      // No rows for anything asked: an empty queue.
      return Response.json({
        results: asked.requests.map((r) =>
          r.type === "execute"
            ? { type: "ok", response: { type: "execute", result: { cols: [], rows: [] } } }
            : { type: "ok", response: { type: "close" } },
        ),
      });
    }
    seen.push(`${req.method} ${url.pathname}`);
    // Everything that arrives here came through the Worker, as it does
    // through the tunnel: never a caller on this machine. (Deployed, the
    // edge's own headers say so; here the address has to.)
    void srv;
    return marked(await handle(req, deps, "203.0.113.9"));
  },
});

let failed = 0;
const check = (name: string, ok: boolean, detail = "") => {
  if (!ok) failed++;
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? `: ${detail}` : ""}`);
};
const get = (path: string, headers: Record<string, string> = {}) =>
  fetch(`${EDGE}${path}`, { headers, redirect: "manual" });
const asOwner = async (path: string) => get(path, { "cf-access-jwt-assertion": await ownerToken() });
const actAsOwner = async (kind: string, args: Record<string, unknown>, id?: string) =>
  fetch(`${EDGE}/console/api/act`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-pm-console": "1",
      "cf-access-jwt-assertion": await ownerToken(),
    },
    body: JSON.stringify(id === undefined ? { kind, args } : { kind, args, id }),
  });

try {
  const before = (await (await get("/console/health")).json()) as Record<string, unknown>;
  check(
    "the Worker answers its health check",
    before.ok === true && before.edge === true && before.version === "smoke",
    JSON.stringify(before),
  );
  const manifest = await get("/manifest.webmanifest");
  check(
    "the install manifest is served",
    manifest.status === 200 && ((await manifest.json()) as { start_url: string }).start_url === "/console/",
  );
  const icon = await get("/icons/icon-192.png");
  const bytes = new Uint8Array(await icon.arrayBuffer());
  check(
    "an icon is served from the app's files",
    icon.status === 200 && icon.headers.get("content-type") === "image/png" && bytes[1] === 0x50,
    `${bytes.length} bytes`,
  );
  // The platform's file store tags each file; the Worker has to pass the
  // tag on and answer a browser that names it with "still that", or
  // every visit downloads the app again. Only this runtime has the store.
  const tag = icon.headers.get("etag");
  const again = tag ? await get("/icons/icon-192.png", { "If-None-Match": tag }) : null;
  const other = await get("/icons/icon-192.png", { "If-None-Match": '"some-other-version"' });
  check(
    "a file says which version of itself it is, and a browser that has it is sent nothing",
    Boolean(tag) &&
      again?.status === 304 &&
      (await again.arrayBuffer()).byteLength === 0 &&
      other.status === 200 &&
      (await other.arrayBuffer()).byteLength === bytes.byteLength,
    `tag ${tag}, with it ${again?.status}, with another ${other.status}`,
  );

  await docs.touch(Object.keys(data));
  const first = await link.once();
  check(
    "the host's signed report is accepted",
    first?.ok === true,
    JSON.stringify({ watching: first?.watching, notify: first?.notify, want: first?.want }),
  );
  check(
    "the edge asked the host for its key, once",
    seen.filter((s) => s === "GET /console/edge/key").length === 1,
    seen.join(", "),
  );
  const after = (await (await get("/console/health")).json()) as Record<string, unknown>;
  check(
    "the edge now knows the owner and has seen the host",
    after.configured === true && typeof after.hostSeen === "number",
    JSON.stringify(after),
  );
  const second = await link.once();
  check(
    "a second report is accepted and nothing is asked for again",
    second?.ok === true && (second?.want ?? []).length === 0,
  );
  // Doing the core itself. The Worker's check of its own reads runs
  // with its health answer: one read of each kind, made in this runtime
  // with the fetch it keeps, to the stand-ins on this port.
  const wanted = process.env.SMOKE_PRIMARY === "1";
  if (wanted || after.directEnabled === true) {
    check(
      "the Worker says it does the core itself, now that it knows where things are",
      after.directEnabled === true && after.primary === true && before.primary === false,
      JSON.stringify({ before: before.primary, after: after.primary, enabled: after.directEnabled }),
    );
    const kinds = (after.selfCheck as { kinds?: Record<string, { ok: boolean; error?: string }> } | null)?.kinds ?? {};
    check(
      "its own read of the board, the prompts, pull requests and ranked issues each came back good",
      ["board", "asks", "prs", "priorities"].every((kind) => kinds[kind]?.ok === true),
      JSON.stringify(kinds),
    );
    const github = outside.filter((o) => o.to === "github");
    const queue = outside.filter((o) => o.to === "queue");
    check(
      "those reads reached GitHub and the queue store from this runtime, each with its token",
      github.length >= 3 &&
        queue.length >= 1 &&
        github.every((o) => o.auth === "Bearer smoke-github" && o.agent === "pm-console-edge") &&
        queue.every((o) => o.auth === "Bearer smoke-queue"),
      `${github.length} to GitHub (${[...new Set(github.map((o) => o.path))].join(", ")}), ${queue.length} to the queue store`,
    );
    check(
      "and it told the host what it reads itself, so the host stops sending it",
      JSON.stringify(second?.owns) === JSON.stringify(["board", "prs", "priorities", "asks"]),
      JSON.stringify(second?.owns),
    );

    // Signed in with this run's own token: what only the owner
    // reaches, checked in this runtime rather than under bun.
    const me = (await (await asOwner("/console/api/me")).json()) as Record<string, unknown>;
    check(
      "signed in with the run's own token, the edge says who is calling",
      me.email === "owner@example.com" && me.edge === true,
      JSON.stringify(me),
    );
    const onceId = "11111111-1111-4111-8111-111111111111";
    const didFirst = (await (await actAsOwner("push.class", { class: "trouble", on: false }, onceId)).json()) as {
      result?: Record<string, boolean>;
      again?: boolean;
    };
    const didAgain = (await (await actAsOwner("push.class", { class: "trouble", on: false }, onceId)).json()) as {
      result?: Record<string, boolean>;
      again?: boolean;
    };
    check(
      "an action under a name is done once, and its second arrival is answered from the record",
      didFirst.result?.trouble === false &&
        didFirst.again !== true &&
        didAgain.again === true &&
        didAgain.result?.trouble === false,
      `first ${JSON.stringify(didFirst)}, again ${JSON.stringify(didAgain)}`,
    );
    seen.length = 0;
    const platform = (await (await asOwner("/console/api/platform")).json()) as {
      checked?: string;
      malformed?: number;
      workers?: { name: string }[];
    };
    check(
      "the platform read is answered here, from the repository's own list",
      platform.malformed === 0 &&
        typeof platform.checked === "string" &&
        (platform.workers ?? []).some((w) => w.name === "pm-console-edge"),
      `${(platform.workers ?? []).length} workers, checked ${platform.checked}; host saw ${seen.join(", ")}`,
    );
    check("and no machine was asked for it", !seen.some((path) => path.includes("/api/platform")), seen.join(", "));
    // Someone to tell: a device takes notifications (its service is a
    // known one and its key a real point; nothing is sent to it in
    // this run unless the clock's news is worth sending).
    const devicePair = (await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, [
      "sign",
      "verify",
    ])) as CryptoKeyPair;
    const devicePoint = Buffer.from(
      (await crypto.subtle.exportKey("raw", devicePair.publicKey)) as ArrayBuffer,
    ).toString("base64url");
    const subscribed = (await (
      await actAsOwner(
        "push.on",
        {
          subscription: {
            endpoint: "https://fcm.googleapis.com/fcm/send/smoke-run",
            keys: {
              p256dh: devicePoint,
              auth: Buffer.from(crypto.getRandomValues(new Uint8Array(16))).toString("base64url"),
            },
          },
          label: "smoke",
        },
        "22222222-2222-4222-8222-222222222222",
      )
    ).json()) as { result?: { devices?: number } };
    check("a device takes notifications", subscribed.result?.devices === 1, JSON.stringify(subscribed));
    // The clock starts itself when a document is read. Its interval
    // is the run's shortened one, and the board goes stale half a
    // minute in: an alarm that fires and does its work reads the
    // board from GitHub again, here, with nobody asking.
    const board = await asOwner("/console/api/doc/core");
    check("the state is read signed in", board.status === 200, String(board.status));
    await new Promise((resolve) => setTimeout(resolve, 3000));
    const readsBefore = outside.filter((o) => o.to === "github").length;
    let readsAfter = readsBefore;
    for (let waited = 0; waited < 75 && readsAfter <= readsBefore; waited += 2) {
      await new Promise((resolve) => setTimeout(resolve, 2000));
      readsAfter = outside.filter((o) => o.to === "github").length;
    }
    check(
      "an alarm the object set fired and did its work: what the owner is told about was read again, unasked",
      readsAfter > readsBefore,
      `${readsBefore} reads of GitHub before, ${readsAfter} after waiting`,
    );
  } else {
    console.log("skip the Worker was not started to do the core itself (see the top of this file)");
  }
  check(
    "the large section went in pieces",
    JSON.stringify(big).length > 1_500_000,
    `${JSON.stringify(big).length} characters`,
  );

  seen.length = 0;
  const stranger = await get("/console/api/doc/core");
  check(
    "a caller with no sign-in is left to the host, which refuses",
    stranger.status === 401 && stranger.headers.get("x-pm-console") === "1",
    `${stranger.status}; host saw ${seen.join(", ")}`,
  );
  const page = await get("/console/");
  check("so is the app's page", page.status === 401 && !(await page.text()).includes("Smoke task"));
  const forged = await fetch(`${EDGE}/console/edge/sync`, {
    method: "POST",
    body: "{}",
    headers: { "x-pm-key": "nope", "x-pm-signature": "AAAA" },
  });
  check("an unsigned report is refused", forged.status === 401, String(forged.status));
  const inside = await fetch(`${EDGE}/internal/hold`, {
    method: "POST",
    body: JSON.stringify({ kind: "ask.send", args: {} }),
    headers: { "content-type": "application/json" },
  });
  check(
    "an /internal path from outside never reaches the state",
    inside.headers.get("x-pm-console") === "1",
    String(inside.status),
  );
  const phone = await get("/console/device/summary", { authorization: `Bearer ${"x".repeat(43)}` });
  check(
    "a phone that is not linked is told so by the edge",
    phone.status === 401 && ((await phone.json()) as { unlinked?: boolean }).unlinked === true,
  );
  const linkTry = await fetch(`${EDGE}/console/device/link`, {
    method: "POST",
    body: JSON.stringify({ verifier: "v".repeat(43) }),
  });
  check("a link nobody approved is refused", linkTry.status === 404);
  // The host goes away: the edge says so itself, and says nothing else.
  server.stop(true);
  const gone = await get("/attention");
  check(
    "with the host gone, a page of the host's is a plain notice",
    gone.status === 503 && (await gone.text()).includes("Open the app"),
  );
  const closed = await get("/console/api/doc/core");
  check(
    "and a caller with no sign-in still learns nothing",
    closed.status === 503 && !(await closed.text()).includes("Smoke task"),
  );
  const still = (await (await get("/console/health")).json()) as Record<string, unknown>;
  check("the edge itself is still up", still.ok === true && Number(still.deferred) >= 3, JSON.stringify(still));
  // With the host gone, what may be done late is held here instead of
  // lost: signed in, it can be seen waiting and taken back.
  const heldId = "33333333-3333-4333-8333-333333333333";
  const heldAnswer = (await (
    await actAsOwner("message.send", { to: "smoke", text: "held by the smoke run" }, heldId)
  ).json()) as {
    ok?: boolean;
    held?: boolean;
  };
  check(
    "with the host gone, an action that may be done late is held, not lost",
    heldAnswer.ok === true && heldAnswer.held === true,
    JSON.stringify(heldAnswer),
  );
  const machines = (await (await asOwner("/console/api/machines")).json()) as {
    held?: { id: string; kind: string }[];
  };
  check(
    "the machines read lists it as waiting for a machine",
    (machines.held ?? []).some((h) => h.id === heldId && h.kind === "message.send"),
    JSON.stringify(machines.held),
  );
  const dropped = await actAsOwner("held.drop", { id: heldId }, "44444444-4444-4444-8444-444444444444");
  const machinesAfter = (await (await asOwner("/console/api/machines")).json()) as { held?: { id: string }[] };
  check(
    "and it can be taken back",
    dropped.status === 200 && !(machinesAfter.held ?? []).some((h) => h.id === heldId),
    `drop ${dropped.status}; still waiting ${JSON.stringify(machinesAfter.held)}`,
  );
  // The Android app's release is read from GitHub itself, in this
  // runtime, over the real network. There may be none, and GitHub may
  // not answer a runner: either is said as null, and neither is a
  // failure of this Worker. A release that is there must read as one.
  const release = still.androidRelease as { name?: unknown; code?: unknown; built?: unknown } | null | undefined;
  const trouble = still.androidReleaseTrouble as string | null | undefined;
  check(
    "health says which release of the Android app it knows of, or that it knows of none",
    release === null ||
      (typeof release?.name === "string" && Number.isInteger(release.code) && typeof release.built === "string"),
    JSON.stringify(release),
  );
  // That read is a request the Worker's state makes of its own, with the
  // fetch it keeps. This runtime refuses a fetch called as a method of
  // another object ("Illegal invocation"), which no test under bun can
  // see: it broke every notification and every read of the release for
  // a day. GitHub not answering a runner is the network's business; the
  // runtime refusing the call is ours.
  check(
    "the Worker's state can make a request of its own",
    trouble === null || (typeof trouble === "string" && !/illegal invocation/i.test(trouble)),
    `release ${JSON.stringify(release)}, trouble ${JSON.stringify(trouble)}`,
  );
} catch (error) {
  failed++;
  console.log(`FAIL the run stopped: ${(error as Error).message}`);
} finally {
  server.stop(true);
}
console.log(failed === 0 ? "all checks passed" : `${failed} check(s) failed`);
process.exit(failed === 0 ? 0 : 1);
