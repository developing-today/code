// pkarr Worker: our own door into pkarr for the id system.
//
// Serves signed pkarr packets over plain HTTP and answers the matching
// DNS-over-HTTPS TXT lookups, per doc/2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md:
//   PUT /pkarr/<z32>   publish a signed packet (verified, newest-wins)
//   GET /pkarr/<z32>   fetch the current signed packet verbatim
//   POST /dns-query    DoH for _iroh.<z32>.<ORIGIN> TXT lookups
//   GET  /dns-query?dns=<base64url>   same, RFC 8484 GET form
//
// It is a lookup service only: not a DHT node, not a DNS server, not a
// relay for traffic. One SQLite-backed Durable Object per public key is
// the strongly consistent store; records expire after RETENTION_SECONDS
// without refresh. Clients verify signatures themselves — the relay also
// verifies before storing, but is trusted for availability, not integrity.

import { DurableObject } from "cloudflare:workers";
import {
  MAX_RELAY_PAYLOAD,
  buildDohResponse,
  parseDnsMessage,
  parsePublicKey,
  parseRelayPayload,
  txtRecords,
  verifyPacket,
  z32Encode,
} from "./pkarr";

export interface Env {
  PKARR_STORE: DurableObjectNamespace<PkarrStore>;
  PUT_LIMITER: RateLimit;
  GET_LIMITER: RateLimit;
  DOH_LIMITER: RateLimit;
  /** DNS origin the DoH endpoint answers under, e.g. "pkarr.example.com". */
  ORIGIN: string;
  /** Seconds a record survives without a refresh. Default 7 days. */
  RETENTION_SECONDS: string;
  /** TTL (seconds) stamped on DoH TXT answers. */
  DNS_TTL: string;
}

const PACKET_CONTENT_TYPE = "application/x-pkarr-signed-packet";
const DOH_CONTENT_TYPE = "application/dns-message";
const MAX_DOH_BODY = 4096;

export class PkarrStore extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    ctx.blockConcurrencyWhile(async () => {
      this.ctx.storage.sql.exec(
        "CREATE TABLE IF NOT EXISTS record (id INTEGER PRIMARY KEY CHECK (id = 1), payload BLOB NOT NULL, ts INTEGER NOT NULL)",
      );
    });
  }

  /** Store the payload only if strictly newer than the current record. */
  async putPacket(payload: Uint8Array, timestampMicros: bigint): Promise<"stored" | "stale"> {
    const rows = [...this.ctx.storage.sql.exec("SELECT ts FROM record WHERE id = 1")];
    const current = rows.length > 0 ? BigInt(rows[0].ts as number) : null;
    // ts is stored as a SQLite INTEGER (signed 64-bit); microsecond
    // timestamps (~1.7e15) fit comfortably.
    if (current !== null && timestampMicros <= current) return "stale";
    this.ctx.storage.sql.exec(
      "INSERT INTO record (id, payload, ts) VALUES (1, ?, ?) ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, ts = excluded.ts",
      payload,
      Number(timestampMicros),
    );
    const retentionMs = Number(this.env.RETENTION_SECONDS || "604800") * 1000;
    await this.ctx.storage.setAlarm(Date.now() + retentionMs);
    return "stored";
  }

  async getPacket(): Promise<Uint8Array | null> {
    const rows = [...this.ctx.storage.sql.exec("SELECT payload FROM record WHERE id = 1")];
    if (rows.length === 0) return null;
    // SQLite stores the payload as a BLOB; workerd hands BLOB columns back
    // as ArrayBuffer, so normalize to a byte view here.
    const raw = rows[0].payload as unknown;
    if (raw instanceof Uint8Array) return raw;
    if (raw instanceof ArrayBuffer) return new Uint8Array(raw);
    if (ArrayBuffer.isView(raw)) {
      return new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength);
    }
    return null;
  }

  async alarm(): Promise<void> {
    this.ctx.storage.sql.exec("DELETE FROM record WHERE id = 1");
  }
}

