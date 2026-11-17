// Currency PoC (#2298) — signing + canonicalisation helpers.
//
// Ed25519 throughout, matching the §6 signed-export format in
// pkgs/id/src/hosted_export.rs (signature = the producing key's
// Ed25519 signature over the canonical bytes of the record,
// domain-separated per record type). Implemented on WebCrypto
// (crypto.subtle) so the same code runs in workerd and in Node's
// test runner, with no node:crypto dependency. Public keys cross
// the wire as the raw 32-byte key in hex.
// Simplification (listed in the results doc): the permanent
// service derives these keys from the Iroh root key; the PoC
// generates standalone Ed25519 keys.

const subtle = globalThis.crypto.subtle;
const encoder = new TextEncoder();

// Deterministic JSON: object keys sorted recursively, so a hash or
// signature over a record does not depend on construction order.
export function canonical(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) return "[" + value.map(canonical).join(",") + "]";
  const entries = Object.entries(value as Record<string, unknown>)
    .filter(([, v]) => v !== undefined)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return "{" + entries.map(([k, v]) => JSON.stringify(k) + ":" + canonical(v)).join(",") + "}";
}

export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

export function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  return out;
}

export async function sha256Hex(text: string): Promise<string> {
  const digest = await subtle.digest("SHA-256", encoder.encode(text));
  return bytesToHex(new Uint8Array(digest));
}

// Hash of a record's intent fields — the value the relay persists
// before any gateway fetch and the value the ledger compares on a
// retry to detect a mutated field under a reused request ID.
export async function intentHash(intent: unknown): Promise<string> {
  return sha256Hex(canonical(intent));
}

export class Ed25519Key {
  readonly #privateKey: CryptoKey;
  readonly #publicKey: CryptoKey;
  readonly publicKeyHex: string;

  private constructor(privateKey: CryptoKey, publicKey: CryptoKey, publicKeyHex: string) {
    this.#privateKey = privateKey;
    this.#publicKey = publicKey;
    this.publicKeyHex = publicKeyHex;
  }

  static async generate(): Promise<Ed25519Key> {
    const pair = (await subtle.generateKey("Ed25519", true, ["sign", "verify"])) as CryptoKeyPair;
    const raw = new Uint8Array(await subtle.exportKey("raw", pair.publicKey));
    return new Ed25519Key(pair.privateKey, pair.publicKey, bytesToHex(raw));
  }

  async signHex(message: string): Promise<string> {
    const sig = await subtle.sign("Ed25519", this.#privateKey, encoder.encode(message));
    return bytesToHex(new Uint8Array(sig));
  }

  static async verifyHex(publicKeyHex: string, message: string, signatureHex: string): Promise<boolean> {
    try {
      const raw = hexToBytes(publicKeyHex);
      const key = await subtle.importKey("raw", raw as BufferSource, "Ed25519", false, ["verify"]);
      return await subtle.verify(
        "Ed25519",
        key,
        hexToBytes(signatureHex) as BufferSource,
        encoder.encode(message),
      );
    } catch {
      return false;
    }
  }
}

// Domain-separated signing payload: a signature made for one record
// type can never be replayed as another's (same rule as
// hosted_export.rs's manifest domain separator).
export function signingPayload(domain: string, record: unknown): string {
  return domain + ":" + canonical(record);
}
