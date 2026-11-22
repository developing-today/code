// The app as it is served from the edge, on this machine, for a browser.
//
//   bun run scripts/edge-demo.ts [port]     then open http://127.0.0.1:<port>/
//
// The Worker's real routing and state answer the browser, with the
// made-up host of demo.ts behind them and the host's real link reporting
// to the edge. Stand-ins: the platform's storage (a map), the sign-in (a
// key made here; the first visit sets its token as a cookie), and push
// services (accepted and dropped).
//
//   /__demo/host/down   the host stops answering and reporting
//   /__demo/host/up     and comes back
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Edge } from "../edge/src/edge.ts";
import { type Io, route } from "../edge/src/entry.ts";
import { heldRunner, handle, marked } from "../src/server.ts";
import { memoryStore } from "../src/store.ts";
import { EdgeLink } from "../src/sync.ts";
import { demoDeps, demoDocs } from "./demo-state.ts";

const extra = process.argv.slice(2);
if (extra.length > 1 || (extra[0] !== undefined && !/^[1-9][0-9]{1,4}$/.test(extra[0]))) {
  console.error("usage: bun run scripts/edge-demo.ts [port]");
  process.exit(2);
}
const port = Number(extra[0] ?? 4897);
const HOSTNAME = "127.0.0.1";
const OWNER = "you@example.com";
const ISSUER = {
  name: "demo-sign-in",
  issuer: "https://demo.invalid",
  jwksUri: "https://demo.invalid/keys",
  audience: "demo",
  source: "access" as const,
};

// A sign-in issuer made here.
const pair = (await crypto.subtle.generateKey(
  { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
  true,
  ["sign", "verify"],
)) as CryptoKeyPair;
const jwk = { ...(await crypto.subtle.exportKey("jwk", pair.publicKey)), kid: "demo" };
const part = (value: unknown) => Buffer.from(JSON.stringify(value)).toString("base64url");
async function token(): Promise<string> {
  const now = Math.floor(Date.now() / 1000);
  const signed = `${part({ alg: "RS256", kid: "demo" })}.${part({ iss: ISSUER.issuer, aud: [ISSUER.audience], exp: now + 86_400, iat: now, email: OWNER })}`;
  const sig = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", pair.privateKey, new TextEncoder().encode(signed));
  return `${signed}.${Buffer.from(sig).toString("base64url")}`;
}

const stored = new Map<string, unknown>();
const edge = new Edge({
  kv: {
    get: async (key) => structuredClone(stored.get(key)),
    put: async (key, value) => {
      stored.set(key, structuredClone(value));
    },
    delete: async (key) => {
      stored.delete(key);
    },
  },
  fetchImpl: (async (url: string) =>
    String(url) === ISSUER.jwksUri ? Response.json({ keys: [jwk] }) : new Response(null, { status: 201 })) as never,
});

const net = { hostUp: true };
const publicDir = join(import.meta.dir, "..", "public");
const io: Io = {
  state: (r) => edge.fetch(r),
  asset: async (path) => {
    try {
      const name = path.startsWith("/console/") ? path.slice("/console/".length) : "";
      return name ? new Response(readFileSync(join(publicDir, name))) : new Response("", { status: 404 });
    } catch {
      return new Response("", { status: 404 });
    }
  },
  // The made-up host trusts every caller, as demo.ts does; here it is
  // only ever reached through the edge.
  host: async (r) => (net.hostUp ? marked(await handle(r, demoDeps)) : null),
  publicHost: HOSTNAME,
  version: "demo",
  later: (work) => void work.catch(() => {}),
  now: Date.now,
};

const link = new EdgeLink({
  url: `http://${HOSTNAME}:${port}`,
  keys: memoryStore(),
  done: memoryStore(),
  docs: demoDocs,
  phones: demoDeps.phones!.devices,
  config: async () => ({
    owners: [OWNER],
    certNames: [],
    issuers: [ISSUER],
    repo: demoDeps.cfg.repo,
    queue: demoDeps.cfg.queue,
    androidPackage: demoDeps.cfg.androidPackage,
    androidFingerprints: [],
    androidApp: false,
    queueDbUrl: demoDeps.cfg.queueDbUrl,
    queueTokenName: demoDeps.cfg.queueTokenName,
    promptLabel: demoDeps.cfg.promptLabel,
    needsRepos: demoDeps.cfg.needsRepos,
  }),
  run: heldRunner(demoDeps),
  fetchImpl: (async (url: string, init: RequestInit) => {
    if (!net.hostUp) throw new Error("the host is down");
    return route(new Request(url, init), io);
  }) as never,
  log: (line) => console.log(`host: ${line}`),
});
demoDeps.link = link;
await demoDocs.touch([
  "board",
  "sessions",
  "census",
  "asks",
  "questionCount",
  "questions",
  "prs",
  "priorities",
  "agents",
  "services",
  "more",
]);
await link.once();
setInterval(() => void link.turn().catch(() => {}), 2_000);

Bun.serve({
  hostname: HOSTNAME,
  port,
  fetch: async (req) => {
    const url = new URL(req.url);
    if (url.pathname === "/__demo/host/down" || url.pathname === "/__demo/host/up") {
      net.hostUp = url.pathname.endsWith("/up");
      return new Response(`host ${net.hostUp ? "up" : "down"}\n`);
    }
    // The first visit signs the browser in.
    if (
      !(req.headers.get("cookie") ?? "").includes("CF_Authorization=") &&
      req.headers.get("sec-fetch-dest") === "document"
    ) {
      return new Response(null, {
        status: 302,
        headers: {
          location: url.pathname + url.search,
          "set-cookie": `CF_Authorization=${await token()}; Path=/; HttpOnly; SameSite=Lax`,
        },
      });
    }
    return route(req, io);
  },
});
console.log(`edge demo: http://${HOSTNAME}:${port}/`);