function storeFor(env: Env, publicKey: Uint8Array): DurableObjectStub<PkarrStore> {
  // One object per key, named by its canonical z32 form.
  return env.PKARR_STORE.get(env.PKARR_STORE.idFromName(z32Encode(publicKey)));
}

function clientIp(request: Request): string {
  return request.headers.get("cf-connecting-ip") ?? "unknown";
}

async function rateLimited(limiter: RateLimit, key: string): Promise<boolean> {
  try {
    const { success } = await limiter.limit({ key });
    return !success;
  } catch {
    // A limiter fault must not take the service down with it.
    return false;
  }
}

const NO_STORE = { "cache-control": "no-store" } as const;

async function handlePut(request: Request, env: Env, keyText: string): Promise<Response> {
  const publicKey = parsePublicKey(keyText);
  if (publicKey === null) return new Response("invalid public key", { status: 400 });
  if (await rateLimited(env.PUT_LIMITER, clientIp(request))) {
    return new Response("rate limited", { status: 429 });
  }
  const body = new Uint8Array(await request.arrayBuffer());
  if (body.length > MAX_RELAY_PAYLOAD) return new Response("payload too large", { status: 413 });
  const parsed = parseRelayPayload(body);
  if (typeof parsed === "string") return new Response(parsed, { status: 400 });
  // The packet itself must be a well-formed DNS message within bounds.
  if (typeof parseDnsMessage(parsed.packet) === "string") {
    return new Response("malformed dns packet", { status: 400 });
  }
  if (!(await verifyPacket(publicKey, parsed))) {
    return new Response("invalid signature", { status: 400 });
  }
  try {
    await storeFor(env, publicKey).putPacket(body, parsed.timestampMicros);
  } catch {
    return new Response("storage failure", { status: 503 });
  }
  // 204 for both newly stored and valid-but-stale packets (iroh clients
  // accept only 2xx; the reference server behaves the same way).
  return new Response(null, { status: 204 });
}

async function handleGet(env: Env, keyText: string, request: Request): Promise<Response> {
  const publicKey = parsePublicKey(keyText);
  if (publicKey === null) return new Response("invalid public key", { status: 400 });
  if (await rateLimited(env.GET_LIMITER, clientIp(request))) {
    return new Response("rate limited", { status: 429 });
  }
  const payload = await storeFor(env, publicKey).getPacket();
  if (payload === null) return new Response("not found", { status: 404, headers: NO_STORE });
  return new Response(payload, {
    status: 200,
    headers: { "content-type": PACKET_CONTENT_TYPE, ...NO_STORE },
  });
}

function originLabels(origin: string): string[] {
  return origin.toLowerCase().replace(/\.$/, "").split(".").filter(Boolean);
}

/** Extract the z32 key from `_iroh.<z32>.<origin>`; null when not that shape. */
function keyFromQueryName(name: string, origin: string): string | null {
  const labels = name.toLowerCase().replace(/\.$/, "").split(".");
  const suffix = originLabels(origin);
  if (labels.length !== suffix.length + 2) return null;
  if (labels[0] !== "_iroh") return null;
  for (let i = 0; i < suffix.length; i++) {
    if (labels[2 + i] !== suffix[i]) return null;
  }
  return labels[1];
}

function underOrigin(name: string, origin: string): boolean {
  const labels = name.toLowerCase().replace(/\.$/, "").split(".");
  const suffix = originLabels(origin);
  if (labels.length < suffix.length) return false;
  return suffix.every((label, i) => labels[labels.length - suffix.length + i] === label);
}

