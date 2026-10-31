// pkarr relay protocol primitives: z-base-32 key encoding, the signed
// packet wire format, and DNS wire parsing/building for the DoH endpoint.
//
// Wire formats (pkarr relay spec, pubky/pkarr design/relays.md; iroh-dns
// 1.3.0 src/pkarr.rs — see doc/2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md):
//   relay payload = signature(64) || timestamp_micros_be(8) || dns_packet(<=1000)
//   signable      = "3:seq" "i" <timestamp_micros> "e" "1:v" <len> ":" <dns_packet>
//   public key    = 32 bytes, addressed as z-base-32 (52 chars)

/** DNS packet hard bound from the pkarr spec. */
export const MAX_DNS_PACKET = 1000;
/** signature(64) + timestamp(8) + packet(1000). */
export const MAX_RELAY_PAYLOAD = 64 + 8 + MAX_DNS_PACKET;

const Z32_ALPHABET = "ybndrfg8ejkmcpqxot1uwisza345h769";
const Z32_LOOKUP: Record<string, number> = Object.fromEntries([...Z32_ALPHABET].map((c, i) => [c, i]));

/** Decode a z-base-32 string. Returns null on any invalid character/length. */
export function z32Decode(text: string): Uint8Array | null {
  if (text.length === 0) return null;
  let buffer = 0;
  let bits = 0;
  const out: number[] = [];
  for (const ch of text.toLowerCase()) {
    const value = Z32_LOOKUP[ch];
    if (value === undefined) return null;
    buffer = (buffer << 5) | value;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      out.push((buffer >>> bits) & 0xff);
      buffer &= (1 << bits) - 1; // keep only the still-pending bits
    }
  }
  // Leftover bits (buffer holds exactly `bits` of them) must be zero padding.
  if (bits >= 5 || buffer !== 0) return null;
  return new Uint8Array(out);
}

/** Encode bytes as z-base-32 (no padding). */
export function z32Encode(bytes: Uint8Array): string {
  let buffer = 0;
  let bits = 0;
  let out = "";
  for (const byte of bytes) {
    buffer = (buffer << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out += Z32_ALPHABET[(buffer >>> bits) & 0x1f];
    }
    buffer &= (1 << bits) - 1; // keep only the still-pending bits
  }
  if (bits > 0) out += Z32_ALPHABET[(buffer << (5 - bits)) & 0x1f];
  return out;
}

/** A public key is exactly 32 bytes encoded as exactly 52 z-base-32 chars. */
export function parsePublicKey(text: string): Uint8Array | null {
  if (text.length !== 52) return null;
  const bytes = z32Decode(text);
  return bytes !== null && bytes.length === 32 ? bytes : null;
}

export interface SignedPacket {
  signature: Uint8Array;
  /** Microseconds since the Unix epoch (the pkarr sequence number). */
  timestampMicros: bigint;
  /** The signed DNS packet, served back verbatim. */
  packet: Uint8Array;
}

/** Split a relay payload into its parts. Returns an error string or the packet. */
export function parseRelayPayload(body: Uint8Array): SignedPacket | string {
  if (body.length > MAX_RELAY_PAYLOAD) return "payload too large";
  if (body.length < 64 + 8 + 1) return "payload too short";
  const signature = body.slice(0, 64);
  let timestamp = 0n;
  for (let i = 0; i < 8; i++) timestamp = (timestamp << 8n) | BigInt(body[64 + i]);
  const packet = body.slice(72);
  if (packet.length > MAX_DNS_PACKET) return "dns packet too large";
  return { signature, timestampMicros: timestamp, packet };
}

/** The exact bytes the endpoint key signs (BEP-44-style bencode of seq + v). */
export function signableBytes(timestampMicros: bigint, packet: Uint8Array): Uint8Array {
  const head = new TextEncoder().encode(`3:seqi${timestampMicros}e1:v${packet.length}:`);
  const out = new Uint8Array(head.length + packet.length);
  out.set(head, 0);
  out.set(packet, head.length);
  return out;
}

const ED25519_SPKI_PREFIX = new Uint8Array([0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00]);

