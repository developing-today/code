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
/** What the stand-in host tells the Worker in place of main's settings,
 * as a console on some other checkout would. Every one is marked, so
 * that anything asked of GitHub or the queue store by one of them is
 * found out. */
const TOLD = {
  repo: "told-by/the-machine",
  queue: "told-by-the-machine",
  queueDbUrl: "https://told-by-the-machine.invalid",
  queueTokenName: "told.by.the.machine",
  androidPackage: "told.by.the.machine",
  promptLabel: "told/by-the-machine",
};
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
  // The stand-in machine's own copy says nothing was finished.
  done: {
    tasks: { day: 0, kinds: [], latest: [] },
    merged: { day: 0, more: false, latest: [] },
    closed: { day: 0, more: false, latest: [] },
  },
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
  // Who may sign in is the host's to tell. The repository's settings it
  // tells too, and here it tells others than main's for every one the
  // Worker goes by its own for (a console on some other checkout): the
  // Worker has to do everything below by its own all the same.
  config: async () => ({
    owners: cfg.owners,
    certNames: [],
    issuers: cfg.issuers,
    repo: TOLD.repo,
    queue: TOLD.queue,
    androidPackage: TOLD.androidPackage,
    androidFingerprints: [],
    androidApp: false,
    queueDbUrl: TOLD.queueDbUrl,
    queueTokenName: TOLD.queueTokenName,
    promptLabel: TOLD.promptLabel,
    needsRepos: cfg.needsRepos,
  }),
  run: heldRunner(deps),
  log: (line) => console.log(`  host: ${line}`),
});
deps.link = link;
const seen: string[] = [];
/** What reached the stand-ins for GitHub and the queue store, and with
 * what: the Worker's own reads, made in its runtime. */
/** How often the history behind the Trends screen was asked of GitHub. */
const historyAsked = { n: 0 };
const outside: {
  to: "github" | "queue";
  path: string;
  auth: string | null;
  agent: string | null;
  /** Whether what was asked named anything the host told in place of
   * main's settings (see TOLD). */
  told?: boolean;
  /** The tag a read of GitHub was asked by, what it was answered, and
   * the tag of what GitHub would say. */
  tag?: string | null;
  status?: number;
  etag?: string;
  /** How GitHub was asked: a read, or something written. */
  method?: string;
}[] = [];
/** The one thing in the stand-in that needs the owner's hands, and
 * whether it still carries the label that says so. */
const hands = { labelled: true };
/** GitHub turning every call away as one too many, until this second
 * (as it says it: nothing left of the allowance, and when it is back). */
const spent = { until: 0 };
/** The host out of reach while GitHub and the queue store still
 * answer, and what answers in its place. 530 is what Cloudflare says of
 * a tunnel with no connection (its error 1033): the host was never
 * reached. 502 is a gateway that could not get an answer out of it:
 * that does not say the host never had the request. */
const hostAway: { on: boolean; status: 530 | 502 } = { on: false, status: 530 };
/** The list of Workers answered at length, as GitHub answers a list of
 * open pull requests: well over a million characters, each of two
 * units (the Worker's store holds about two million bytes a value, so
 * such an answer is kept in pieces), in two runs so that the end of a
 * piece falls inside a character in one of them whichever way the
 * count before it comes out. What the Worker reads the list from comes
 * after all of it. */
const long = {
  on: false,
  padding: `${"\u{1F600}".repeat(310_000)}x${"\u{1F600}".repeat(310_000)}`,
};
/** GitHub's answer to a read: tagged, and "not modified" (with no body)
 * to a read that carries the tag of what it would say. */