async function handleDoh(request: Request, env: Env, queryBytes: Uint8Array): Promise<Response> {
  if (queryBytes.length > MAX_DOH_BODY) return new Response("message too large", { status: 413 });
  const query = parseDnsMessage(queryBytes);
  if (typeof query === "string") return new Response("malformed dns message", { status: 400 });
  if (query.questions.length !== 1) {
    return new Response("exactly one question is supported", { status: 400 });
  }
  const question = query.questions[0];
  const respond = (rcode: number, answers: { ttl: number; text: string }[]) =>
    new Response(buildDohResponse(queryBytes, question, rcode, answers), {
      status: 200,
      headers: { "content-type": DOH_CONTENT_TYPE, ...NO_STORE },
    });
  // Out-of-zone names are refused; we are not a recursive resolver.
  if (!underOrigin(question.name, env.ORIGIN)) return respond(5, []);
  const keyText = keyFromQueryName(question.name, env.ORIGIN);
  if (keyText === null) return respond(0, []); // in-zone but not a pkarr name: NODATA
  if (question.type !== 16 && question.type !== 255) return respond(0, []); // TXT/ANY only
  const publicKey = parsePublicKey(keyText);
  if (publicKey === null) return respond(0, []);
  if (await rateLimited(env.DOH_LIMITER, clientIp(request))) {
    return new Response("rate limited", { status: 429 });
  }
  const payload = await storeFor(env, publicKey).getPacket();
  if (payload === null) return respond(0, []);
  const parsed = parseRelayPayload(payload);
  if (typeof parsed === "string") return respond(0, []);
  const packet = parseDnsMessage(parsed.packet);
  if (typeof packet === "string") return respond(0, []);
  const ttl = Number.parseInt(env.DNS_TTL ?? "", 10);
  const answers = txtRecords(packet)
    .filter((record) => {
      const name = record.name.toLowerCase();
      // Stored names vary by publisher encoding: accept the record under
      // any of its canonical names for this key.
      return (
        name === `_iroh.${keyText}` ||
        name === "_iroh" ||
        name === keyText ||
        name === `_iroh.${keyText}.${env.ORIGIN.toLowerCase()}`
      );
    })
    .map((record) => ({ ttl: Number.isFinite(ttl) && ttl > 0 ? ttl : 30, text: record.text ?? "" }));
  return respond(0, answers);
}

const SCOPE_TEXT = `pkarr lookup service (P7.2)

This Worker is a self-hosted pkarr relay + DNS-over-HTTPS endpoint for the
id system. It stores signed pkarr packets and serves them back over HTTP:

  PUT /pkarr/<z32-public-key>   publish a signed packet
  GET /pkarr/<z32-public-key>   fetch the current packet
  POST /dns-query               DoH TXT lookups for _iroh.<key>.<origin>

It is lookup only: not a Mainline DHT node, not a general DNS server, not
a traffic relay, and not a replacement for the default n0 pkarr service.
Records are signed by the endpoint key and expire after a retention
window without refresh.
`;

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const path = url.pathname;
    if (path === "/") {
      if (request.method !== "GET") return new Response("method not allowed", { status: 405 });
      return new Response(SCOPE_TEXT, { status: 200, headers: { "content-type": "text/plain" } });
    }
    if (path === "/healthz") {
      if (request.method !== "GET") return new Response("method not allowed", { status: 405 });
      return new Response("ok", { status: 200 });
    }
    if (path.startsWith("/pkarr/")) {
      const keyText = path.slice("/pkarr/".length);
      if (keyText.includes("/")) return new Response("not found", { status: 404 });
      if (request.method === "PUT") return handlePut(request, env, keyText);
      if (request.method === "GET") return handleGet(env, keyText, request);
      return new Response("method not allowed", { status: 405 });
    }
    if (path === "/dns-query") {
      if (request.method === "POST") {
        return handleDoh(request, env, new Uint8Array(await request.arrayBuffer()));
      }
      if (request.method === "GET") {
        const dns = url.searchParams.get("dns");
        if (dns === null) return new Response("missing dns parameter", { status: 400 });
        try {
          const base64 = dns.replace(/-/g, "+").replace(/_/g, "/");
          const padded = base64 + "=".repeat((4 - (base64.length % 4)) % 4);
          const binary = atob(padded);
          const bytes = new Uint8Array(binary.length);
          for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
          return handleDoh(request, env, bytes);
        } catch {
          return new Response("invalid dns parameter", { status: 400 });
        }
      }
      return new Response("method not allowed", { status: 405 });
    }
    return new Response("not found", { status: 404 });
  },
};