/** Verify a packet's Ed25519 signature against the path public key. */
export async function verifyPacket(publicKey: Uint8Array, pkt: SignedPacket): Promise<boolean> {
  try {
    const spki = new Uint8Array(ED25519_SPKI_PREFIX.length + publicKey.length);
    spki.set(ED25519_SPKI_PREFIX, 0);
    spki.set(publicKey, ED25519_SPKI_PREFIX.length);
    const key = await crypto.subtle.importKey("spki", spki, { name: "Ed25519" }, false, ["verify"]);
    return await crypto.subtle.verify(
      { name: "Ed25519" },
      key,
      pkt.signature,
      signableBytes(pkt.timestampMicros, pkt.packet),
    );
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------
// DNS wire format (RFC 1035 + RFC 8484 DoH). Only what the design needs:
// strict bounded parsing of stored packets and queries, TXT extraction, and
// response construction. No recursion, no forwarding.
// ---------------------------------------------------------------------------

export interface DnsQuestion {
  name: string;
  type: number;
  klass: number;
  /** Offset just past this question in the original message. */
  endOffset: number;
}

export interface DnsRecord {
  name: string;
  type: number;
  klass: number;
  ttl: number;
  /** For TXT records: the concatenated character-strings, UTF-8 decoded. */
  text?: string;
  rdata: Uint8Array;
}

export interface DnsMessage {
  id: number;
  flags: number;
  questions: DnsQuestion[];
  answers: DnsRecord[];
  authorities: DnsRecord[];
  additionals: DnsRecord[];
}

const TYPE_TXT = 16;

function readName(msg: Uint8Array, start: number): { name: string; next: number } | string {
  const labels: string[] = [];
  let offset = start;
  let next = -1;
  let jumps = 0;
  for (;;) {
    if (offset >= msg.length) return "name out of bounds";
    const len = msg[offset];
    if (len === 0) {
      if (next === -1) next = offset + 1;
      break;
    }
    if ((len & 0xc0) === 0xc0) {
      if (offset + 1 >= msg.length) return "pointer out of bounds";
      const target = ((len & 0x3f) << 8) | msg[offset + 1];
      if (target >= offset) return "pointer does not point backwards";
      if (next === -1) next = offset + 2;
      offset = target;
      if (++jumps > 16) return "too many pointer jumps";
      continue;
    }
    if ((len & 0xc0) !== 0) return "reserved label bits set";
    if (offset + 1 + len > msg.length) return "label out of bounds";
    let label = "";
    for (let i = 0; i < len; i++) label += String.fromCharCode(msg[offset + 1 + i]);
    labels.push(label.toLowerCase());
    offset += 1 + len;
  }
  return { name: labels.join("."), next };
}

function parseRecords(msg: Uint8Array, start: number, count: number): { records: DnsRecord[]; next: number } | string {
  const records: DnsRecord[] = [];
  let offset = start;
  for (let i = 0; i < count; i++) {
    const name = readName(msg, offset);
    if (typeof name === "string") return name;
    offset = name.next;
    if (offset + 10 > msg.length) return "record header out of bounds";
    const view = new DataView(msg.buffer, msg.byteOffset, msg.byteLength);
    const type = view.getUint16(offset);
    const klass = view.getUint16(offset + 2);
    const ttl = view.getUint32(offset + 4);
    const rdlength = view.getUint16(offset + 8);
    offset += 10;
    if (offset + rdlength > msg.length) return "rdata out of bounds";
    const rdata = msg.slice(offset, offset + rdlength);
    const record: DnsRecord = { name: name.name, type, klass, ttl, rdata };
    if (type === TYPE_TXT) {
      let text = "";
      let p = 0;
      while (p < rdata.length) {
        const len = rdata[p];
        if (p + 1 + len > rdata.length) return "txt string out of bounds";
        text += new TextDecoder().decode(rdata.slice(p + 1, p + 1 + len));
        p += 1 + len;
      }
      record.text = text;
    }
    records.push(record);
    offset += rdlength;
  }
  return { records, next: offset };
}

/** Strictly parse a DNS message; any bounds violation is an error string. */
export function parseDnsMessage(msg: Uint8Array): DnsMessage | string {
  if (msg.length < 12) return "message shorter than dns header";
  if (msg.length > 65535) return "message too large";
  const view = new DataView(msg.buffer, msg.byteOffset, msg.byteLength);
  const id = view.getUint16(0);
  const flags = view.getUint16(2);
  const qd = view.getUint16(4);
  const an = view.getUint16(6);
  const ns = view.getUint16(8);
  const ar = view.getUint16(10);
  let offset = 12;
  const questions: DnsQuestion[] = [];
  for (let i = 0; i < qd; i++) {
    const name = readName(msg, offset);
    if (typeof name === "string") return name;
    offset = name.next;
    if (offset + 4 > msg.length) return "question out of bounds";
    questions.push({
      name: name.name,
      type: view.getUint16(offset),
      klass: view.getUint16(offset + 2),
      endOffset: offset + 4,
    });
    offset += 4;
  }
  const answers = parseRecords(msg, offset, an);
  if (typeof answers === "string") return answers;
  offset = answers.next;
  const authorities = parseRecords(msg, offset, ns);
  if (typeof authorities === "string") return authorities;
  offset = authorities.next;
  const additionals = parseRecords(msg, offset, ar);
  if (typeof additionals === "string") return additionals;
  return {
    id,
    flags,
    questions,
    answers: answers.records,
    authorities: authorities.records,
    additionals: additionals.records,
  };
}

/** All TXT records in a parsed packet, from every section. */
export function txtRecords(msg: DnsMessage): DnsRecord[] {
  return [...msg.answers, ...msg.authorities, ...msg.additionals].filter((r) => r.type === TYPE_TXT);
}

function encodeTxtRdata(text: string): Uint8Array {
  const bytes = new TextEncoder().encode(text);
  const chunks: Uint8Array[] = [];
  let p = 0;
  while (p < bytes.length) {
    let end = Math.min(p + 255, bytes.length);
    // Do not split a UTF-8 sequence: back off to a leading byte.
    while (end > p && end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
    const chunk = new Uint8Array(1 + (end - p));
    chunk[0] = end - p;
    chunk.set(bytes.slice(p, end), 1);
    chunks.push(chunk);
    p = end;
  }
  if (chunks.length === 0) chunks.push(new Uint8Array([0]));
  const total = chunks.reduce((n, c) => n + c.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

export interface DohAnswer {
  ttl: number;
  text: string;
}

/**
 * Build a DoH response for a single-question query. Answer names use a
 * compression pointer to the question name at offset 12, the standard
 * shape for name == qname answers.
 */
export function buildDohResponse(
  query: Uint8Array,
  question: DnsQuestion,
  rcode: number,
  answers: DohAnswer[],
): Uint8Array {
  const questionBytes = query.slice(12, question.endOffset);
  const answerBytes = answers.map((answer) => {
    const rdata = encodeTxtRdata(answer.text);
    const out = new Uint8Array(12 + rdata.length);
    const view = new DataView(out.buffer);
    view.setUint16(0, 0xc00c); // pointer to the question name
    view.setUint16(2, TYPE_TXT);
    view.setUint16(4, question.klass === 255 ? 1 : question.klass);
    view.setUint32(6, answer.ttl >>> 0);
    view.setUint16(10, rdata.length);
    out.set(rdata, 12);
    return out;
  });
  const total = 12 + questionBytes.length + answerBytes.reduce((n, b) => n + b.length, 0);
  const out = new Uint8Array(total);
  const view = new DataView(out.buffer);
  const queryView = new DataView(query.buffer, query.byteOffset, query.byteLength);
  view.setUint16(0, queryView.getUint16(0)); // id
  const queryFlags = queryView.getUint16(2);
  let flags = 0x8000 | (queryFlags & 0x7900) | (rcode & 0x000f); // QR + opcode + RD + rcode
  view.setUint16(2, flags);
  view.setUint16(4, 1);
  view.setUint16(6, answers.length);
  view.setUint16(8, 0);
  view.setUint16(10, 0);
  out.set(questionBytes, 12);
  let offset = 12 + questionBytes.length;
  for (const answer of answerBytes) {
    out.set(answer, offset);
    offset += answer.length;
  }
  return out;
}
