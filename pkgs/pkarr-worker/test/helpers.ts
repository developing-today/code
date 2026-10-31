// Test helpers: build pkarr relay payloads exactly per the spec
// (pubky/pkarr design/relays.md; iroh-dns 1.3.0) using Node's Ed25519, so
// the tests anchor the Worker's WebCrypto verification to an independent
// implementation.
import { createPrivateKey, createPublicKey, sign, type KeyObject } from "node:crypto";

const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");

export interface TestKeypair {
  privateKey: KeyObject;
  publicKey: Uint8Array;
}

export function keypairFromSeed(seed: Uint8Array): TestKeypair {
  const privateKey = createPrivateKey({
    key: Buffer.concat([PKCS8_PREFIX, Buffer.from(seed)]),
    format: "der",
    type: "pkcs8",
  });
  const spki = createPublicKey(privateKey).export({ format: "der", type: "spki" });
  return { privateKey, publicKey: new Uint8Array(spki.subarray(spki.length - 32)) };
}

/** Assemble a relay payload: signature || timestamp_be64 || packet. */
export function buildRelayPayload(
  keypair: TestKeypair,
  timestampMicros: bigint,
  packet: Uint8Array,
): Uint8Array {
  const head = Buffer.from(`3:seqi${timestampMicros}e1:v${packet.length}:`, "ascii");
  const signature = sign(null, Buffer.concat([head, Buffer.from(packet)]), keypair.privateKey);
  const timestamp = Buffer.alloc(8);
  timestamp.writeBigUInt64BE(timestampMicros);
  return new Uint8Array(Buffer.concat([signature, timestamp, Buffer.from(packet)]));
}

function encodeName(name: string): Buffer {
  const parts: Buffer[] = [];
  for (const label of name.replace(/\.$/, "").split(".")) {
    parts.push(Buffer.from([label.length]), Buffer.from(label, "ascii"));
  }
  parts.push(Buffer.from([0]));
  return Buffer.concat(parts);
}

function encodeTxtRdata(text: string): Buffer {
  const bytes = Buffer.from(text, "utf8");
  const parts: Buffer[] = [];
  for (let p = 0; p < bytes.length; p += 255) {
    const chunk = bytes.subarray(p, p + 255);
    parts.push(Buffer.from([chunk.length]), chunk);
  }
  if (parts.length === 0) parts.push(Buffer.from([0]));
  return Buffer.concat(parts);
}

/** A minimal DNS message (response shape) carrying one TXT answer. */
export function dnsPacketWithTxt(name: string, text: string, ttl = 30): Uint8Array {
  const header = Buffer.alloc(12);
  header.writeUInt16BE(0x1234, 0);
  header.writeUInt16BE(0x8400, 2); // QR + AA
  header.writeUInt16BE(0, 4);
  header.writeUInt16BE(1, 6);
  const rr = Buffer.alloc(10);
  rr.writeUInt16BE(16, 0); // TXT
  rr.writeUInt16BE(1, 2); // IN
  rr.writeUInt32BE(ttl, 4);
  const rdata = encodeTxtRdata(text);
  rr.writeUInt16BE(rdata.length, 8);
  return new Uint8Array(Buffer.concat([header, encodeName(name), rr, rdata]));
}

/** A DoH query for `name` (TXT/IN unless overridden). */
export function dnsQuery(name: string, type = 16, id = 0x2222): Uint8Array {
  const header = Buffer.alloc(12);
  header.writeUInt16BE(id, 0);
  header.writeUInt16BE(0x0100, 2); // RD
  header.writeUInt16BE(1, 4);
  const tail = Buffer.alloc(4);
  tail.writeUInt16BE(type, 0);
  tail.writeUInt16BE(1, 2);
  return new Uint8Array(Buffer.concat([header, encodeName(name), tail]));
}
