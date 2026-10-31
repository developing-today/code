import { SELF } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import { parseDnsMessage, signableBytes, z32Decode, z32Encode } from "../src/pkarr";
import { buildRelayPayload, dnsPacketWithTxt, dnsQuery, keypairFromSeed } from "./helpers";

const BASE = "https://pkarr.test";
const ORIGIN = "pkarr.developing-today.workers.dev";
const TS = 1_762_000_000_000_000n; // fixed microsecond timestamp base

function keypair(tag: number) {
  const seed = new Uint8Array(32).fill(tag);
  return keypairFromSeed(seed);
}

function z32(publicKey: Uint8Array): string {
  return z32Encode(publicKey);
}

async function put(key: string, payload: Uint8Array): Promise<Response> {
  return SELF.fetch(`${BASE}/pkarr/${key}`, {
    method: "PUT",
    headers: { "content-type": "application/x-pkarr-signed-packet" },
    body: payload,
  });
}

async function get(key: string): Promise<Response> {
  return SELF.fetch(`${BASE}/pkarr/${key}`);
}

describe("z32 codec", () => {
  it("matches hand-computed vectors", () => {
    expect(z32Encode(new Uint8Array([0x00]))).toBe("yy");
    expect(z32Encode(new Uint8Array([0xff]))).toBe("9h");
    expect(z32Decode("yy")).toEqual(new Uint8Array([0x00]));
    expect(z32Decode("9h")).toEqual(new Uint8Array([0xff]));
  });

  it("round-trips 32-byte keys to 52 chars", () => {
    const kp = keypair(7);
    const text = z32(kp.publicKey);
    expect(text).toHaveLength(52);
    expect(z32Decode(text)).toEqual(kp.publicKey);
  });

  it("rejects invalid characters and non-zero padding", () => {
    expect(z32Decode("l1v0")).toBeNull(); // l, 1, 0, v are not in the alphabet... v is; l/1/0 are not
    expect(z32Decode("yb")).toBeNull(); // non-zero leftover bits
  });
});

describe("signable bytes", () => {
  it("uses the pkarr v3 bencode framing", () => {
    const packet = new Uint8Array([1, 2, 3]);
    const out = signableBytes(42n, packet);
    expect(new TextDecoder().decode(out)).toBe("3:seqi42e1:v3:\x01\x02\x03");
  });
});

describe("pkarr HTTP relay", () => {
  it("publishes and resolves a signed packet verbatim", async () => {
    const kp = keypair(11);
    const key = z32(kp.publicKey);
    const packet = dnsPacketWithTxt(`_iroh.${key}`, "relay=https://relay.example");
    const payload = buildRelayPayload(kp, TS, packet);
    expect((await put(key, payload)).status).toBe(204);
    const res = await get(key);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("application/x-pkarr-signed-packet");
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(payload);
  });

  it("returns 404 for an unknown key", async () => {
    const kp = keypair(12);
    expect((await get(z32(kp.publicKey))).status).toBe(404);
  });

  it("rejects an invalid signature", async () => {
    const kp = keypair(13);
    const other = keypair(14);
    const key = z32(kp.publicKey);
    const packet = dnsPacketWithTxt(`_iroh.${key}`, "x");
    const payload = buildRelayPayload(other, TS, packet); // signed by the wrong key
    expect((await put(key, payload)).status).toBe(400);
  });

  it("rejects a packet that is not a DNS message", async () => {
    const kp = keypair(15);
    const key = z32(kp.publicKey);
    const payload = buildRelayPayload(kp, TS, new Uint8Array([1, 2, 3, 4]));
    expect((await put(key, payload)).status).toBe(400);
  });

  it("rejects oversize payloads at the 1072-byte bound", async () => {
    const kp = keypair(16);
    const key = z32(kp.publicKey);
    const payload = buildRelayPayload(kp, TS, new Uint8Array(1001).fill(7));
    expect(payload.length).toBe(1073);
    expect((await put(key, payload)).status).toBe(413);
  });

  it("accepts a packet at exactly 1000 bytes", async () => {
    const kp = keypair(17);
    const key = z32(kp.publicKey);
    // Packet size = 12 header + 60 name (_iroh.<52-char key>) + 10 RR
    // header + rdata (text + one length byte per 255-char chunk).
    // 914 chars of text -> 4 chunks -> 82 + 914 + 4 = exactly 1000 bytes.
    const exact = dnsPacketWithTxt(`_iroh.${key}`, "a".repeat(914));
    expect(exact.length).toBe(1000);
    const payload = buildRelayPayload(kp, TS, exact);
    expect(payload.length).toBe(1072);
    expect((await put(key, payload)).status).toBe(204);
    const res = await get(key);
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(payload);
  });

  it("never lets a stale packet overwrite a newer one, but still answers 204", async () => {
    const kp = keypair(18);
    const key = z32(kp.publicKey);
    const newer = buildRelayPayload(kp, TS + 5_000_000n, dnsPacketWithTxt(`_iroh.${key}`, "new"));
    const stale = buildRelayPayload(kp, TS, dnsPacketWithTxt(`_iroh.${key}`, "old"));
    expect((await put(key, newer)).status).toBe(204);
    expect((await put(key, stale)).status).toBe(204);
    const res = await get(key);
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(newer);
  });

  it("lets a newer packet replace an older one", async () => {
    const kp = keypair(19);
    const key = z32(kp.publicKey);
    const first = buildRelayPayload(kp, TS, dnsPacketWithTxt(`_iroh.${key}`, "first"));
    const second = buildRelayPayload(kp, TS + 1n, dnsPacketWithTxt(`_iroh.${key}`, "second"));
    expect((await put(key, first)).status).toBe(204);
    expect((await put(key, second)).status).toBe(204);
    const res = await get(key);
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(second);
  });

  it("rejects malformed keys and unknown routes/methods", async () => {
    expect((await get("notakey")).status).toBe(400);
    expect((await SELF.fetch(`${BASE}/nope`)).status).toBe(404);
    const kp = keypair(20);
    const res = await SELF.fetch(`${BASE}/pkarr/${z32(kp.publicKey)}`, { method: "DELETE" });
    expect(res.status).toBe(405);
  });
});