const tagged = (
  req: Request,
  entry: { tag?: string | null; status?: number; etag?: string },
  body: unknown,
  status = 200,
) => {
  const etag = `W/"${Bun.hash(JSON.stringify(body)).toString(16)}"`;
  entry.tag = req.headers.get("if-none-match");
  entry.etag = etag;
  if (status === 200 && req.method === "GET" && entry.tag === etag) {
    entry.status = 304;
    return new Response(null, { status: 304, headers: { etag } });
  }
  entry.status = status;
  return Response.json(body, { status, headers: status === 200 ? { etag } : {} });
};
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 4898,
  fetch: async (req, srv) => {
    const url = new URL(req.url);
    if (url.pathname === "/jwks") return Response.json({ keys: [jwk] });
    if (url.pathname.startsWith("/github/")) {
      const path = url.pathname.slice("/github/".length);
      const entry: (typeof outside)[number] = {
        to: "github",
        told: /told[-./]by/.test(decodeURIComponent(url.pathname + url.search)),
        path,
        auth: req.headers.get("authorization"),
        agent: req.headers.get("user-agent"),
        method: req.method,
      };
      outside.push(entry);
      if (spent.until > 0) {
        entry.status = 403;
        return Response.json(
          { message: "API rate limit exceeded for user ID 1.", status: "403" },
          { status: 403, headers: { "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(spent.until) } },
        );
      }
      if (path.endsWith("/contents/pm-stack/platform/workers.json")) {
        // The platform read's list, served as GitHub serves a file.
        const content = Buffer.from(
          readFileSync(join(import.meta.dir, "..", "..", "platform", "workers.json")),
        ).toString("base64");
        return tagged(req, entry, long.on ? { padding: long.padding, content } : { content });
      }
      if (path.includes("/actions/workflows/")) return tagged(req, entry, { workflow_runs: [] });
      if (path.endsWith("/pulls")) {
        return tagged(req, entry, [
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
      if (req.method === "POST" && /\/issues\/\d+\/comments$/.test(path)) {
        // A comment is made, as GitHub answers one.
        entry.status = 201;
        return Response.json({ html_url: `https://github.com/${path.replace("repos/", "")}#made` }, { status: 201 });
      }
      if (path === "search/issues") return tagged(req, entry, { total_count: 0, items: [] });
      if (path === "repos/developing-today/code/issues" && url.searchParams.get("labels") === "user/review") {
        // What waits on the owner, as a repository's own list has it:
        // one pull request he was asked to review.
        return tagged(req, entry, [
          {
            number: 31,
            title: "Smoke review",
            body: null,
            state: "open",
            html_url: "https://github.com/developing-today/code/pull/31",
            updated_at: "2026-11-23T09:00:00Z",
            labels: [{ name: "user/review" }],
            pull_request: { url: "https://api.github.com/repos/developing-today/code/pulls/31" },
          },
        ]);
      }
      if (
        path === "repos/developing-today/code/issues" &&
        url.searchParams.get("state") === "all" &&
        url.searchParams.has("since")
      ) {
        // Everything changed at or after a time, oldest change first:
        // what the history behind the Trends screen asks. One feature
        // merged an hour ago.
        historyAsked.n++;
        const at = new Date(Date.now() - 3_600_000).toISOString().replace(/\.\d+Z$/, "Z");
        const all = [
          {
            number: 43,
            title: "feat(pm-stack): a smoke feature",
            created_at: at,
            updated_at: at,
            closed_at: at,
            state: "closed",
            labels: [],
            pull_request: { merged_at: at },
          },
        ];
        return Response.json(all.filter((item) => item.updated_at >= (url.searchParams.get("since") ?? "")));
      }
      if (path === "repos/developing-today/code/issues" && url.searchParams.get("state") === "closed") {
        // What GitHub lists as closed: a pull request merged an hour ago
        // and an issue closed two hours ago, as it sends them.
        const hoursAgo = (hours: number) =>
          new Date(Date.now() - hours * 3_600_000).toISOString().replace(/\.\d+Z$/, "Z");
        return tagged(req, entry, [
          {
            number: 41,
            title: "fix: smoke merged",
            html_url: "https://github.com/developing-today/code/pull/41",
            closed_at: hoursAgo(1),
            updated_at: hoursAgo(1),
            user: { login: "smoke-lane" },
            pull_request: { merged_at: hoursAgo(1) },
          },
          {
            number: 42,
            title: "Smoke closed",
            html_url: "https://github.com/developing-today/code/issues/42",
            closed_at: hoursAgo(2),
            updated_at: hoursAgo(2),
            state_reason: "completed",
            user: { login: "owner" },
          },
        ]);
      }
      if (path === "repos/developing-today/code/issues" && url.searchParams.get("labels") === "user/escalated") {
        // Something only the owner's hands can do, for as long as it carries the label.
        return tagged(
          req,
          entry,
          hands.labelled
            ? [
                {
                  number: 32,
                  title: "Smoke hands",
                  body: 'A person is needed.\n<!-- hands:v1 kind=physical action="Plug it in." -->',
                  state: "open",
                  html_url: "https://github.com/developing-today/code/issues/32",
                  updated_at: "2026-11-23T09:30:00Z",
                  labels: [{ name: "user/escalated" }],
                },
              ]
            : [],
        );
      }
      if (req.method === "DELETE" && path === "repos/developing-today/code/issues/32/labels/user%2Fescalated") {
        // The label taken off, as GitHub answers it: what is left on the
        // issue, or that the label was not there.
        if (!hands.labelled) {
          entry.status = 404;
          return Response.json({ message: "Label does not exist", status: "404" }, { status: 404 });
        }
        hands.labelled = false;
        entry.status = 200;
        return Response.json([]);
      }
      if (path.endsWith("/issues")) return tagged(req, entry, []);
      return tagged(req, entry, { message: "Not Found" }, 404);
    }
    if (url.pathname === "/queue/v2/pipeline") {
      const asked = (await req.json()) as { requests: { type: string }[] };
      outside.push({
        to: "queue",
        path: url.pathname,
        auth: req.headers.get("authorization"),
        agent: null,
        told: /told[-./]by/.test(JSON.stringify(asked)),
      });
      // No rows for anything asked: an empty queue.
      return Response.json({
        results: asked.requests.map((r) =>
          r.type === "execute"
            ? { type: "ok", response: { type: "execute", result: { cols: [], rows: [] } } }
            : { type: "ok", response: { type: "close" } },
        ),
      });
    }
    if (hostAway.on) {
      return new Response(hostAway.status === 530 ? "error code: 1033" : "Bad Gateway", { status: hostAway.status });
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
  // The Worker carries the repository's settings in its own bundle. The
  // host here told others for the six the Worker goes by its own for,
  // and main's list of repositories: exactly those six are said, so the
  // list in the bundle as deployed is the one this console has.
  check(
    "the Worker says in which settings the host told it others than main's, by name",
    before.machineSettingsDiffer === null &&
      JSON.stringify(after.machineSettingsDiffer) ===
        JSON.stringify(["repo", "queue", "queueDbUrl", "queueTokenName", "androidPackage", "promptLabel"]),
    JSON.stringify({ before: before.machineSettingsDiffer, after: after.machineSettingsDiffer }),
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
      "the Worker says it does the core itself, now that a machine has said who may sign in",
      after.directEnabled === true && after.primary === true && before.primary === false,
      JSON.stringify({ before: before.primary, after: after.primary, enabled: after.directEnabled }),
    );
    const kinds = (after.selfCheck as { kinds?: Record<string, { ok: boolean; error?: string }> } | null)?.kinds ?? {};
    check(
      "its own read of the board, the prompts, pull requests, ranked issues and what needs the owner each came back good",
      ["board", "asks", "prs", "priorities", "census"].every((kind) => kinds[kind]?.ok === true),
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
      "and they were of main's repository, queue and label: nothing the host named in their place was asked for",
      outside.length > 0 &&
        outside.every((o) => o.told === false) &&
        github.some((o) => o.path.startsWith(`repos/${cfg.repo}/`)),
      `${outside.filter((o) => o.told).length} of ${outside.length} named what the host told`,
    );
    check(
      "and it told the host what it reads itself, so the host stops sending it",
      JSON.stringify(second?.owns) === JSON.stringify(["board", "prs", "priorities", "asks", "census", "done"]),
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

    // The app is one list of files (public/files.json): the Worker's build
    // ships them and the Worker serves them. Each is asked for here, as
    // the signed-in owner, and has to come back whole and as what it is.
    const appFiles = JSON.parse(readFileSync(join(import.meta.dir, "..", "public", "files.json"), "utf8")) as {
      files: { path: string; type: string }[];
    };
    const wrong: string[] = [];
    for (const file of appFiles.files) {
      const res = await asOwner(`/console/${file.path}`);
      const got = new Uint8Array(await res.arrayBuffer());
      const want = readFileSync(join(import.meta.dir, "..", "public", file.path));
      if (res.status !== 200 || res.headers.get("content-type") !== file.type || got.length !== want.length) {
        wrong.push(`${file.path}: ${res.status} ${res.headers.get("content-type")} ${got.length}/${want.length}`);
      }
    }
    check(
      `every file the app is made of is served whole and as what it is (${appFiles.files.length} files, typefaces among them)`,
      appFiles.files.length > 20 && wrong.length === 0,
      wrong.join("; "),
    );

    const unlisted = await asOwner("/console/fonts/licences/inter.txt");
    check("a file the list does not name is not served", unlisted.status !== 200, String(unlisted.status));

    // Trends: the Worker reads GitHub's history and the queue's store
    // itself, keeps what it read in its own storage, and answers from
    // that. Only this runtime has that storage and that fetch.
    const trendsRes = await asOwner("/console/api/trends?days=7");
    const trends = (await trendsRes.json()) as {
      window?: number;
      days?: string[];
      history?: { whole: boolean };
      merged?: { all: number[]; kinds: { feat: number[] } };
      features?: { latest: Record<string, { number: number; title: string }[]> };
      tasks?: { all?: number[] };
    };
    const sumOf = (list: number[] | undefined) => (list ?? []).reduce((total, n) => total + n, 0);
    check(
      "Trends is read by the Worker itself: GitHub's history a page at a time, the queue's finished tasks, by day",
      trendsRes.status === 200 &&
        trends.window === 7 &&
        trends.days?.length === 14 &&
        trends.history?.whole === true &&
        sumOf(trends.merged?.all) === 1 &&
        sumOf(trends.merged?.kinds.feat) === 1 &&
        trends.features?.latest["pm-stack"]?.[0]?.number === 43 &&
        trends.features?.latest["pm-stack"]?.[0]?.title === "a smoke feature" &&
        Array.isArray(trends.tasks?.all),
      JSON.stringify(trends).slice(0, 300),
    );
    const askedOnce = historyAsked.n;
    const trendsAgain = await asOwner("/console/api/trends?days=30");
    const again30 = (await trendsAgain.json()) as { window?: number; merged?: { all: number[] } };
    check(
      "asked again at once it answers from what it kept: GitHub is not asked a second time",
      trendsAgain.status === 200 &&
        again30.window === 30 &&
        sumOf(again30.merged?.all) === 1 &&
        historyAsked.n === askedOnce,
      `asked GitHub ${historyAsked.n} time(s)`,
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
    // Read again, the list is asked for by the tag GitHub gave it, GitHub
    // says "not modified" with no body, and this runtime's fetch hands
    // that back as it is: the answer is the one kept, whole.
    type Said = {
      /** When the object answering started. */
      woke?: number;
      github?: {
        answers: number;
        unchanged: number;
        since: number;
        kept: number;
        quiet: { core: number | null; search: number | null };
      };
    };
    const healthSaid = async () => (await (await get("/console/health")).json()) as Said;
    const githubSaid = async () => (await healthSaid()).github;
    const list = () => outside.filter((call) => call.path.endsWith("/contents/pm-stack/platform/workers.json"));
    const unchangedBefore = (await githubSaid())?.unchanged ?? -1;
    const again = (await (await asOwner("/console/api/platform")).json()) as typeof platform;
    const lastAsk = list().at(-1);
    const said = await githubSaid();
    check(
      'a list read again is asked for by its tag, and GitHub\'s "not modified" is the kept answer',
      lastAsk?.status === 304 &&
        typeof lastAsk.tag === "string" &&
        lastAsk.tag.startsWith('W/"') &&
        JSON.stringify(again.workers) === JSON.stringify(platform.workers) &&
        (again.workers ?? []).length > 0 &&
        (said?.unchanged ?? 0) > unchangedBefore &&
        (said?.kept ?? 0) > 0,
      `asked by ${lastAsk?.tag}, answered ${lastAsk?.status}; health ${JSON.stringify(said)}, before ${unchangedBefore}`,
    );
    // The same list answered at length (see `long`): a full answer, since
    // it is not the one kept, and from here on the one that is.
    long.on = true;
    const atLength = (await (await asOwner("/console/api/platform")).json()) as typeof platform;
    const longAsk = list().at(-1);
    check(
      "an answer too long for one stored value is read as any other",
      longAsk?.status === 200 && JSON.stringify(atLength.workers) === JSON.stringify(platform.workers),
      `answered ${longAsk?.status}, ${(atLength.workers ?? []).length} workers`,
    );
    // GitHub turns the account away: nothing left of the hour, and none
    // to come for ten minutes.
    spent.until = Math.ceil(Date.now() / 1000) + 600;
    const quietTo = spent.until * 1000;
    const turned = await asOwner("/console/api/platform");
    await turned.text();
    spent.until = 0;
    const quietSaid = await healthSaid();
    const until = quietSaid.github?.quiet.core ?? null;
    // (Searches are counted apart by GitHub, and are not what was spent:
    // only what draws on the hour is looked at.)
    const hourly = () => outside.filter((call) => call.to === "github" && !call.path.startsWith("search/")).length;
    const askedBy = hourly();
    // The object is put away. Cloudflare takes an idle one out of memory
    // within seconds, and in service nearly every read is made by one
    // that has just started with nothing in mind: what the Worker keeps
    // of GitHub (its answers, the time it said to ask again, the count)
    // has to be in its storage, and for a day it was not. So nothing is
    // asked of the Worker here (asking is what keeps an object) until
    // health says another one answers, and the rest is asked of that.
    // It goes within a quarter of a minute of its last request, having
    // left no timer running (src/timed.ts). Looking is itself a request
    // and starts the count again, so each wait is longer than the last.
    let idled = 0;
    let woke = quietSaid.woke;
    for (const idle of [15_000, 20_000, 30_000]) {
      await new Promise((resolve) => setTimeout(resolve, idle));
      idled += idle;
      woke = (await healthSaid()).woke;
      if (woke !== quietSaid.woke) break;
    }
    check(
      "left alone, the Worker's object is put away and another answers: what follows is asked of one that was not there before",
      typeof quietSaid.woke === "number" && typeof woke === "number" && woke > quietSaid.woke,
      `${idled / 1000} s with nothing asked; the object answering started at ${quietSaid.woke}, then ${woke}`,
    );
    // GitHub would answer now, and is not asked: not for this read, and
    // not by anything else the Worker does meanwhile.
    const held = await asOwner("/console/api/platform");
    await held.text();
    const heldSaid = await githubSaid();
    check(
      "turned away with the hour used up, the Worker keeps to the time GitHub gave and asks it nothing more, put away in between though it was",
      until === quietTo && heldSaid?.quiet.core === quietTo && hourly() === askedBy,
      `quiet until ${until} then ${heldSaid?.quiet.core}, GitHub said ${quietTo}; the read answered ${turned.status}, then ${
        held.status
      }; ${hourly() - askedBy} calls while quiet`,
    );
    // The owner says to try again now, which outweighs GitHub's "not
    // before". Asked again, the list is asked for by the tag of the long
    // answer kept before all this: every piece of it is still there, the
    // store took each (a value of over a million bytes), and what the
    // Worker reads the list from, which comes after the last of them,
    // is whole.
    const retried = await actAsOwner("edge.retry", {});
    await retried.text();
    const freed = await githubSaid();
    const back = (await (await asOwner("/console/api/platform")).json()) as typeof platform;
    const backAsk = list().at(-1);
    const backSaid = await githubSaid();
    check(
      "told to try again now, it does: the wait is lifted, and the list is read",
      retried.status === 200 &&
        freed?.quiet.core === null &&
        (back.workers ?? []).some((w) => w.name === "pm-console-edge") &&
        hourly() > askedBy,
      `try again answered ${retried.status}; quiet until ${freed?.quiet.core}; ${(back.workers ?? []).length} workers; ${
        hourly() - askedBy
      } more calls`,
    );
    check(
      'by the tag of the long answer it kept before being put away, which GitHub\'s "not modified" still stands for; and its count goes on from the one before',
      backAsk?.status === 304 &&
        backAsk.tag === longAsk?.etag &&
        JSON.stringify(back.workers) === JSON.stringify(platform.workers) &&
        (backSaid?.unchanged ?? 0) > (quietSaid.github?.unchanged ?? -1) &&
        (backSaid?.answers ?? 0) > (quietSaid.github?.answers ?? -1) &&
        backSaid?.since === said?.since &&
        (backSaid?.kept ?? 0) >= (quietSaid.github?.kept ?? 1),
      `asked by ${backAsk?.tag} (the long answer's is ${longAsk?.etag}), answered ${backAsk?.status}; health ${JSON.stringify(
        backSaid,
      )}, before ${JSON.stringify(quietSaid.github)}`,
    );
    long.on = false;
    // And a wait that runs out by itself: turned away again, back in a
    // few seconds. Until the time comes nothing counted by the hour is
    // asked, and then it is.
    spent.until = Math.ceil(Date.now() / 1000) + 4;
    const shortTo = spent.until * 1000;
    await (await asOwner("/console/api/platform")).text();
    spent.until = 0;
    const shortSaid = (await githubSaid())?.quiet.core ?? null;
    const askedThen = hourly();
    await (await asOwner("/console/api/platform")).text();
    const heldThen = hourly();
    await new Promise((resolve) => setTimeout(resolve, Math.max(0, shortTo - Date.now()) + 400));
    const ranOut = (await (await asOwner("/console/api/platform")).json()) as typeof platform;
    check(
      "and a wait left to run out is kept to until the time GitHub gave, and no longer",
      shortSaid === shortTo &&
        heldThen === askedThen &&
        hourly() > askedThen &&
        JSON.stringify(ranOut.workers) === JSON.stringify(platform.workers) &&
        (await githubSaid())?.quiet.core === null,
      `quiet until ${shortSaid}, GitHub said ${shortTo}; ${heldThen - askedThen} calls while quiet, ${hourly() - askedThen} after`,
    );
    // What is held that Cloudflare can do itself, it does, with no
    // machine. The host is out of reach and GitHub turns the account
    // away, so a comment the owner makes is held. Then GitHub answers
    // again and the owner says to try again now: the Worker asks to be
    // woken, the runtime wakes it, and it makes the comment itself, once.
    {
      const commentId = "55555555-5555-4555-8555-555555555555";
      const commentsOn = (issue: number, status: number) =>
        outside.filter(
          (o) =>
            o.to === "github" &&
            o.method === "POST" &&
            o.path.endsWith(`/issues/${issue}/comments`) &&
            o.status === status,
        ).length;
      const comments = (status: number) => commentsOn(7, status);
      type Waiting = { held?: { id: string; waits?: string }[] };
      hostAway.on = true;
      hostAway.status = 530;
      spent.until = Math.ceil(Date.now() / 1000) + 600;
      const heldComment = (await (
        await actAsOwner(
          "issue.comment",
          { issue: 7, text: "held by the smoke run, then done by the Worker" },
          commentId,
        )
      ).json()) as { ok?: boolean; held?: boolean; waits?: string };
      spent.until = 0;
      check(
        "turned away by GitHub with the tunnel down, a comment is held as either side's, and nothing was written",
        heldComment.held === true && heldComment.waits === "either" && comments(403) === 1 && comments(201) === 0,
        `${JSON.stringify(heldComment)}; ${comments(403)} turned away, ${comments(201)} made`,
      );
      const tryAgain = await actAsOwner("edge.retry", {});
      await tryAgain.text();
      for (let waited = 0; waited < 20_000 && comments(201) === 0; waited += 500) {
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
      // (Time for it to be done a second time, if it were going to be.)
      await new Promise((resolve) => setTimeout(resolve, 4000));
      const waiting = (await (await asOwner("/console/api/machines")).json()) as Waiting;
      check(
        "and when GitHub answers and the owner says to try again, the Worker wakes itself and makes the comment: once, with no machine",
        tryAgain.status === 200 && comments(201) === 1 && !(waiting.held ?? []).some((h) => h.id === commentId),
        `try again ${tryAgain.status}; ${comments(201)} made; still waiting ${JSON.stringify(waiting.held)}`,
      );
      // The other way of being out of reach. What answers in the host's
      // place does not say the host never had the request (a gateway
      // that got no answer out of it): the host may have made the
      // comment, so the Worker holds it for the host and never makes it
      // itself, however able it is. This is the Worker's own reading of
      // what came back, which only this run exercises.
      const maybeId = "66666666-6666-4666-8666-666666666666";
      hostAway.status = 502;
      spent.until = Math.ceil(Date.now() / 1000) + 600;
      const heldMaybe = (await (
        await actAsOwner("issue.comment", { issue: 9, text: "held by the smoke run for the host alone" }, maybeId)
      ).json()) as { held?: boolean; waits?: string };
      spent.until = 0;
      const againMaybe = await actAsOwner("edge.retry", {});
      await againMaybe.text();
      // (As long as the other took to be made, and more.)
      await new Promise((resolve) => setTimeout(resolve, 8000));
      const waitingMaybe = (await (await asOwner("/console/api/machines")).json()) as Waiting;
      const countsMaybe = ((await (await get("/console/health")).json()) as { held?: Record<string, number> }).held;
      check(
        "out of reach with nothing to say the host never had it, a comment is held for the host alone: the Worker does not make it",
        heldMaybe.held === true &&
          heldMaybe.waits === "asked" &&
          againMaybe.status === 200 &&
          commentsOn(9, 403) === 1 &&
          commentsOn(9, 201) === 0 &&
          (waitingMaybe.held ?? []).some((h) => h.id === maybeId && h.waits === "asked") &&
          countsMaybe?.asked === 1 &&
          countsMaybe?.either === 0,
        `${JSON.stringify(heldMaybe)}; ${commentsOn(9, 403)} turned away, ${commentsOn(9, 201)} made; waiting ${JSON.stringify(waitingMaybe.held)}; health ${JSON.stringify(countsMaybe)}`,
      );
      // Taken back, so that nothing of this is left waiting for the rest of the run.
      const takenBack = await actAsOwner("held.drop", { id: maybeId });
      await takenBack.text();
      const left = (await (await asOwner("/console/api/machines")).json()) as Waiting;
      check(
        "and the owner can take it back",
        takenBack.status === 200 && (left.held ?? []).length === 0,
        `${takenBack.status}; waiting ${JSON.stringify(left.held)}`,
      );
      hostAway.on = false;
      hostAway.status = 530;
    }
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
    // With someone to tell, the object wakes itself, and health says so
    // to anyone: that the clock is wanted, and when it has next asked
    // to be woken (a time this runtime holds, not the object).
    const clockSaid = (
      (await (await get("/console/health")).json()) as { clock?: { wanted: boolean; next: number | null } }
    ).clock;
    check(
      "and health says the object's own clock is wanted, and when it next wakes",
      clockSaid?.wanted === true && typeof clockSaid.next === "number" && clockSaid.next > Date.now() - 60_000,
      JSON.stringify(clockSaid),
    );
    // The clock starts itself when a document is read. Its interval
    // is the run's shortened one, and the board goes stale half a
    // minute in: an alarm that fires and does its work reads the
    // board from GitHub again, here, with nobody asking.
    const board = await asOwner("/console/api/doc/core");
    check("the state is read signed in", board.status === 200, String(board.status));
    // What needs the owner is read by the Worker from each repository's
    // own list of each label (never the search, and with no machine):
    // the stand-in's one review is there, and the lists were asked for.
    const needs = (
      (await board.json()) as { census?: { complete?: boolean; reviews?: { ref: string; pr: boolean }[] } }
    ).census;
    const needsAsked = outside.filter(
      (o) => o.to === "github" && o.method === "GET" && /^repos\/[^/]+\/[^/]+\/issues$/.test(o.path),
    );
    check(
      "what needs the owner is read by the Worker itself from the repositories' lists",
      needs?.complete === true &&
        JSON.stringify(needs.reviews?.map((i) => [i.ref, i.pr])) ===
          JSON.stringify([["developing-today/code#31", true]]) &&
        needsAsked.length >= 8,
      `${JSON.stringify(needs)}; ${needsAsked.length} list reads`,
    );
    // What was finished lately is read by the Worker itself too: GitHub's
    // list of what is closed and the queue's own events (none, here),
    // in this runtime, with no machine asked.
    const finished = (await (await asOwner("/console/api/doc/done")).json()) as {
      done?: {
        tasks?: { day: number; latest: unknown[] };
        merged?: { day: number; latest: { number: number; by: string }[] };
        closed?: { day: number; latest: { number: number; reason: string | null }[] };
      };
      health?: { done?: { ok: boolean; error: string | null } };
    };
    check(
      "what was finished lately is read by the Worker itself: the pull request merged and the issue closed",
      finished.health?.done?.ok === true &&
        JSON.stringify(finished.done?.merged?.latest.map((m) => [m.number, m.by])) ===
          JSON.stringify([[41, "smoke-lane"]]) &&
        JSON.stringify(finished.done?.closed?.latest.map((c) => [c.number, c.reason])) ===
          JSON.stringify([[42, "completed"]]) &&
        finished.done?.merged?.day === 1 &&
        finished.done?.tasks?.day === 0,
      JSON.stringify(finished).slice(0, 400),
    );
    // The app connected to be told of changes: a real WebSocket, held by
    // this runtime's own object. Only the platform can make one, so
    // only here is it known to work. A stranger is not connected.
    const liveAt = `${EDGE.replace(/^http/, "ws")}/console/api/events`;
    const liveSaid: string[] = [];
    const stranger = await fetch(`${EDGE}/console/api/events`, {
      headers: { upgrade: "websocket" },
      redirect: "manual",
    });
    await stranger.body?.cancel();
    const liveWs = new WebSocket(liveAt, { headers: { "cf-access-jwt-assertion": await ownerToken() } } as never);
    const liveOpened = await new Promise<boolean>((resolve) => {
      liveWs.onopen = () => resolve(true);
      liveWs.onerror = () => resolve(false);
      setTimeout(() => resolve(false), 8000);
    });
    liveWs.onmessage = (event) => liveSaid.push(String(event.data));
    if (liveOpened) liveWs.send("ping");
    await new Promise((resolve) => setTimeout(resolve, 500));
    const listening = ((await (await get("/console/health")).json()) as { listening?: number }).listening;
    check(
      "the owner's app is connected to be told of changes, a stranger is not, and the platform answers its ping",
      liveOpened && stranger.status !== 101 && listening === 1 && liveSaid.includes("pong"),
      `opened ${liveOpened}, a stranger got ${stranger.status}, listening ${listening}, said ${JSON.stringify(liveSaid)}`,
    );
    liveSaid.length = 0;
    // "Done" on what needed his hands, by the Worker itself: the label
    // taken off, then the fleet told, and the list read again so the
    // item is gone from the next look.
    const handsBefore = (
      (await (await asOwner("/console/api/doc/core")).json()) as { census?: { hands?: { ref: string }[] } }
    ).census?.hands?.map((i) => i.ref);
    const wroteFrom = outside.length;
    const askedHostFrom = seen.length;
    const handsDone = (await (
      await actAsOwner("hands.done", { ref: "developing-today/code#32", note: "smoke" })
    ).json()) as {
      ok?: boolean;
      result?: { done?: boolean };
    };
    const handsWrote = outside
      .slice(wroteFrom)
      .filter((o) => o.to === "github" && o.method !== "GET")
      .map((o) => `${o.method} ${o.path} ${o.status} ${o.auth === "Bearer smoke-github"}`);
    const handsAfter = (
      (await (await asOwner("/console/api/doc/core")).json()) as { census?: { hands?: { ref: string }[] } }
    ).census?.hands?.map((i) => i.ref);
    check(
      "done on what needed the owner's hands is done by the Worker: the label off, then the fleet told, and it leaves the list",
      JSON.stringify(handsBefore) === JSON.stringify(["developing-today/code#32"]) &&
        handsDone.ok === true &&
        handsDone.result?.done === true &&
        JSON.stringify(handsWrote) ===
          JSON.stringify([
            "DELETE repos/developing-today/code/issues/32/labels/user%2Fescalated 200 true",
            "POST repos/developing-today/code/issues/32/comments 201 true",
          ]) &&
        JSON.stringify(handsAfter) === JSON.stringify([]) &&
        !seen.slice(askedHostFrom).some((s) => s.includes("/console/api/act")),
      `before ${JSON.stringify(handsBefore)}, did ${JSON.stringify(handsDone)}, wrote ${JSON.stringify(handsWrote)}, after ${JSON.stringify(handsAfter)}`,
    );
    // What he did changed what the app shows: the connected app was
    // told so, by name of the document, without asking.
    await new Promise((resolve) => setTimeout(resolve, 500));
    const toldOf = liveSaid
      .filter((text) => text.startsWith("{"))
      .map((text) => JSON.parse(text) as { t: string; docs: string[] });
    check(
      "and the connected app was told that the core document changed",
      toldOf.length >= 1 && toldOf.every((m) => m.t === "changed") && toldOf.some((m) => m.docs.includes("core")),
      JSON.stringify(liveSaid),
    );
    liveWs.close();
    await new Promise((resolve) => setTimeout(resolve, 500));
    const stillListening = ((await (await get("/console/health")).json()) as { listening?: number }).listening;
    check("and once the app goes away nobody is connected", stillListening === 0, String(stillListening));
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
