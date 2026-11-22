// A run of the edge in the real Worker runtime, on this machine.
//
//   cd edge && node build.mjs && npx wrangler dev --port 8799 \
//     --var PUBLIC_HOST:localhost --var HOST_ORIGIN:http://127.0.0.1:4898 --var VERSION:smoke
//   bun run scripts/edge-smoke.ts            (in another shell)
//
// It starts a host on port 4898 with made-up state and this repo's real
// handler and link, then checks what can be checked without a sign-in
// issuer: the Worker starts, serves its public files, takes the host's
// signed report (fetching the host's key through the "tunnel"), keeps a
// section too large for one stored value, leaves an unsigned caller to
// the host, and answers a phone that is not linked.
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
    name: "cloudflare-access",
    issuer: "https://smoke.cloudflareaccess.com",
    jwksUri: "https://smoke.cloudflareaccess.com/cdn-cgi/access/certs",
    audience: "smoke",
    source: "access",
  },
];
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
  config: async () => ({
    owners: cfg.owners,
    certNames: [],
    issuers: cfg.issuers,
    repo: cfg.repo,
    queue: cfg.queue,
    androidPackage: cfg.androidPackage,
    androidFingerprints: [],
    androidApp: false,
  }),
  run: heldRunner(deps),
  log: (line) => console.log(`  host: ${line}`),
});
deps.link = link;
const seen: string[] = [];
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 4898,
  fetch: async (req, srv) => {
    seen.push(`${req.method} ${new URL(req.url).pathname}`);
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
} catch (error) {
  failed++;
  console.log(`FAIL the run stopped: ${(error as Error).message}`);
} finally {
  server.stop(true);
}
console.log(failed === 0 ? "all checks passed" : `${failed} check(s) failed`);
process.exit(failed === 0 ? 0 : 1);