describe("DNS-over-HTTPS", () => {
  async function publishForDoh(tag: number, text: string) {
    const kp = keypair(tag);
    const key = z32(kp.publicKey);
    const packet = dnsPacketWithTxt(`_iroh.${key}`, text);
    expect((await put(key, buildRelayPayload(kp, TS, packet))).status).toBe(204);
    return key;
  }

  async function doh(query: Uint8Array): Promise<Response> {
    return SELF.fetch(`${BASE}/dns-query`, {
      method: "POST",
      headers: { "content-type": "application/dns-message" },
      body: query,
    });
  }

  it("answers TXT lookups for a published key", async () => {
    const key = await publishForDoh(21, "relay=https://relay.example");
    const res = await doh(dnsQuery(`_iroh.${key}.${ORIGIN}`));
    expect(res.status).toBe(200);
    const parsed = parseDnsMessage(new Uint8Array(await res.arrayBuffer()));
    if (typeof parsed === "string") throw new Error(parsed);
    expect(parsed.answers).toHaveLength(1);
    expect(parsed.answers[0].text).toBe("relay=https://relay.example");
    expect(parsed.answers[0].name).toBe(`_iroh.${key}.${ORIGIN}`);
  });

  it("answers NODATA for an unpublished key and for non-TXT types", async () => {
    const kp = keypair(22);
    const key = z32(kp.publicKey);
    const missing = await doh(dnsQuery(`_iroh.${key}.${ORIGIN}`));
    const parsedMissing = parseDnsMessage(new Uint8Array(await missing.arrayBuffer()));
    if (typeof parsedMissing === "string") throw new Error(parsedMissing);
    expect(parsedMissing.answers).toHaveLength(0);

    const published = await publishForDoh(23, "v");
    const aQuery = await doh(dnsQuery(`_iroh.${published}.${ORIGIN}`, 1)); // A record
    const parsedA = parseDnsMessage(new Uint8Array(await aQuery.arrayBuffer()));
    if (typeof parsedA === "string") throw new Error(parsedA);
    expect(parsedA.answers).toHaveLength(0);
  });

  it("refuses out-of-zone names and rejects malformed messages", async () => {
    const refused = await doh(dnsQuery("example.com"));
    const parsed = parseDnsMessage(new Uint8Array(await refused.arrayBuffer()));
    if (typeof parsed === "string") throw new Error(parsed);
    expect(parsed.flags & 0x000f).toBe(5); // REFUSED
    expect((await doh(new Uint8Array([1, 2, 3]))).status).toBe(400);
  });
});

describe("scope guardrails", () => {
  it("serves the scope text at / and ok at /healthz", async () => {
    const root = await SELF.fetch(`${BASE}/`);
    expect(root.status).toBe(200);
    expect(await root.text()).toContain("not a Mainline DHT node");
    expect((await SELF.fetch(`${BASE}/healthz`)).status).toBe(200);
  });
});
